import re

from publicator.prompts import (
    Exact, ImageIdentity, Lineage, Nearest, PromptVersion, Unknown,
    parse_identity, rank_paths,
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
