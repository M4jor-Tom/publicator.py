import json
import os
import shutil
import subprocess
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

import pytest

import publish_next


# The scheduler runs in the browser; the user schedules "Tuesday 20:00" in their
# own timezone (Europe/Paris), so the stored timestamps are LOCAL 20:00 — never
# UTC 20:00. Pin a non-UTC zone so the test can tell local- from UTC-hour matching
# apart (in UTC they'd coincide and the bug would hide).
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
    tmpl = publish_next._PAGE_TMPL
    start = tmpl.index("// >>> scheduler core")
    end = tmpl.index("// <<< scheduler core")
    return tmpl[start:end]


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

    # Fake publications.json: each taken Tuesday holds per_slot=2 scheduled entries.
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

    existing_ts = publish_next._existing_ts(str(pub_json))
    # Anchor slot generation just before the first Tuesday so its slot list starts
    # exactly at weeks[0] — deterministic regardless of when the test runs.
    schedules = publish_next._schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2},
            {"name": "thursday", "day": "thursday", "hour": 20, "per_slot": 2}]},
        now=weeks[0] - 3600)

    for tz in ("Europe/Paris", "UTC", "America/New_York"):
        got = _next_slot_for(schedules, existing_ts, "tuesday", tz)
        assert got == expected, (
            f"[browser TZ={tz}] picker defaulted to "
            f"{datetime.fromtimestamp(got, _PARIS)}, expected "
            f"{datetime.fromtimestamp(expected, _PARIS)} (first open Tuesday)")


def test_schedule_slots_stay_2000_paris_across_dst():
    """Every generated slot is 20:00 Europe/Paris wall-clock, summer or winter —
    the invariant a fixed UTC offset would violate at the DST boundary."""
    data = publish_next._schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2}]})
    slots = data[0]["slots"]
    assert len(slots) >= 52
    hours = {datetime.fromtimestamp(s, _PARIS).hour for s in slots}
    assert hours == {20}, sorted(hours)
    # crosses at least one DST transition -> two distinct UTC offsets present
    offsets = {datetime.fromtimestamp(s, _PARIS).utcoffset() for s in slots}
    assert len(offsets) == 2, offsets


def test_page_offers_both_models():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = []; H.pending = []
    H.existing_ts = []
    H.ai_model = "claude-cli-opus"
    H.openrouter_model = "openrouter/google/gemini-2.0-flash-exp:free"
    # ponytail: _build_page reads only class attrs, so call it with the class as
    # `self` — avoids constructing a real BaseHTTPRequestHandler (needs a socket).
    page = publish_next.GalleryHandler._build_page(H)
    assert 'id="ai-model"' in page
    assert 'claude-cli-opus' in page
    assert 'openrouter/google/gemini-2.0-flash-exp:free' in page
    assert '__AI_MODEL__' not in page and '__OPENROUTER_MODEL__' not in page


def test_page_renders_config_tiers_galleries():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = ["/t/a.png"]; H.pending = []
    H.existing_ts = []; H.ai_model = "m"; H.openrouter_model = "o"
    H.config = {"tiers": ["gold"], "galleries": ["Art"]}
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-tier"' in page and '>gold<' in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = ["/t/a.png"]; H.pending = []
    H.existing_ts = []; H.ai_model = "m"; H.openrouter_model = "o"
    H.config = {"schedule": {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}}
    H.schedules = publish_next._schedule_data(H.config["schedule"])
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page   # SCHEDULES injected
    assert '"slots"' in page                                       # canonical instants embedded
    assert '__SCHEDULES__' not in page and '__EXISTING_TS__' not in page
    assert '__LABELS__' not in page                               # label map substituted
