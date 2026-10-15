import json
from datetime import datetime, timezone

import pytest

from publicator import scheduling, store

_TZ = "Europe/Paris"
_CFG = {"schedule": {"timezone": _TZ}, "tiers": ["gold"], "galleries": ["Art"]}
_TUE_2000 = int(datetime(2026, 1, 6, 20, tzinfo=timezone.utc).timestamp())  # 2026-01-06 is a Tue


@pytest.fixture
def img(tmp_path):
    p = tmp_path / "pic.png"
    p.write_bytes(b"\x89PNG\r\n\x1a\n")  # bytes are enough for sha512
    return str(p)


def _rows(path):
    return json.loads(path.read_text())


def test_mutate_publications_returns_result_and_persists_valid_data(tmp_path):
    path = tmp_path / "publications.json"
    path.write_text("[]")
    pub = {"uuid": "00000000-0000-0000-0000-000000000001",
           "state": "PICKED", "submissionTimestamp": 1,
           "title": "", "description": "", "files": []}
    result = store.mutate_publications(
        str(path), lambda rows: rows.append(pub), config={})
    assert result is None
    assert json.loads(path.read_text()) == [pub]


def test_mutate_publications_does_not_write_invalid_schema(tmp_path):
    from jsonschema import ValidationError
    path = tmp_path / "publications.json"
    path.write_text("[]")
    before = path.read_bytes()
    with pytest.raises(ValidationError):
        store.mutate_publications(str(path), lambda rows: rows.append({"invalid": True}), config={})
    assert path.read_bytes() == before


def test_mutate_publications_does_not_write_after_callback_error(tmp_path):
    path = tmp_path / "publications.json"
    path.write_text("[]")
    before = path.read_bytes()
    def fail(rows):
        rows.append({"invalid": True})
        raise ValueError("rejected")
    with pytest.raises(ValueError, match="rejected"):
        store.mutate_publications(str(path), fail, config={})
    assert path.read_bytes() == before


def test_marking_one_platform_success_does_not_complete_other_targets(tmp_path):
    path = str(tmp_path / "publications.json")
    uuid_ = "00000000-0000-0000-0000-000000000001"
    config = {"schedule": {}, "tiers": [], "galleries": []}
    store.atomic_write_json(path, [{
        "uuid": uuid_, "files": [], "submissionTimestamp": 1,
        "apparitionTimestampIfDifferentThanSubmission": 1800000000,
        "title": "Title", "description": "Description", "state": "DESCRIBED",
        "apparitions": [{"platformName": name, "state": "unpublished"}
                        for name in ("deviantart", "example")],
    }])
    store.mark_state(path, uuid_, "published_or_scheduled", platform="deviantart", config=config)
    assert store.load_publications(path)[0]["state"] == "DESCRIBED"
    store.mark_state(path, uuid_, "published_or_scheduled", platform="example", config=config)
    assert store.load_publications(path)[0]["state"] == "APPEARED"


@pytest.mark.parametrize("price", [0, 1.0])
@pytest.mark.parametrize("named_profile", [False, True])
def test_toml_defaults_apply_only_on_first_platform_selection(tmp_path, img, price, named_profile):
    from publicator.config import load_config

    profile = '[[schedule.profiles]]\nname = "paid"\n' if named_profile else '[schedule]\n'
    (tmp_path / "publicator.toml").write_text(
        '[deviantart]\ntiers = ["gold"]\n' + profile +
        f'default_tiers = ["gold"]\ndefault_price = {price}\n')
    cfg = load_config(tmp_path)
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), cfg)
    assert _rows(path)[0]["apparitions"] == []
    store.repack_publications(str(path), cfg)
    store.approve_publication_date(str(path), uuid_, cfg)
    store.save_publication_metadata(str(path), uuid_, title="Saved", description="Saved", config=cfg)
    store.approve_publication_metadata(str(path), uuid_, cfg)
    assert _rows(path)[0]["apparitions"] == []

    store.select_publication_platforms(str(path), selected=True, registered_platforms=["deviantart"],
                                      uuid_=uuid_, platform="deviantart", config=cfg)
    pub = _rows(path)[0]
    assert "priceIfNotFree" not in pub
    assert pub["apparitions"] == [{"platformName": "deviantart", "state": "unpublished",
                                   "tiers": ["gold"], "priceIfNotFree": price}]

    store.save_platform_options(str(path), uuid_, "deviantart", {"price": 7.5}, cfg)
    store.select_publication_platforms(str(path), selected=True, registered_platforms=["deviantart"],
                                      config=cfg)
    assert store.load_publications(str(path))[0]["apparitions"][0]["priceIfNotFree"] == 7.5


def test_options_save_does_not_silently_convert_singular_tier(tmp_path):
    uuid_ = "00000000-0000-4000-8000-000000000001"
    path = tmp_path / "publications.json"
    store.atomic_write_json(str(path), [{
        "uuid": uuid_, "state": "DESCRIBED", "title": "T", "description": "D",
        "submissionTimestamp": 1, "files": [],
        "apparitions": [{"platformName": "deviantart", "state": "unpublished", "tier": "gold"}],
    }])
    before = path.read_bytes()
    with pytest.raises(ValueError, match="obsolete tier"):
        store.save_platform_options(str(path), uuid_, "deviantart", {"tiers": ["gold"]}, _CFG)
    assert path.read_bytes() == before


def test_save_platform_options_selects_deviantart_not_first_platform(tmp_path):
    uuid_ = "00000000-0000-4000-8000-000000000001"
    pixiv = {"platformName": "pixiv", "state": "unpublished", "priceIfNotFree": 2}
    path = str(tmp_path / "publications.json")
    store.atomic_write_json(path, [{"uuid": uuid_, "state": "DESCRIBED",
        "title": "T", "description": "D", "submissionTimestamp": 1700000000,
        "files": [], "apparitions": [pixiv.copy(),
            {"platformName": "deviantart", "state": "unpublished"}]}])

    store.save_platform_options(path, uuid_, "deviantart",
        {"price": 5, "tiers": ["gold"], "galleries": ["Art"]}, _CFG)

    pub = store.load_publications(path)[0]
    assert pub["apparitions"][0] == pixiv
    assert pub["apparitions"][1] == {
        "platformName": "deviantart", "state": "unpublished", "priceIfNotFree": 5,
        "tiers": ["gold"], "galleries": ["Art"]}
