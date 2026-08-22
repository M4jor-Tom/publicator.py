import hashlib
import re
import subprocess

from publicator.apps.prompt_audit import audit, format_report

ART = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                 r"[0-9a-f-]{36}\.[^.]+$")
UUID = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"


def setup_dir(tmp_path):
    repo = tmp_path / "hf"; repo.mkdir()

    def run(*a):
        subprocess.run(["git", "-C", str(repo), *a], check=True, capture_output=True)

    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("prompt text\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")
    digest = hashlib.sha1(b"prompt text\n").hexdigest()

    picked = tmp_path / "picked"; picked.mkdir()
    (picked / f"a_{digest}_{UUID}.webp").write_bytes(b"1")     # exact
    (picked / f"a_{'f' * 40}_{UUID}.webp").write_bytes(b"2")   # nearest
    (picked / f"zz_{'e' * 40}_{UUID}.webp").write_bytes(b"3")  # unknown lineage
    (picked / "holiday.jpg").write_bytes(b"4")                 # not prompt-bearing
    cfg = {"publicable": ["picked"],
           "prompts": {"repo": "hf", "pattern": ART, "version_hash": "sha1"}}
    return cfg


def test_audit_counts_each_resolution_state(tmp_path):
    cfg = setup_dir(tmp_path)
    got = audit(str(tmp_path), cfg)
    assert got["files"] == 4
    assert got["parsed"] == 3
    assert got["exact"] == 1 and got["exact_versions"] == 1
    assert got["nearest"] == 1 and got["nearest_versions"] == 1
    assert got["unknown"] == 1


def test_report_leads_with_the_parse_rate(tmp_path):
    """`parsed` dropping to 0 is how grammar drift between identify_image.sh
    and publicator.toml is caught (ADR 0003)."""
    lines = format_report(audit(str(tmp_path), setup_dir(tmp_path))).splitlines()
    assert lines[0].startswith("parsed")
    assert "3 of 4" in lines[0]


def test_audit_without_a_prompts_section_parses_nothing(tmp_path):
    (tmp_path / "picked").mkdir()
    got = audit(str(tmp_path), {"publicable": ["picked"], "prompts": None})
    assert got == {"files": 0, "parsed": 0, "exact": 0, "exact_versions": 0,
                   "nearest": 0, "nearest_versions": 0, "unknown": 0}
