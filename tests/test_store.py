import json
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

import pytest

from publicator import publish_next as store   # Task 4 -> from publicator import store
from publicator.entries import STATE_UNPUBLISHED

_TZ = "Europe/Paris"
_PARIS = ZoneInfo(_TZ)
_CFG = {"schedule": {"timezone": _TZ}, "tiers": ["gold"], "galleries": ["Art"]}
_TUE_2000 = int(datetime(2026, 1, 6, 20, tzinfo=timezone.utc).timestamp())  # 2026-01-06 is a Tue


@pytest.fixture
def img(tmp_path):
    p = tmp_path / "pic.png"
    p.write_bytes(b"\x89PNG\r\n\x1a\n")  # bytes are enough for sha512
    return str(p)


def test_write_publications_appends_an_unpublished_entry(tmp_path, img):
    jp = str(tmp_path / "publications.json")
    uuids = store.write_publications(
        [{"path": img, "title": "t", "description": "d", "scheduleTs": _TUE_2000}], jp, _CFG)
    assert len(uuids) == 1
    rows = json.loads(open(jp).read())
    assert len(rows) == 1 and rows[0]["uuid"] == uuids[0]
    app = rows[0]["apparitions"][0]
    assert app["state"] == STATE_UNPUBLISHED
    assert app["apparitionTimestampIfDifferentThanSubmission"] == _TUE_2000
    assert app["urlElsePublicationName"] == "t"


def test_write_publications_requires_a_schedule(tmp_path, img):
    jp = str(tmp_path / "publications.json")
    with pytest.raises(ValueError):
        store.write_publications([{"path": img, "title": "t", "description": "d"}], jp, _CFG)


def test_apply_update_sets_then_clears_optional_fields(tmp_path, img):
    jp = str(tmp_path / "publications.json")
    uuids = store.write_publications(
        [{"path": img, "title": "t", "description": "d", "scheduleTs": _TUE_2000}], jp, _CFG)
    rows = json.loads(open(jp).read())

    store.apply_update(rows, uuids[0],
                       {"title": "T2", "description": "D2", "scheduleTs": _TUE_2000 + 604800,
                        "price": 3, "tier": "gold", "galleries": ["Art"]}, _PARIS)
    app = rows[0]["apparitions"][0]
    assert app["urlElsePublicationName"] == "T2" and app["priceIfNotFree"] == 3.0
    assert app["tier"] == "gold" and app["galleries"] == ["Art"]

    store.apply_update(rows, uuids[0],
                       {"title": "T3", "description": "D3", "scheduleTs": _TUE_2000 + 604800},
                       _PARIS)
    app = rows[0]["apparitions"][0]
    assert "priceIfNotFree" not in app and "tier" not in app and "galleries" not in app


def test_apply_update_raises_on_unknown_uuid():
    with pytest.raises(KeyError):
        store.apply_update([], "nope", {"title": "t", "scheduleTs": _TUE_2000}, _PARIS)


def test_resolve_ts_reads_a_naive_string_in_the_schedule_timezone():
    assert store.resolve_ts({"schedule": "2026-10-15T20:00"}, _PARIS) == \
        int(datetime(2026, 10, 15, 20, tzinfo=_PARIS).timestamp())


def test_custom_schedule_resolves_in_config_timezone(tmp_path, img):
    """A custom-typed wall-clock string persists as the schedule-TZ instant, not
    whatever the browser's timezone makes of it. A UTC-spoofed private window sends
    a scheduleTs 2h early; the naive 'schedule' string must win on both write paths."""
    jp = str(tmp_path / "publications.json")
    intended = int(datetime(2026, 10, 15, 20, tzinfo=_PARIS).timestamp())        # 20:00 Paris
    wrong = int(datetime(2026, 10, 15, 20, tzinfo=ZoneInfo("UTC")).timestamp())  # 20:00 UTC

    uuids = store.write_publications(
        [{"path": img, "title": "t", "description": "d",
          "schedule": "2026-10-15T20:00", "scheduleTs": wrong}], jp, _CFG)
    rows = json.loads(open(jp).read())
    assert rows[0]["apparitions"][0][
        "apparitionTimestampIfDifferentThanSubmission"] == intended

    store.apply_update(rows, uuids[0],
                       {"title": "t2", "description": "d",
                        "schedule": "2026-10-20T20:00", "scheduleTs": wrong}, _PARIS)
    assert rows[0]["apparitions"][0]["apparitionTimestampIfDifferentThanSubmission"] == \
        int(datetime(2026, 10, 20, 20, tzinfo=_PARIS).timestamp())
