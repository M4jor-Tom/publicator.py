import json
import os
import shutil
import subprocess
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

import pytest

from publicator import scheduling
from publicator.publish_next import PAGE_TEMPLATE

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


def _schedule_core_js():
    """The live scheduling functions, sliced verbatim from the served page
    between its marker comments."""
    start = PAGE_TEMPLATE.index("// >>> scheduler core")
    end = PAGE_TEMPLATE.index("// <<< scheduler core")
    return PAGE_TEMPLATE[start:end]


def _next_slot_for(schedules, existing_ts, profile_name, tz):
    """Run the real client scheduler in Node under browser timezone `tz` and
    return the epoch it would pre-fill the date picker with for `profile_name`.
    LABELS is irrelevant to slot choice; an empty map keeps the harness honest."""
    harness = (
        f"const SCHEDULES = {json.dumps(schedules)};\n"
        f"const EXISTING_TS = {json.dumps(existing_ts)};\n"
        "const LABELS = {};\n"
        "const queue = [];\n"
        f"{_schedule_core_js()}\n"
        f"console.log(nextSlotForProfile(profileByName({json.dumps(profile_name)})));\n"
    )
    out = subprocess.run(
        ["node", "-e", harness], check=True, capture_output=True, text=True,
        env={"TZ": tz, "PATH": os.environ["PATH"]},
    )
    return int(out.stdout.strip())


def test_profiles_normalize_day_names_to_python_weekdays():
    assert scheduling.schedule_profiles({"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2}]}) == \
        [{"name": "free", "day": 1, "hour": 20, "per_slot": 2}]


def test_flat_config_becomes_one_default_profile():
    assert scheduling.schedule_profiles({"day": "friday", "hour": 18, "per_slot": 1}) == \
        [{"name": "default", "day": 4, "hour": 18, "per_slot": 1}]


def test_empty_config_falls_back_to_tuesday_2000():
    assert scheduling.schedule_profiles({}) == \
        [{"name": "default", "day": 1, "hour": 20, "per_slot": 2}]


@pytest.mark.parametrize("bad,exc", [
    ({"profiles": [{"frequency": "monthly"}]}, NotImplementedError),
    ({"profiles": [{"day": "tuesday", "per_slot": 0}]}, ValueError),
    ({"profiles": [{"day": "someday"}]}, ValueError),
    ({"profiles": [{"name": "a", "day": "tuesday", "hour": 20},
                   {"name": "b", "day": "tuesday", "hour": 20}]}, ValueError),
    ({"profiles": []}, ValueError),
])
def test_invalid_profiles_are_rejected(bad, exc):
    with pytest.raises(exc):
        scheduling.schedule_profiles(bad)


def test_existing_ts_keeps_only_future_scheduled_entries(tmp_path):
    jp = tmp_path / "publications.json"
    future = int(datetime.now(_PARIS).timestamp()) + 86400
    past = int(datetime.now(_PARIS).timestamp()) - 86400
    jp.write_text(json.dumps([
        {"apparitions": [{"state": "published_or_scheduled",
                          "apparitionTimestampIfDifferentThanSubmission": future}]},
        {"apparitions": [{"state": "published_or_scheduled",
                          "apparitionTimestampIfDifferentThanSubmission": past}]},
        {"apparitions": [{"state": "unpublished",
                          "apparitionTimestampIfDifferentThanSubmission": future + 60}]},
    ]))
    assert scheduling.existing_ts(str(jp)) == [future]


def test_existing_ts_tolerates_a_missing_file(tmp_path):
    assert scheduling.existing_ts(str(tmp_path / "nope.json")) == []


def test_schedule_slots_stay_2000_paris_across_dst():
    """Every generated slot is 20:00 Europe/Paris wall-clock, summer or winter —
    the invariant a fixed UTC offset would violate at the DST boundary."""
    data = scheduling.schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2}]})
    slots = data[0]["slots"]
    assert len(slots) >= 52
    hours = {datetime.fromtimestamp(s, _PARIS).hour for s in slots}
    assert hours == {20}, sorted(hours)
    # crosses at least one DST transition -> two distinct UTC offsets present
    offsets = {datetime.fromtimestamp(s, _PARIS).utcoffset() for s in slots}
    assert len(offsets) == 2, offsets


def test_new_slot_packs_after_taken_slots_regardless_of_browser_tz(tmp_path):
    """Regression: with 6 Tuesdays fully booked (Europe/Paris 20:00), a fresh
    image must default to the 7th Tuesday — and it must do so no matter what
    timezone the *browser* reports. `firefox --private-window` with resist-
    fingerprinting spoofs Date to UTC, and the old wall-clock matching (getHours)
    then failed to recognize the 18:00-UTC slots as taken, re-suggesting the very
    next, already-full Tuesday. Slots are now server-generated absolute instants,
    so occupancy is TZ-independent."""
    if not shutil.which("node"):
        pytest.skip("node required to exercise the client scheduler")

    weeks = _paris_tuesday_2000_epochs(7)
    taken, expected = weeks[:6], weeks[6]

    pubs = []
    for i, ts in enumerate(taken):
        for slot in range(2):
            pubs.append({
                "uuid": f"u{i}-{slot}", "description": "d",
                "files": [{"basename": "img.png"}],
                "apparitions": [{
                    "platformName": "deviantart", "state": "published_or_scheduled",
                    "urlElsePublicationName": "t",
                    "apparitionTimestampIfDifferentThanSubmission": ts,
                }],
            })
    pub_json = tmp_path / "publications.json"
    pub_json.write_text(json.dumps(pubs))

    existing = scheduling.existing_ts(str(pub_json))
    # Anchor slot generation just before the first Tuesday so its slot list starts
    # exactly at weeks[0] — deterministic regardless of when the test runs.
    schedules = scheduling.schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2},
            {"name": "thursday", "day": "thursday", "hour": 20, "per_slot": 2}]},
        now=weeks[0] - 3600)

    for tz in ("Europe/Paris", "UTC", "America/New_York"):
        got = _next_slot_for(schedules, existing, "tuesday", tz)
        assert got == expected, (
            f"[browser TZ={tz}] picker defaulted to "
            f"{datetime.fromtimestamp(got, _PARIS)}, expected "
            f"{datetime.fromtimestamp(expected, _PARIS)} (first open Tuesday)")
