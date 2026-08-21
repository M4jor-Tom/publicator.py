import re

import pytest

from publicator.config import load_config, validate_publications


def test_load_config_defaults_when_file_missing(tmp_path):
    assert load_config(tmp_path) == {
        "tiers": [], "galleries": [], "publicable": [], "tags": None,
        "schedule": {}, "prompts": None}


def test_load_config_reads_flat_schedule(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        'publicable = ["picked"]\n'
        'tags = "sub/tags.txt"\n'
        '[deviantart]\ntiers = ["T"]\ngalleries = ["G"]\n'
        '[schedule]\nfrequency = "weekly"\nday = "tuesday"\nhour = 20\nper_slot = 2\n')
    c = load_config(tmp_path)
    assert c["tiers"] == ["T"] and c["galleries"] == ["G"], c
    assert c["publicable"] == ["picked"], c
    assert c["tags"] == "sub/tags.txt", c
    assert c["schedule"] == {
        "frequency": "weekly", "day": "tuesday", "hour": 20, "per_slot": 2}, c


def test_load_config_reads_schedule_profiles(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[[schedule.profiles]]\nname = "free"\nday = "tuesday"\nhour = 20\nper_slot = 2\n'
        '[[schedule.profiles]]\nname = "paid"\nday = "friday"\nhour = 20\nper_slot = 1\n')
    assert load_config(tmp_path)["schedule"] == {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}


PROMPTS_TOML = (
    '[prompts]\n'
    'repo = "huggingface_prompts"\n'
    'filename = "^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_[0-9a-f-]{36}\\\\.[^.]+$"\n'
    'version_hash = "sha1"\n')


def test_load_config_compiles_the_prompt_grammar(tmp_path):
    (tmp_path / "publicator.toml").write_text(PROMPTS_TOML)
    p = load_config(tmp_path)["prompts"]
    assert p["repo"] == "huggingface_prompts"
    assert p["version_hash"] == "sha1"
    m = p["pattern"].match(
        "hot_warrior.json_0e2d420dcabd86c88cacdf55224f6e3d0e52b615"
        "_bb6ba911-8d16-4d5a-82c4-a46b844863ed.webp")
    assert m and m.group("lineage") == "hot_warrior.json"
    assert m.group("version") == "0e2d420dcabd86c88cacdf55224f6e3d0e52b615"


def test_load_config_accepts_grammar_without_lineage_group(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        'filename = "^prompt-(?P<version>[0-9a-f]{64})\\\\.png$"\n'
        'version_hash = "sha256"\n')
    p = load_config(tmp_path)["prompts"]
    assert "lineage" not in p["pattern"].groupindex


def test_load_config_rejects_grammar_without_version_group(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        'filename = "^(?P<lineage>.+)\\\\.png$"\n'
        'version_hash = "sha1"\n')
    with pytest.raises(ValueError, match="version"):
        load_config(tmp_path)


def test_load_config_rejects_unknown_version_hash(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        'filename = "^(?P<version>.+)\\\\.png$"\n'
        'version_hash = "crc32-of-my-dreams"\n')
    with pytest.raises(ValueError, match="version_hash"):
        load_config(tmp_path)


def test_load_config_rejects_uncompilable_pattern(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        'filename = "^(?P<version>[unterminated"\n'
        'version_hash = "sha1"\n')
    with pytest.raises(ValueError, match="filename"):
        load_config(tmp_path)


def test_load_config_rejects_missing_repo(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\n'
        'filename = "^(?P<version>.+)\\\\.png$"\n'
        'version_hash = "sha1"\n')
    with pytest.raises(ValueError, match="repo"):
        load_config(tmp_path)


def _pubs(**apparition):
    """One schema-valid publication; uuid is exactly 36 chars as the schema demands."""
    return [{
        "uuid": "00000000-0000-4000-8000-000000000001",
        "submissionTimestamp": 1700000000,
        "description": "d",
        "files": [{"basename": "a.png", "sha512sum": "0" * 128}],
        "apparitions": [{
            "platformName": "deviantart",
            "state": "unpublished",
            "urlElsePublicationName": "t",
            "apparitionTimestampIfDifferentThanSubmission": 1800000000,
            **apparition,
        }],
    }]


def test_validate_accepts_configured_tier_and_gallery():
    validate_publications(_pubs(tier="gold", galleries=["Art"]),
                          {"tiers": ["gold"], "galleries": ["Art"]})


def test_validate_rejects_tier_absent_from_config():
    with pytest.raises(ValueError, match="tier"):
        validate_publications(_pubs(tier="gold"), {"tiers": [], "galleries": []})


def test_validate_rejects_gallery_absent_from_config():
    with pytest.raises(ValueError, match="gallery"):
        validate_publications(_pubs(galleries=["Art"]), {"tiers": [], "galleries": []})
