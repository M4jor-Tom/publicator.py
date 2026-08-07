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
    """The live scheduling functions, sliced verbatim from the served page."""
    tmpl = publish_next._PAGE_TMPL
    start = tmpl.index("function profileForTs")
    end = tmpl.index("function tsToLocalInput")
    return tmpl[start:end]


def _next_slot_for(schedules, existing_ts, profile_name):
    """Run the real client scheduler in Node under Europe/Paris and return the
    epoch it would pre-fill the date picker with for `profile_name`."""
    harness = (
        f"const SCHEDULES = {json.dumps(schedules)};\n"
        f"const EXISTING_TS = {json.dumps(existing_ts)};\n"
        "const queue = [];\n"
        f"{_schedule_core_js()}\n"
        f"console.log(nextSlotForProfile(profileByName({json.dumps(profile_name)})));\n"
    )
    out = subprocess.run(
        ["node", "-e", harness], check=True, capture_output=True, text=True,
        env={"TZ": _TZ, "PATH": os.environ["PATH"]},
    )
    return int(out.stdout.strip())


def test_new_slot_packs_after_already_taken_local_time_slots(tmp_path):
    """Regression: with 6 Tuesdays already fully booked (local 20:00), a fresh
    image must default to the 7th Tuesday, not the very next (already-full) one."""
    if not shutil.which("node"):
        pytest.skip("node required to exercise the client scheduler")

    weeks = _paris_tuesday_2000_epochs(7)
    taken, expected = weeks[:6], weeks[6]

    # Fake publications.json: each taken Tuesday holds per_slot=2 scheduled entries.
    (tmp_path / "img.png").write_bytes(b"\x89PNG\r\n\x1a\n fake")
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
    schedules = publish_next._schedules_js({"profiles": [
        {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "thursday", "day": "thursday", "hour": 20, "per_slot": 2}]})

    got = _next_slot_for(schedules, existing_ts, "tuesday")
    assert got == expected, (
        f"picker defaulted to {datetime.fromtimestamp(got, _PARIS)}, "
        f"expected {datetime.fromtimestamp(expected, _PARIS)} (first open Tuesday)")


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
    H.existing_ts = []; H.ai_model = "m"; H.openrouter_model = "o"; H.config = {}
    H.schedules = publish_next._schedules_js({"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]})
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page   # SCHEDULES injected
    assert '__SCHEDULES__' not in page and '__EXISTING_TS__' not in page
