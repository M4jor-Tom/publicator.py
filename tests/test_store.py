import json
from datetime import datetime, timezone

import pytest

from publicator import scheduling, store
from publicator.entries import (
    APPARITION_STATE_PUBLISHED,
    PUBLICATION_STATE_APPEARED,
    PUBLICATION_STATE_DESCRIBED,
    PUBLICATION_STATE_PICKED,
    PUBLICATION_STATE_SCHEDULED,
)

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


def test_metadata_approval_rejects_blank_and_marker_metadata(tmp_path, img):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)

    with pytest.raises(ValueError, match="title and description required"):
        store.approve_publication_metadata(str(path), uuid_, _CFG)

    store.save_publication_metadata(str(path), uuid_, title="T", description=store.TODO_GENERATE, config=_CFG)
    with pytest.raises(ValueError, match="title and description required"):
        store.approve_publication_metadata(str(path), uuid_, _CFG)


@pytest.mark.parametrize("field", ["title", "description"])
@pytest.mark.parametrize("marked", [
    f" {store.TODO_GENERATE} ",
    f"Before {store.TODO_GENERATE} after",
])
def test_metadata_approval_rejects_marker_occurrence_without_writing(tmp_path, img, field, marked):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)
    fields = {"title": "T", "description": "D", field: marked}
    store.save_publication_metadata(str(path), uuid_, **fields, config=_CFG)
    before = path.read_bytes()

    with pytest.raises(ValueError, match="title and description required"):
        store.approve_publication_metadata(str(path), uuid_, _CFG)

    assert path.read_bytes() == before
    assert _rows(path)[0]["state"] == PUBLICATION_STATE_SCHEDULED


def test_illegal_backward_edits_rejected_after_delivery_success(tmp_path, img):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)
    store.save_publication_metadata(str(path), uuid_, title="T", description="D", config=_CFG)
    store.approve_publication_metadata(str(path), uuid_, _CFG)
    rows = _rows(path)
    rows[0]["state"] = PUBLICATION_STATE_APPEARED
    rows[0]["apparitions"] = [{
        "platformName": "deviantart",
        "state": APPARITION_STATE_PUBLISHED,
    }]
    path.write_text(json.dumps(rows))

    with pytest.raises(ValueError, match="delivered"):
        store.return_to_scheduling(str(path), uuid_, _CFG)
    with pytest.raises(ValueError, match="delivered"):
        store.save_publication_metadata(str(path), uuid_, title="new", description="D", config=_CFG)


@pytest.mark.parametrize("fields", [{"title": ""}, {"description": store.TODO_GENERATE}])
def test_metadata_edits_after_approval_require_explicit_return(tmp_path, img, fields):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)
    store.save_publication_metadata(str(path), uuid_, title="T", description="D", config=_CFG)
    store.approve_publication_metadata(str(path), uuid_, _CFG)
    rows = _rows(path)
    rows[0]["apparitions"] = [{"platformName": "deviantart", "state": "unpublished"}]
    path.write_text(json.dumps(rows))
    before = path.read_bytes()

    with pytest.raises(ValueError, match="metadata edits require SCHEDULED"):
        store.save_publication_metadata(str(path), uuid_, **fields, config=_CFG)
    assert path.read_bytes() == before
    store.return_to_metadata(str(path), uuid_, _CFG)
    expected = rows[0] | {"state": PUBLICATION_STATE_SCHEDULED}
    assert _rows(path)[0] == expected
    store.save_publication_metadata(str(path), uuid_, **fields, config=_CFG)
    with pytest.raises(ValueError, match="title and description required"):
        store.approve_publication_metadata(str(path), uuid_, _CFG)


def test_return_to_metadata_rejects_delivery_success_without_writing(tmp_path):
    path = tmp_path / "publications.json"
    pub = {"uuid": "00000000-0000-4000-8000-000000000001", "state": "DESCRIBED",
           "title": "T", "description": "D", "submissionTimestamp": 1700000000,
           "files": [], "apparitions": [
               {"platformName": "deviantart", "state": APPARITION_STATE_PUBLISHED},
               {"platformName": "pixiv", "state": "unpublished"}]}
    path.write_text(json.dumps([pub]))
    before = path.read_bytes()
    with pytest.raises(ValueError, match="delivered"):
        store.return_to_metadata(str(path), pub["uuid"], _CFG)
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


@pytest.mark.parametrize('profile', [None, 'removed'])
def test_return_to_scheduling_without_override_does_not_invent_custom_date(tmp_path, profile):
    path = tmp_path / 'publications.json'
    pub = {'uuid': '00000000-0000-4000-8000-000000000001', 'state': 'DESCRIBED',
           'submissionTimestamp': 1700000000, 'title': 'Saved', 'description': 'Saved',
           'files': [], 'apparitions': []}
    if profile is not None:
        pub['scheduleProfile'] = profile
    store.atomic_write_json(str(path), [pub])
    store.return_to_scheduling(str(path), pub['uuid'], _CFG)
    assert _rows(path) == [pub | {'state': 'PICKED'}]
