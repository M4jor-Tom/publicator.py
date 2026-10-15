import json
from datetime import datetime, timezone

import pytest

from publicator import scheduling, store, workflow
from publicator.entries import (
    APPARITION_STATE_PUBLISHED,
    PUBLICATION_STATE_APPEARED,
    PUBLICATION_STATE_PICKED,
    PUBLICATION_STATE_SCHEDULED,
)

_CFG = {"schedule": {"timezone": "Europe/Paris"}, "tiers": ["gold"], "galleries": ["Art"]}
_TUE_2000 = int(datetime(2026, 1, 6, 20, tzinfo=timezone.utc).timestamp())

@pytest.fixture
def img(tmp_path):
    p = tmp_path / "pic.png"
    p.write_bytes(b"\x89PNG\r\n\x1a\n")
    return str(p)

def _rows(path):
    return json.loads(path.read_text())


def test_metadata_approval_rejects_blank_and_marker_metadata(tmp_path, img):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)

    with pytest.raises(ValueError, match="title and description required"):
        workflow.approve_publication_metadata(str(path), uuid_, _CFG)

    workflow.save_publication_metadata(str(path), uuid_, title="T", description=store.TODO_GENERATE, config=_CFG)
    with pytest.raises(ValueError, match="title and description required"):
        workflow.approve_publication_metadata(str(path), uuid_, _CFG)


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
    workflow.save_publication_metadata(str(path), uuid_, **fields, config=_CFG)
    before = path.read_bytes()

    with pytest.raises(ValueError, match="title and description required"):
        workflow.approve_publication_metadata(str(path), uuid_, _CFG)

    assert path.read_bytes() == before
    assert _rows(path)[0]["state"] == PUBLICATION_STATE_SCHEDULED


def test_illegal_backward_edits_rejected_after_delivery_success(tmp_path, img):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)
    workflow.save_publication_metadata(str(path), uuid_, title="T", description="D", config=_CFG)
    workflow.approve_publication_metadata(str(path), uuid_, _CFG)
    rows = _rows(path)
    rows[0]["state"] = PUBLICATION_STATE_APPEARED
    rows[0]["apparitions"] = [{
        "platformName": "deviantart",
        "state": APPARITION_STATE_PUBLISHED,
    }]
    path.write_text(json.dumps(rows))

    with pytest.raises(ValueError, match="delivered"):
        workflow.return_to_scheduling(str(path), uuid_, _CFG)
    with pytest.raises(ValueError, match="delivered"):
        workflow.save_publication_metadata(str(path), uuid_, title="new", description="D", config=_CFG)


@pytest.mark.parametrize("fields", [{"title": ""}, {"description": store.TODO_GENERATE}])
def test_metadata_edits_after_approval_require_explicit_return(tmp_path, img, fields):
    path = tmp_path / "publications.json"
    uuid_ = store.pick_publication(img, str(path), _CFG)
    store.edit_publication_date(str(path), uuid_, {"scheduleTs": _TUE_2000}, _CFG)
    store.approve_publication_date(str(path), uuid_, _CFG)
    workflow.save_publication_metadata(str(path), uuid_, title="T", description="D", config=_CFG)
    workflow.approve_publication_metadata(str(path), uuid_, _CFG)
    rows = _rows(path)
    rows[0]["apparitions"] = [{"platformName": "deviantart", "state": "unpublished"}]
    path.write_text(json.dumps(rows))
    before = path.read_bytes()

    with pytest.raises(ValueError, match="metadata edits require SCHEDULED"):
        workflow.save_publication_metadata(str(path), uuid_, **fields, config=_CFG)
    assert path.read_bytes() == before
    workflow.return_to_metadata(str(path), uuid_, _CFG)
    expected = rows[0] | {"state": PUBLICATION_STATE_SCHEDULED}
    assert _rows(path)[0] == expected
    workflow.save_publication_metadata(str(path), uuid_, **fields, config=_CFG)
    with pytest.raises(ValueError, match="title and description required"):
        workflow.approve_publication_metadata(str(path), uuid_, _CFG)


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
        workflow.return_to_metadata(str(path), pub["uuid"], _CFG)
    assert path.read_bytes() == before


@pytest.mark.parametrize('profile', [None, 'removed'])
def test_return_to_scheduling_without_override_does_not_invent_custom_date(tmp_path, profile):
    path = tmp_path / 'publications.json'
    pub = {'uuid': '00000000-0000-4000-8000-000000000001', 'state': 'DESCRIBED',
           'submissionTimestamp': 1700000000, 'title': 'Saved', 'description': 'Saved',
           'files': [], 'apparitions': []}
    if profile is not None:
        pub['scheduleProfile'] = profile
    store.atomic_write_json(str(path), [pub])
    workflow.return_to_scheduling(str(path), pub['uuid'], _CFG)
    assert _rows(path) == [pub | {'state': 'PICKED'}]


def test_pick_is_durable_and_creates_no_apparition(tmp_path):
    art = tmp_path / "art.png"
    art.write_bytes(b"image")
    path = tmp_path / "publications.json"

    uuid_ = workflow.pick_publication(str(art), str(path), {"schedule": {}, "tiers": [], "galleries": []})
    pub = _rows(path)[0]

    assert pub["uuid"] == uuid_
    assert pub["state"] == PUBLICATION_STATE_PICKED
    assert pub["title"] == pub["description"] == ""
    assert not pub.get("apparitions")


def test_repeated_pick_returns_same_uuid_by_content_hash(tmp_path, img):
    same = tmp_path / "same-bytes.png"
    same.write_bytes((tmp_path / "pic.png").read_bytes())
    path = tmp_path / "publications.json"

    first = workflow.pick_publication(img, str(path), _CFG)
    second = workflow.pick_publication(str(same), str(path), _CFG)

    assert second == first
    assert len(_rows(path)) == 1


def test_date_edit_pins_one_image_and_repacks_only_automatic_picked_items(tmp_path):
    path = tmp_path / "publications.json"
    cfg = {"schedule": {"timezone": "UTC", "profiles": [
        {"name": "default", "day": "tuesday", "hour": 20, "per_slot": 1},
    ]}, "tiers": [], "galleries": []}
    slots = scheduling.schedule_data(cfg["schedule"])[0]["slots"]
    first_slot = slots[0]
    second_slot = slots[1]
    third_slot = slots[2]
    a, b, c = tmp_path / "a.png", tmp_path / "b.png", tmp_path / "c.png"
    a.write_bytes(b"a")
    b.write_bytes(b"b")
    c.write_bytes(b"c")
    workflow.pick_publication(str(a), str(path), cfg)
    pinned = workflow.pick_publication(str(b), str(path), cfg)
    frozen = workflow.pick_publication(str(c), str(path), cfg)
    workflow.edit_publication_date(str(path), frozen, {"scheduleTs": first_slot}, cfg)
    workflow.approve_publication_date(str(path), frozen, cfg)

    workflow.edit_publication_date(str(path), pinned, {"scheduleTs": second_slot}, cfg)

    rows = {p["uuid"]: p for p in _rows(path)}
    assert rows[pinned]["scheduleIsManual"] is True
    assert rows[pinned]["apparitionTimestampIfDifferentThanSubmission"] == second_slot
    assert rows[frozen]["state"] == PUBLICATION_STATE_SCHEDULED
    assert rows[frozen]["apparitionTimestampIfDifferentThanSubmission"] == first_slot
    automatic = [p for p in rows.values() if p["state"] == PUBLICATION_STATE_PICKED and p["uuid"] != pinned][0]
    assert automatic["scheduleIsManual"] is False
    assert automatic["apparitionTimestampIfDifferentThanSubmission"] == third_slot


def test_schedule_approval_requires_a_visible_date(tmp_path, img):
    path = tmp_path / "publications.json"
    uuid_ = workflow.pick_publication(img, str(path), _CFG)

    with pytest.raises(ValueError, match="date required"):
        workflow.approve_publication_date(str(path), uuid_, _CFG)


def test_profile_only_edit_reallocates_and_unpin_rejoins_stored_order(tmp_path):
    path = tmp_path / "publications.json"
    cfg = {"schedule": {"timezone": "UTC", "profiles": [
        {"name": "default", "day": "tuesday", "hour": 20, "per_slot": 1},
        {"name": "other", "day": "friday", "hour": 20, "per_slot": 1},
    ]}, "tiers": [], "galleries": []}
    slots = {p["name"]: p["slots"] for p in scheduling.schedule_data(cfg["schedule"])}
    a, b = tmp_path / "a.png", tmp_path / "b.png"
    a.write_bytes(b"a")
    b.write_bytes(b"b")
    first = workflow.pick_publication(str(a), str(path), cfg)
    second = workflow.pick_publication(str(b), str(path), cfg)

    workflow.edit_publication_date(str(path), first, {"scheduleProfile": "other"}, cfg)
    rows = _rows(path)
    assert rows[0]["scheduleProfile"] == "other"
    assert rows[0]["scheduleIsManual"] is False
    assert rows[0]["apparitionTimestampIfDifferentThanSubmission"] == slots["other"][0]
    assert rows[1]["apparitionTimestampIfDifferentThanSubmission"] == slots["default"][0]

    workflow.edit_publication_date(str(path), first, {
        "scheduleProfile": "default", "scheduleTs": slots["default"][2]}, cfg)
    workflow.edit_publication_date(str(path), first, {"scheduleIsManual": False}, cfg)
    rows = _rows(path)
    assert rows[0]["scheduleIsManual"] is False
    assert rows[0]["apparitionTimestampIfDifferentThanSubmission"] == slots["default"][0]
    assert rows[1]["uuid"] == second
    assert rows[1]["apparitionTimestampIfDifferentThanSubmission"] == slots["default"][1]
    before = path.read_bytes()
    with pytest.raises(ValueError, match="profile"):
        workflow.edit_publication_date(str(path), first, {"scheduleProfile": "missing"}, cfg)
    assert path.read_bytes() == before


@pytest.mark.parametrize('profile', [None, 'removed', 'default'])
def test_migrated_custom_date_survives_return_and_unrelated_repack(tmp_path, img, profile):
    from publicator.migration import migrate_publications
    path = tmp_path / 'publications.json'
    legacy = {'uuid': '00000000-0000-4000-8000-000000000001',
              'submissionTimestamp': 1700000000, 'description': 'Saved description',
              'files': [], 'apparitions': [{'platformName': 'pixiv', 'state': 'unpublished',
                  'urlElsePublicationName': 'Saved title',
                  'apparitionTimestampIfDifferentThanSubmission': 1800000000}]}
    if profile is not None:
        legacy['scheduleProfile'] = profile
    migrated = migrate_publications([legacy], {}, _CFG)
    store.atomic_write_json(str(path), migrated)

    store.return_to_scheduling(str(path), legacy['uuid'], _CFG)
    workflow.pick_publication(img, str(path), _CFG)
    workflow.repack_publications(str(path), _CFG)

    pub = _rows(path)[0]
    assert pub['title'] == 'Saved title' and pub['description'] == 'Saved description'
    assert pub['apparitions'] == migrated[0]['apparitions']
    assert pub.get('scheduleProfile') == profile
    if profile == 'default':
        assert pub['scheduleIsManual'] is False
        assert pub['apparitionTimestampIfDifferentThanSubmission'] in scheduling.schedule_data(_CFG['schedule'])[0]['slots']
    else:
        assert pub['scheduleIsManual'] is True
        assert pub['apparitionTimestampIfDifferentThanSubmission'] == 1800000000


@pytest.mark.parametrize('profile', [None, 'removed'])
def test_manual_custom_date_edit_does_not_assign_profile_or_platform_defaults(tmp_path, img, profile):
    cfg = {'schedule': {'default_tiers': ['gold'], 'default_price': 1.0},
           'tiers': ['gold'], 'galleries': []}
    path = tmp_path / 'publications.json'
    uuid_ = workflow.pick_publication(img, str(path), cfg)
    rows = _rows(path)
    if profile is not None:
        rows[0]['scheduleProfile'] = profile
    path.write_text(json.dumps(rows))

    workflow.edit_publication_date(str(path), uuid_, {'scheduleTs': 1800000000}, cfg)
    pub = _rows(path)[0]
    assert pub.get('scheduleProfile') == profile
    assert pub['scheduleIsManual'] is True
    assert pub['apparitionTimestampIfDifferentThanSubmission'] == 1800000000
    workflow.approve_publication_date(str(path), uuid_, cfg)
    store.save_publication_metadata(str(path), uuid_, title='Saved', description='Saved', config=cfg)
    store.approve_publication_metadata(str(path), uuid_, cfg)
    store.select_publication_platforms(str(path), selected=True, registered_platforms=['deviantart'],
                                      uuid_=uuid_, platform='deviantart', config=cfg)
    assert _rows(path)[0]['apparitions'] == [{'platformName': 'deviantart', 'state': 'unpublished'}]

    store.select_publication_platforms(str(path), selected=False, registered_platforms=['deviantart'],
                                      uuid_=uuid_, platform='deviantart', config=cfg)
    store.return_to_scheduling(str(path), uuid_, cfg)
    workflow.edit_publication_date(str(path), uuid_, {'scheduleIsManual': False}, cfg)
    pub = _rows(path)[0]
    assert pub['scheduleProfile'] == 'default' and pub['scheduleIsManual'] is False
    assert pub['apparitionTimestampIfDifferentThanSubmission'] in scheduling.schedule_data(cfg['schedule'])[0]['slots']
    workflow.approve_publication_date(str(path), uuid_, cfg)
    store.approve_publication_metadata(str(path), uuid_, cfg)
    store.select_publication_platforms(str(path), selected=True, registered_platforms=['deviantart'],
                                      uuid_=uuid_, platform='deviantart', config=cfg)
    assert _rows(path)[0]['apparitions'][0]['tiers'] == ['gold']
    assert _rows(path)[0]['apparitions'][0]['priceIfNotFree'] == 1.0

