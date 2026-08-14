import pytest

from publicator.config import load_config, validate_publications


def test_load_config_defaults_when_file_missing(tmp_path):
    assert load_config(tmp_path) == {
        "tiers": [], "galleries": [], "publicable": [], "tags": None, "schedule": {}}


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
