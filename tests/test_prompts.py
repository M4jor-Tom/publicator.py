import hashlib
import re
import subprocess
import threading

from publicator.prompts import (
    Exact, ImageIdentity, Lineage, Nearest, PromptArchive, PromptVersion,
    Unknown, from_config, parse_identity, rank_paths,
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
    # ponytail: HEAD_PROBE_TTL throttles the `git rev-parse HEAD` probe; reset
    # it here to deliberately defeat the cache and prove the index still
    # refreshes once it does probe, rather than weakening the assertion below.
    a._probed = None
    assert second in a.versions()


def test_index_does_not_refresh_within_the_head_probe_ttl(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "one\n")
    a = archive(repo)
    assert len(a.versions()) == 1
    second = commit_file(repo, run, "p/a", "two\n")
    assert second not in a.versions(), "probed HEAD again before the TTL elapsed"
    a._probed = None
    assert second in a.versions(), "resetting the probe lets the next call see it"


def test_missing_repo_yields_an_empty_index(tmp_path):
    assert archive(tmp_path / "nope").versions() == {}


def test_resolve_returns_exact_for_an_archived_version(tmp_path):
    repo, run = make_repo(tmp_path)
    digest = commit_file(repo, run, "p/hot.json", "the real prompt\n")
    got = archive(repo).resolve(ImageIdentity(version=digest, lineage="hot.json"))
    assert isinstance(got, Exact)
    assert got.version.text == "the real prompt\n"


def test_resolve_returns_exact_for_a_superseded_version(tmp_path):
    """HEAD has drifted; the image's own version must still win. This is the
    404-of-426 silently-wrong case from the audit, frozen into a test."""
    repo, run = make_repo(tmp_path)
    old = commit_file(repo, run, "p/hot.json", "version one\n")
    commit_file(repo, run, "p/hot.json", "version two\n")
    got = archive(repo).resolve(ImageIdentity(version=old, lineage="hot.json"))
    assert isinstance(got, Exact)
    assert got.version.text == "version one\n"


def test_resolve_returns_nearest_when_the_version_was_never_archived(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/hot.json", "some version\n")
    got = archive(repo).resolve(ImageIdentity(version="f" * 40, lineage="hot.json"))
    assert isinstance(got, Nearest)
    assert [c.path for c in got.candidates] == ["p/hot.json"]
    assert got.candidates[0].versions[0].text == "some version\n"


def test_nearest_keeps_every_ambiguous_lineage_ranked_not_filtered(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "Heartsync__adult/mommy_tentacles", "adult\n")
    commit_file(repo, run, "Heartsync__NSFW_Uncensored_image/mommy_tentacles", "image\n")
    got = archive(repo).resolve(
        ImageIdentity(version="f" * 40, lineage="mommy_tentacles"),
        near="picked/huggingface/Heartsync__NSFW-Uncensored-image")
    assert isinstance(got, Nearest)
    assert len(got.candidates) == 2, "ranking must never discard a candidate"
    assert got.candidates[0].path == "Heartsync__NSFW_Uncensored_image/mommy_tentacles"


def test_resolve_returns_unknown_when_the_grammar_has_no_lineage(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "x\n")
    got = archive(repo, NOLIN, "sha256").resolve(
        ImageIdentity(version="f" * 64, lineage=None))
    assert isinstance(got, Unknown)


def test_resolve_returns_unknown_for_an_unrecognised_lineage(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "x\n")
    got = archive(repo).resolve(ImageIdentity(version="f" * 40, lineage="never_seen"))
    assert isinstance(got, Unknown)


def test_resolve_path_parses_then_resolves(tmp_path):
    repo, run = make_repo(tmp_path)
    digest = commit_file(repo, run, "p/hot.json", "text\n")
    got = archive(repo).resolve_path(f"picked/hug/hot.json_{digest}_{UUID}.webp")
    assert isinstance(got, Exact)


def test_resolve_path_returns_unknown_for_a_non_prompt_filename(tmp_path):
    repo, _ = make_repo(tmp_path)
    assert isinstance(archive(repo).resolve_path("picked/tpl/clip.mp4"), Unknown)


def test_resolve_path_ranks_nearest_by_the_images_own_directory(tmp_path):
    """Regression for the `near=os.path.dirname(path)` line inside resolve_path:
    same basename archived under two different directories. The correct
    directory (matching the image's own) has near-zero similarity to the
    filename itself, while the wrong one was picked to score *higher* against
    the raw filename (lots of shared hex-like characters with the version/uuid)
    than the right directory scores against that same filename — so this only
    passes if resolve_path compares against os.path.dirname(path), not path
    itself; passing the bare path would rank the wrong lineage first."""
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "abcdef_abcdef_abcdef/hot.json", "wrong\n")
    commit_file(repo, run, "output_gallery_mirror/hot.json", "right\n")
    image_path = (f"picked/hf/output_gallery_mirror/"
                  f"hot.json_{'f' * 40}_{UUID}.webp")
    got = archive(repo).resolve_path(image_path)
    assert isinstance(got, Nearest)
    assert len(got.candidates) == 2, "ranking must never discard a candidate"
    assert got.candidates[0].path == "output_gallery_mirror/hot.json"


def test_from_config_returns_none_without_a_prompts_section(tmp_path):
    assert from_config(str(tmp_path), {"prompts": None}) is None


def test_from_config_returns_none_when_the_repo_is_absent(tmp_path):
    cfg = {"prompts": {"repo": "nope", "pattern": ART, "version_hash": "sha1"}}
    assert from_config(str(tmp_path), cfg) is None


def test_from_config_builds_an_archive_for_a_present_repo(tmp_path):
    repo, run = make_repo(tmp_path, "hf")
    commit_file(repo, run, "p/a", "x\n")
    cfg = {"prompts": {"repo": "hf", "pattern": ART, "version_hash": "sha1"}}
    assert isinstance(from_config(str(tmp_path), cfg), PromptArchive)


def test_concurrent_queries_never_raise_or_tear(tmp_path):
    """Smoke/deadlock test, not a regression test: it cannot reliably force the
    real bug RED without a fixture large enough to span a GIL switch, which
    would be out of proportion for a unit test. What it does check: several
    threads hammering one PromptArchive (as GalleryHandler holds it, shared
    across ThreadingHTTPServer's one-thread-per-request) while a writer keeps
    moving HEAD never raises, and never observes the two known windows a
    missing lock opens -- both cheaper to hit than a KeyError:
    a half-built _by_path/_by_basename makes paths_for() answer () too early,
    so resolve() calls that Unknown and the card silently shows no prompt
    block where it should show a NOT ARCHIVED warning; and two concurrent
    refreshes can each wipe the other's half-built dicts while both still set
    _head last, corrupting the index for the life of the process (the
    head-unchanged early-return then always succeeds)."""
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/hot.json", "seed\n")
    a = archive(repo)
    unarchived_path = f"pre/hot.json_{'0' * 40}_{UUID}.webp"

    errors = []
    stop = threading.Event()

    def writer():
        try:
            for i in range(10):
                commit_file(repo, run, "p/hot.json", f"v{i}\n")
        finally:
            stop.set()   # readers must not spin forever if a commit fails

    def reader():
        while not stop.is_set():
            try:
                a.versions()
                assert a.paths_for("hot.json") == ("p/hot.json",)
                assert a.versions_at("p/hot.json") != ()
                a.resolve_path(unarchived_path)
            except Exception as e:
                errors.append(e)
                return

    threads = [threading.Thread(target=writer)]
    threads += [threading.Thread(target=reader) for _ in range(6)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert errors == []
