import hashlib
import re
import subprocess

from publicator.prompts import (
    Exact, ImageIdentity, Lineage, Nearest, PromptArchive, PromptVersion,
    Unknown, parse_identity, rank_paths,
)

ART = re.compile(
    r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})"
    r"_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[^.]+$")
NOLIN = re.compile(r"^prompt-(?P<version>[0-9a-f]{64})\.png$")

SHA = "0e2d420dcabd86c88cacdf55224f6e3d0e52b615"
UUID = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"


def test_parse_identity_splits_lineage_and_version():
    got = parse_identity(f"hot_warrior.json_{SHA}_{UUID}.webp", ART)
    assert got == ImageIdentity(version=SHA, lineage="hot_warrior.json")


def test_parse_identity_keeps_underscores_inside_the_lineage():
    got = parse_identity(f"topless_mommy_tentacles_{SHA}_{UUID}.webp", ART)
    assert got.lineage == "topless_mommy_tentacles"
    assert got.version == SHA


def test_parse_identity_returns_none_for_a_plain_filename():
    assert parse_identity("VirtualDesktop.Android-20260403-004503-0.mp4", ART) is None


def test_parse_identity_without_a_lineage_group_yields_none_lineage():
    got = parse_identity(f"prompt-{'a' * 64}.png", NOLIN)
    assert got == ImageIdentity(version="a" * 64, lineage=None)


def test_nearest_has_no_text_attribute():
    """The anti-conflation device of ADR 0002: a near-miss cannot be rendered
    as the real prompt because it has no field to render."""
    near = Nearest(candidates=(Lineage(path="a/b", versions=()),))
    assert not hasattr(near, "text")
    assert not hasattr(near, "version")


def test_exact_exposes_the_prompt_text():
    v = PromptVersion(version=SHA, text="a prompt", paths=("a/b",), committed=7)
    assert Exact(version=v).version.text == "a prompt"


def test_unknown_carries_a_reason():
    assert Unknown(reason="nope").reason == "nope"


def test_rank_paths_prefers_the_directory_matching_the_image():
    paths = ["Heartsync__adult/mommy_tentacles",
             "Heartsync__NSFW_Uncensored_image/mommy_tentacles"]
    got = rank_paths(paths, "picked/huggingface/Heartsync__NSFW-Uncensored-image")
    assert got[0] == "Heartsync__NSFW_Uncensored_image/mommy_tentacles"


def test_rank_paths_never_discards():
    paths = ["a/x", "b/x", "c/x"]
    assert sorted(rank_paths(paths, "somewhere/else")) == sorted(paths)


def make_repo(tmp_path, name="prompts"):
    repo = tmp_path / name
    repo.mkdir()

    def run(*a):
        subprocess.run(["git", "-C", str(repo), *a], check=True, capture_output=True)

    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@example.com")
    run("config", "user.name", "t")
    return repo, run


def commit_file(repo, run, relpath, text):
    """Write, commit, and return the CONTENT digest — deliberately not the git
    blob OID, which hashes 'blob <len>\\0' + content and would never match."""
    p = repo / relpath
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)
    run("add", "--", relpath)
    run("commit", "-q", "-m", f"snapshot: {relpath}")
    return hashlib.sha1(text.encode()).hexdigest()


def archive(repo, pattern=ART, algo="sha1"):
    return PromptArchive(str(repo), pattern, algo)


def test_index_holds_a_committed_prompt_by_content_digest(tmp_path):
    repo, run = make_repo(tmp_path)
    digest = commit_file(repo, run, "anima_v1/hot.json", "a prompt\n")
    v = archive(repo).versions()[digest]
    assert v.text == "a prompt\n"
    assert v.paths == ("anima_v1/hot.json",)
    assert v.committed > 0


def test_index_keeps_superseded_versions(tmp_path):
    """The regression test for the whole feature: HEAD has moved on, and the
    old digest must still resolve to the OLD text."""
    repo, run = make_repo(tmp_path)
    old = commit_file(repo, run, "p/a", "version one\n")
    new = commit_file(repo, run, "p/a", "version two\n")
    versions = archive(repo).versions()
    assert versions[old].text == "version one\n"
    assert versions[new].text == "version two\n"


def test_index_uses_the_configured_digest_algorithm(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "hello\n")
    digest = hashlib.sha256(b"hello\n").hexdigest()
    assert archive(repo, NOLIN, "sha256").versions()[digest].text == "hello\n"


def test_paths_for_finds_a_basename_that_no_longer_exists_at_head(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/hot_mommy.json", "old\n")
    run("mv", "p/hot_mommy.json", "p/lab_hot_mommy.json")
    run("commit", "-q", "-m", "rename")
    a = archive(repo)
    assert a.paths_for("hot_mommy.json") == ("p/hot_mommy.json",)
    assert a.paths_for("lab_hot_mommy.json") == ("p/lab_hot_mommy.json",)


def test_versions_at_returns_newest_first(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "one\n")
    commit_file(repo, run, "p/a", "two\n")
    texts = [v.text for v in archive(repo).versions_at("p/a")]
    assert texts == ["two\n", "one\n"]


def test_index_refreshes_when_head_moves(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "one\n")
    a = archive(repo)
    assert len(a.versions()) == 1
    second = commit_file(repo, run, "p/a", "two\n")
    assert second in a.versions()


def test_missing_repo_yields_an_empty_index(tmp_path):
    assert archive(tmp_path / "nope").versions() == {}
