import json
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

import pytest

from publicator import scheduling

_TZ = "Europe/Paris"
_PARIS = ZoneInfo(_TZ)


def _paris_tuesday_2000_epochs(count):
    """`count` consecutive Tuesday-20:00 Europe/Paris timestamps, first one the
    next Tuesday strictly after now — the real cadence the schedule produces.
    Rebuilt at local 20:00 each week so it stays correct across DST."""
    now = datetime.now(_PARIS)
    ahead = (1 - now.weekday()) % 7  # Python weekday: Tuesday == 1
    first = datetime(now.year, now.month, now.day, 20, tzinfo=_PARIS) + timedelta(days=ahead)
    if first <= now:
        first += timedelta(days=7)
    out = []
    for i in range(count):
        d = (first + timedelta(days=7 * i)).date()
        out.append(int(datetime(d.year, d.month, d.day, 20, tzinfo=_PARIS).timestamp()))
    return out


def test_profiles_normalize_day_names_to_python_weekdays_and_default_tiers():
    assert scheduling.schedule_profiles({"profiles": [
        {"name": "paid", "day": "wednesday", "hour": 20, "per_slot": 2,
         "default_tiers": ["gold", "silver"]}]}) == \
        [{"name": "paid", "day": 2, "hour": 20, "per_slot": 2,
          "default_tiers": ["gold", "silver"], "default_price": None}]


def test_schedule_data_carries_profile_tier_defaults():
    profile = scheduling.schedule_data({"timezone": "UTC", "profiles": [
        {"name": "paid", "day": "wednesday", "default_tiers": ["gold", "silver"]}]},
        now=0)[0]
    assert profile["default_tiers"] == ["gold", "silver"]


def test_schedule_data_carries_profile_price_default():
    profile = scheduling.schedule_data({"timezone": "UTC", "profiles": [
        {"name": "paid", "default_price": 1.0}]}, now=0)[0]
    assert profile["default_price"] == 1.0


@pytest.mark.parametrize("price", [-1, "1", True, False, float("inf"), float("-inf"), float("nan")])
def test_schedule_profiles_reject_invalid_default_prices(price):
    with pytest.raises(ValueError, match="default_price"):
        scheduling.schedule_profiles({"default_price": price})


def test_schedule_data_rejects_tier_defaults_missing_from_allow_list():
    with pytest.raises(ValueError, match="silver"):
        scheduling.schedule_data({"profiles": [
            {"name": "paid", "default_tiers": ["gold", "silver"]}]},
            allowed_tiers=["gold"])


def test_flat_config_becomes_one_default_profile():
    assert scheduling.schedule_profiles({"day": "friday", "hour": 18, "per_slot": 1}) == \
        [{"name": "default", "day": 4, "hour": 18, "per_slot": 1,
          "default_tiers": [], "default_price": None}]


def test_empty_config_falls_back_to_tuesday_2000():
    assert scheduling.schedule_profiles({}) == \
        [{"name": "default", "day": 1, "hour": 20, "per_slot": 2,
          "default_tiers": [], "default_price": None}]


@pytest.mark.parametrize("bad,exc", [
    ({"profiles": [{"frequency": "monthly"}]}, NotImplementedError),
    ({"profiles": [{"day": "tuesday", "per_slot": 0}]}, ValueError),
    ({"profiles": [{"day": "someday"}]}, ValueError),
    ({"profiles": [{"default_tiers": "gold"}]}, ValueError),
    ({"profiles": [{"name": "a", "day": "tuesday", "hour": 20},
                   {"name": "b", "day": "tuesday", "hour": 20}]}, ValueError),
    ({"profiles": []}, ValueError),
])
def test_invalid_profiles_are_rejected(bad, exc):
    with pytest.raises(exc):
        scheduling.schedule_profiles(bad)


def test_resolve_ts_reads_a_naive_string_in_the_schedule_timezone():
    assert scheduling.resolve_ts({"schedule": "2026-10-15T20:00"}, _PARIS) == \
        int(datetime(2026, 10, 15, 20, tzinfo=_PARIS).timestamp())


def test_repack_leaves_missing_suggestion_when_slots_are_exhausted(monkeypatch):
    monkeypatch.setattr(scheduling, "SLOT_HORIZON_WEEKS", 1)
    slot = int(datetime(2026, 1, 6, 20, tzinfo=ZoneInfo("UTC")).timestamp())
    pubs = [
        {"uuid": "frozen", "state": "SCHEDULED", "apparitionTimestampIfDifferentThanSubmission": slot},
        {"uuid": "picked", "state": "PICKED", "scheduleProfile": "default", "scheduleIsManual": False},
    ]

    scheduling.repack_publication_dates(pubs, {"timezone": "UTC", "profiles": [
        {"name": "default", "day": "tuesday", "hour": 20, "per_slot": 1},
    ]}, now=slot - 3600)

    assert "apparitionTimestampIfDifferentThanSubmission" not in pubs[1]


def test_schedule_slots_stay_2000_paris_across_dst():
    """Every generated slot is 20:00 Europe/Paris wall-clock, summer or winter —
    the invariant a fixed UTC offset would violate at the DST boundary."""
    data = scheduling.schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2}]})
    slots = data[0]["slots"]
    assert len(slots) == scheduling.SLOT_HORIZON_WEEKS
    hours = {datetime.fromtimestamp(s, _PARIS).hour for s in slots}
    assert hours == {20}, sorted(hours)
    # crosses at least one DST transition -> two distinct UTC offsets present
    offsets = {datetime.fromtimestamp(s, _PARIS).utcoffset() for s in slots}
    assert len(offsets) == 2, offsets


def test_repack_packs_after_taken_slots_without_browser_clock(tmp_path, monkeypatch):
    """Six full Tuesdays must reserve six slots; browser/host TZ cannot affect allocation."""
    weeks = _paris_tuesday_2000_epochs(7)
    taken, expected = weeks[:6], weeks[6]

    pubs = []
    for i, ts in enumerate(taken):
        for slot in range(2):
            pubs.append({
                "uuid": f"u{i}-{slot}", "state": "SCHEDULED", "description": "d",
                "files": [{"basename": "img.png"}],
                "apparitionTimestampIfDifferentThanSubmission": ts,
            })
    pub_json = tmp_path / "publications.json"
    pub_json.write_text(json.dumps(pubs))

    for tz in ("Europe/Paris", "UTC", "America/New_York"):
        monkeypatch.setenv("TZ", tz)
        rows = json.loads(pub_json.read_text()) + [{"state": "PICKED", "scheduleProfile": "tuesday"}]
        scheduling.repack_publication_dates(rows,
            {"timezone": _TZ, "profiles": [
                {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2},
                {"name": "thursday", "day": "thursday", "hour": 20, "per_slot": 2}]},
            now=weeks[0] - 3600)
        assert rows[-1]["apparitionTimestampIfDifferentThanSubmission"] == expected


def test_repack_reserves_historical_submission_timestamp_without_override():
    slot = int(datetime(2026, 1, 6, 20, tzinfo=ZoneInfo("UTC")).timestamp())
    pubs = [
        {"state": "APPEARED", "submissionTimestamp": slot, "apparitions": [
            {"platformName": "deviantart", "state": "published_or_scheduled"},
            {"platformName": "pixiv", "state": "published_or_scheduled"}]},
        {"state": "PICKED", "submissionTimestamp": slot},
    ]
    scheduling.repack_publication_dates(pubs, {"timezone": "UTC", "profiles": [
        {"name": "default", "day": "tuesday", "hour": 20, "per_slot": 1},
    ]}, now=slot - 3600)
    assert "apparitionTimestampIfDifferentThanSubmission" not in pubs[0]
    assert pubs[1]["apparitionTimestampIfDifferentThanSubmission"] == slot + 7 * 86400
