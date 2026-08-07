#!/usr/bin/env python3
"""Unified publish workflow: gallery + AI metadata + Playwright Firefox batch.

Single entrypoint. Scans picked/, opens a dark gallery on 127.0.0.1:PORT, lets
you delete images or add them to a queue (with AI-generated title/description
and per-entry schedule override), then writes to publications.json and drives a
Firefox persistent context through the DeviantArt submission flow.

DeviantArt is behind PerimeterX bot detection, which blocks EVERY Playwright
browser at the login page (Chromium and Firefox alike, since Playwright forces
navigator.webdriver). So login is NOT automated: `nix run <publicator>#login`
opens a real, flake-managed Firefox on the repo-local `.deviantart-login/`
profile for a one-time human sign-in. publish_batch copies that logged-in +
PerimeterX-cleared profile into the Playwright Firefox session and only VERIFIES
it — the submission pages themselves are not bot-walled.
"""

import argparse
import hashlib
import html
import json
import logging
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
import uuid
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from zoneinfo import ZoneInfo

from da_publish import (
    STATE_UNPUBLISHED,
    _atomic_write_json,
    configure,
    load_pending_entries,
    publish_batch,
    setup_logging,
)
from echo_first_unpublished_publication_data import deviantart_apparition
from llm_meta import DEFAULT_MODEL, generate_metadata
from validate import load_config, validate_publications

log = logging.getLogger("publicator.gallery")

# Runtime state (publications.json, images, browser session) resolves against the
# publication database dir (--data-dir, default CWD). da_publish.configure() owns
# the DATA_DIR/session paths; main() calls it once args are parsed.
IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tiff", ".mp4"}
MIME = {
    ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png",
    ".webp": "image/webp", ".gif": "image/gif", ".mp4": "video/mp4",
}


# ---------------------------------------------------------------------------
# Candidate discovery + thumbnails
# ---------------------------------------------------------------------------

def compute_sha512(filepath: str, chunk_size: int = 1024 * 1024) -> str:
    h = hashlib.sha512()
    with open(filepath, "rb") as f:
        while chunk := f.read(chunk_size):
            h.update(chunk)
    return h.hexdigest()


def load_publicated_hashes(json_path: str) -> set[str]:
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        return set()
    publicated = set()
    for entry in data:
        for file_obj in entry.get("files", []) + entry.get("previewFilesIfNotFree", []):
            sha = file_obj.get("sha512sum") or file_obj.get("fileSha512sum")
            if sha:
                publicated.add(sha.lower())
    return publicated


def collect_images(directory: str) -> list[str]:
    images = []
    for root, _, files in os.walk(directory):
        for filename in files:
            if os.path.splitext(filename)[1].lower() in IMAGE_EXTENSIONS:
                images.append(os.path.join(root, filename))
    return images


def find_candidates(directories: list[str], json_path: str, limit: int) -> list[str]:
    publicated = load_publicated_hashes(json_path)
    images = []
    for d in directories:
        images.extend(collect_images(d))   # os.walk on a missing dir yields nothing
    random.shuffle(images)
    candidates = []
    for path in images:
        if len(candidates) >= limit:
            break
        try:
            if compute_sha512(path) not in publicated:
                candidates.append(path)
        except OSError as e:
            print(f"Error hashing {path}: {e}", file=sys.stderr)
    return candidates


def generate_thumbnails(image_paths: list[str], thumb_dir: str) -> dict[str, str]:
    os.makedirs(thumb_dir, exist_ok=True)
    path_to_thumb = {}
    for i, path in enumerate(image_paths):
        ext = os.path.splitext(path)[1].lower()
        thumb_path = os.path.join(thumb_dir, f"thumb_{i:04d}{ext}")
        try:
            subprocess.run(
                ["convert", path, "-resize", "300x300>", thumb_path],
                capture_output=True, check=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            shutil.copy2(path, thumb_path)
        path_to_thumb[path] = thumb_path
    return path_to_thumb


# ---------------------------------------------------------------------------
# Schedule cadence — all weekday/hour/timezone math is done here server-side and
# shipped to the browser as absolute slot instants (see _build_page).
# ---------------------------------------------------------------------------

_DEFAULT_TZ = "Europe/Paris"       # the wall clock "20:00" is anchored to; overridable via schedule.timezone
_SLOT_HORIZON_WEEKS = 52           # ponytail: 1y of weekly slots embedded; bump if you ever queue further out
_WEEKDAY = {"monday": 0, "tuesday": 1, "wednesday": 2, "thursday": 3,
            "friday": 4, "saturday": 5, "sunday": 6}   # Python date.weekday()


def _zone(schedule: dict) -> ZoneInfo:
    return ZoneInfo(schedule.get("timezone", _DEFAULT_TZ))


def _schedule_profiles(schedule: dict) -> list[dict]:
    """Validated cadence: [{name, day: <Python weekday, 0=Mon>, hour, per_slot}, ...].
    Reads schedule['profiles']; a flat day/hour/per_slot config becomes one 'default'
    profile, an empty dict the built-in default. Only frequency='weekly' is
    implemented; profiles need distinct (day, hour). Internal to _schedule_data —
    the browser never sees day/hour, only the generated slot instants."""
    raw = schedule.get("profiles")
    if raw is None:               # flat day/hour/per_slot config, or {} -> one 'default' profile
        raw = [schedule]
    out, seen = [], set()
    for p in raw:
        freq = p.get("frequency", "weekly")
        if freq != "weekly":
            raise NotImplementedError(f"schedule.frequency {freq!r} not implemented (only 'weekly')")
        day = str(p.get("day", "tuesday")).lower()
        if day not in _WEEKDAY:
            raise ValueError(f"schedule.day {day!r} invalid")
        hour = int(p.get("hour", 20))
        per_slot = int(p.get("per_slot", 2))
        if per_slot < 1:
            raise ValueError(f"schedule.per_slot must be >= 1, got {per_slot}")
        key = (_WEEKDAY[day], hour)
        if key in seen:
            raise ValueError(f"schedule profiles collide on (day={day}, hour={hour})")
        seen.add(key)
        out.append({"name": str(p.get("name", "default")),
                    "day": _WEEKDAY[day], "hour": hour, "per_slot": per_slot})
    if not out:
        raise ValueError("schedule.profiles is empty")
    return out


def _profile_slots(profile: dict, zone: ZoneInfo, start: int) -> list[int]:
    """Ascending epochs of the next _SLOT_HORIZON_WEEKS weekly slots for `profile`,
    each at its (weekday, hour) wall-clock in `zone`. Built date-by-date in the zone
    so a slot stays 20:00 local across DST (never a fixed UTC offset). First slot is
    the earliest matching instant strictly after `start`."""
    hour = profile["hour"]
    day = datetime.fromtimestamp(start, zone).date()
    day += timedelta(days=(profile["day"] - day.weekday()) % 7)   # this week's (or today's) weekday
    if datetime(day.year, day.month, day.day, hour, tzinfo=zone).timestamp() <= start:
        day += timedelta(days=7)                                  # today's slot already passed
    slots = []
    for _ in range(_SLOT_HORIZON_WEEKS):
        slots.append(int(datetime(day.year, day.month, day.day, hour, tzinfo=zone).timestamp()))
        day += timedelta(days=7)
    return slots


def _schedule_data(schedule: dict, now: int | None = None) -> list[dict]:
    """Client scheduling payload: per profile, its upcoming canonical slot
    instants. ALL weekday/hour/timezone math lives here (Python zoneinfo) so the
    browser never reads a clock — it only counts occupancy by exact-timestamp
    equality, immune to the private-window UTC spoof."""
    zone = _zone(schedule)
    start = int(now if now is not None else time.time())
    return [{"name": p["name"], "per_slot": p["per_slot"],
             "slots": _profile_slots(p, zone, start)}
            for p in _schedule_profiles(schedule)]


def _ts_labels(zone: ZoneInfo, tss) -> dict[int, str]:
    """{ts: 'YYYY-MM-DDTHH:MM'} in the schedule timezone, so datetime-local inputs
    show the intended wall clock regardless of the browser's timezone."""
    return {ts: datetime.fromtimestamp(ts, zone).strftime("%Y-%m-%dT%H:%M")
            for ts in set(tss)}


def _existing_ts(json_path: str) -> list[int]:
    """Future timestamps of already-scheduled apparitions (state != unpublished) —
    the occupancy background the client packs new slots around. Unpublished
    (pending) entries are excluded; they ride in the client queue instead.
    Past timestamps are dropped: nextSlotForProfile only compares against slots
    >= now, so they can never occupy a candidate slot, and this keeps the
    embedded __EXISTING_TS__ payload from growing without bound over the years."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return []
    out, now = [], time.time()
    for entry in data:
        for app in entry.get("apparitions", []):
            ts = app.get("apparitionTimestampIfDifferentThanSubmission")
            if ts and ts >= now and app.get("state") != STATE_UNPUBLISHED:
                out.append(int(ts))
    return out


def _selfcheck() -> None:
    # schedule cadence: profiles -> validated list; weekly day-name -> Python weekday (0=Mon)
    assert _schedule_profiles({"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2}]}) == \
        [{"name": "free", "day": 1, "hour": 20, "per_slot": 2}]
    # flat keys -> single "default" profile
    assert _schedule_profiles({"day": "friday", "hour": 18, "per_slot": 1}) == \
        [{"name": "default", "day": 4, "hour": 18, "per_slot": 1}]
    # empty -> built-in default (Tuesday 20:00, per_slot 2)
    assert _schedule_profiles({}) == [{"name": "default", "day": 1, "hour": 20, "per_slot": 2}]
    for bad, exc in [
        ({"profiles": [{"frequency": "monthly"}]}, NotImplementedError),
        ({"profiles": [{"day": "tuesday", "per_slot": 0}]}, ValueError),
        ({"profiles": [{"name": "a", "day": "tuesday", "hour": 20},
                       {"name": "b", "day": "tuesday", "hour": 20}]}, ValueError),
        ({"profiles": []}, ValueError),
    ]:
        try:
            _schedule_profiles(bad); assert False, f"{bad} not rejected"
        except exc:
            pass
    # slots stay on the profile's wall clock: 52 future Tuesday-20:00 Europe/Paris
    _slots = _schedule_data({"timezone": "Europe/Paris", "profiles": [
        {"name": "t", "day": "tuesday", "hour": 20, "per_slot": 2}]})[0]["slots"]
    assert len(_slots) == 52 and all(datetime.fromtimestamp(s, ZoneInfo("Europe/Paris")).hour == 20
                                     for s in _slots), _slots
    # literal Tuesday 20:00 UTC anchors for the write_publications round-trip
    s0 = int(datetime(2026, 1, 6, 20, tzinfo=timezone.utc).timestamp())  # 2026-01-06 is a Tue
    s1 = s0 + 7 * 24 * 3600
    # /original decodes the path arg verbatim, so the allow-list (path in
    # thumb_map) sees the real path — a non-listed path can't sneak through.
    q = urllib.parse.parse_qs(urllib.parse.urlparse("/original?path=%2Fetc%2Fpasswd").query)
    assert q.get("path", [""])[0] == "/etc/passwd", q
    with tempfile.TemporaryDirectory() as _d:
        _jp = os.path.join(_d, "publications.json")
        _img = os.path.join(_d, "pic.png")
        with open(_img, "wb") as _f:
            _f.write(b"\x89PNG\r\n\x1a\n")  # bytes are enough for sha512
        _uuids = write_publications(
            [{"path": _img, "title": "t", "description": "d", "scheduleTs": s0}],
            _jp,
        )
        assert len(_uuids) == 1, _uuids
        # scheduleTs is now required — a missing one must raise, no write
        try:
            write_publications([{"path": _img, "title": "t", "description": "d"}], _jp)
            assert False, "missing scheduleTs not rejected"
        except ValueError:
            pass
        with open(_jp) as _f:
            _rows = json.load(_f)
        assert len(_rows) == 1 and _rows[0]["uuid"] == _uuids[0], _rows
        assert _rows[0]["apparitions"][0]["state"] == STATE_UNPUBLISHED, _rows[0]
        import copy
        ok = copy.deepcopy(_rows)
        ok[0]["apparitions"][0]["tier"] = "gold"; ok[0]["apparitions"][0]["galleries"] = ["Art"]
        validate_publications(ok, {"tiers": ["gold"], "galleries": ["Art"]})
        try:
            validate_publications(ok, {"tiers": [], "galleries": []}); assert False, "tier not rejected"
        except ValueError:
            pass
        apply_update(_rows, _uuids[0], {"title": "T2", "description": "D2", "scheduleTs": s1,
                                        "price": 3, "tier": "gold", "galleries": ["Art"]})
        a = _rows[0]["apparitions"][0]
        assert a["urlElsePublicationName"] == "T2" and a["priceIfNotFree"] == 3.0
        assert a["tier"] == "gold" and a["galleries"] == ["Art"], a
        apply_update(_rows, _uuids[0], {"title": "T3", "description": "D3", "scheduleTs": s1})
        a = _rows[0]["apparitions"][0]
        assert "priceIfNotFree" not in a and "tier" not in a and "galleries" not in a, a
    print("publish_next selfcheck OK")


# ---------------------------------------------------------------------------
# HTTP: gallery + queue endpoints
# ---------------------------------------------------------------------------

_PAGE_TMPL = r"""<!DOCTYPE html>
<html lang="en"><head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Publish next</title>
<style>
  body { font-family: sans-serif; margin: 0; background: #1a1a1a; color: #eee; }
  header { position: sticky; top: 0; background: #111; padding: 12px 20px; z-index: 10;
           display: flex; align-items: center; gap: 16px; border-bottom: 1px solid #333; }
  header h1 { margin: 0; font-size: 18px; }
  #publish-btn, #stage-btn { padding: 10px 24px; font-size: 16px; background: #4caf50; color: white;
                 border: none; border-radius: 6px; cursor: pointer; }
  #stage-btn { background: #2196f3; }
  #publish-btn:disabled, #stage-btn:disabled { background: #444; color: #888; cursor: not-allowed; }
  #stage-status { font-size: 13px; color: #8ac; }
  #queue-count { font-size: 15px; color: #aaa; }
  main { padding: 20px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 16px; }
  .card { background: #2a2a2a; border-radius: 8px; padding: 10px; display: flex; flex-direction: column; gap: 8px; }
  .card img { max-width: 100%; max-height: 240px; border-radius: 4px; object-fit: contain; background: #000; }
  .card .name { font-size: 13px; word-break: break-all; color: #ccc; }
  .card .sched { font-size: 12px; color: #aaa; }
  .card.queued { opacity: 0.6; border: 1px solid #4caf50; }
  .badge { display: inline-block; padding: 2px 8px; background: #4caf50; color: #fff;
           border-radius: 4px; font-size: 12px; }
  .actions { display: flex; gap: 8px; }
  .actions button, .actions a { flex: 1; padding: 8px; border: none; border-radius: 4px;
                    cursor: pointer; font-size: 14px; text-align: center; text-decoration: none;
                    box-sizing: border-box; }
  .btn-add { background: #2196f3; color: white; }
  .btn-del { background: #b03030; color: white; }
  .btn-view { background: #555; color: white; }
  .form { display: none; flex-direction: column; gap: 6px; }
  .form label { font-size: 12px; color: #aaa; }
  .form input, .form textarea { background: #1a1a1a; color: #eee; border: 1px solid #444;
                                border-radius: 4px; padding: 6px; font-family: inherit; font-size: 13px; }
  .form textarea { min-height: 70px; resize: vertical; }
  .form-actions { display: flex; gap: 6px; margin-top: 4px; }
  .form-actions button { flex: 1; padding: 8px; border: none; border-radius: 4px;
                         cursor: pointer; font-size: 13px; }
  .btn-ai { background: #7b3ec1; color: white; }
  .btn-save { background: #4caf50; color: white; }
  .btn-cancel { background: #555; color: white; }
  .err { color: #ff8080; font-size: 12px; min-height: 14px; }
</style>
</head><body>
<header>
  <h1>Publish next</h1>
  <span id="queue-count">0 queued</span>
  <select id="ai-model" title="AI model for Generate with AI">
    <option value="__OPENROUTER_MODEL__">OpenRouter (free)</option>
    <option value="__AI_MODEL__">Claude (subscription)</option>
  </select>
  <span id="stage-status"></span>
  <button id="stage-btn" onclick="stageQueue()" disabled>Add to publish pad</button>
  <button id="publish-btn" onclick="publishQueue()" disabled>Publish</button>
</header>
<main>
  <div class="grid" id="gallery">__CARDS__</div>
</main>
<script>
const EXISTING_TS = __EXISTING_TS__;   // already-scheduled background instants (occupancy)
const SCHEDULES = __SCHEDULES__;       // [{name, per_slot, slots:[ts,...]}]  slots: upcoming canonical instants, ascending
const LABELS = __LABELS__;             // {ts: "YYYY-MM-DDTHH:MM"} wall clock in the schedule TZ, browser-TZ-proof
const PENDING = __PENDING__;           // already-queued entries from publications.json

const queue = []; // [{cardId, path, title, description, price, scheduleTs, tier, galleries, uuid?}]

// >>> scheduler core (sliced verbatim by test_publish_next.py) >>>
// Slots are absolute instants generated server-side in the schedule's timezone,
// so nothing here reads the browser clock — `firefox --private-window` spoofing
// Date to UTC can no longer make a taken slot look free (the bug this replaced).
const LABEL_TO_TS = Object.fromEntries(Object.entries(LABELS).map(([ts, s]) => [s, +ts]));

function profileByName(name) {
  return SCHEDULES.find(s => s.name === name) || SCHEDULES[0];
}
function presetForTs(ts) {               // profile whose slot list holds ts, else custom
  if (!ts) return SCHEDULES[0].name;
  return SCHEDULES.find(s => s.slots.includes(ts))?.name ?? "__custom__";
}
function nextSlotForProfile(p) {         // earliest upcoming slot with room (< per_slot)
  const taken = EXISTING_TS.concat(queue.map(e => e.scheduleTs).filter(Boolean));  // background + queue
  const slot = p.slots.find(s => taken.filter(t => t === s).length < p.per_slot);
  return slot ?? p.slots[p.slots.length - 1];  // ponytail: horizon full -> reuse last; widen _SLOT_HORIZON_WEEKS
}
function tsToLocalInput(ts) {            // schedule-TZ label; browser-local only for an unlabeled custom instant
  if (LABELS[ts]) return LABELS[ts];
  const d = new Date(ts * 1000), pad = n => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
function localInputToTs(s) {             // known slot -> exact instant (TZ-proof); else read as browser-local
  return (s in LABEL_TO_TS) ? LABEL_TO_TS[s]
                            : Math.floor(new Date(s).getTime() / 1000);  // ponytail: custom time assumes browser TZ
}
// <<< scheduler core <<<
function applyPreset(sel, cardId) {      // preset change -> refill the picker
  if (sel.value === "__custom__") return;
  const card = document.getElementById(cardId);
  card.querySelector(".f-schedule").value = tsToLocalInput(nextSlotForProfile(profileByName(sel.value)));
}

function refreshCount() {
  const onPad = queue.filter(e => e.uuid).length; // entries persisted to publications.json
  document.getElementById("queue-count").textContent = queue.length + " queued";
  document.getElementById("publish-btn").disabled = queue.length === 0;
  document.getElementById("stage-btn").disabled = queue.length === onPad; // nothing fresh to stage
  document.getElementById("stage-status").textContent = onPad ? onPad + " on pad" : "";
}

function openForm(cardId) {
  const card = document.getElementById(cardId);
  const form = card.querySelector(".form");
  const entry = queue.find(e => e.cardId === cardId);
  const q = sel => form.querySelector(sel);
  const presetEl = q(".f-preset");
  if (entry) {
    q(".f-title").value = entry.title || "";
    q(".f-description").value = entry.description || "";
    q(".f-price").value = (entry.price == null) ? "" : entry.price;
    q(".f-schedule").value = entry.scheduleTs ? tsToLocalInput(entry.scheduleTs) : "";
    if (presetEl) presetEl.value = presetForTs(entry.scheduleTs);
    if (q(".f-tier")) q(".f-tier").value = entry.tier || "";
    form.querySelectorAll(".f-gallery").forEach(cb => cb.checked = (entry.galleries || []).includes(cb.value));
  } else {
    const p = SCHEDULES[0];
    if (presetEl) presetEl.value = p.name;
    q(".f-schedule").value = tsToLocalInput(nextSlotForProfile(p));
  }
  form.style.display = "flex";
  card.querySelector(".actions").style.display = "none";
}
function closeForm(cardId) {
  const card = document.getElementById(cardId);
  card.querySelector(".form").style.display = "none";
  card.querySelector(".actions").style.display = "flex";
}

async function delCard(cardId, path) {
  if (!confirm("Delete " + path + " from disk?")) return;
  const idx = queue.findIndex(e => e.cardId === cardId);
  if (idx >= 0) queue.splice(idx, 1);
  const r = await fetch("/delete", {method:"POST", headers:{"Content-Type":"application/json"},
                                     body: JSON.stringify({path})});
  const d = await r.json();
  if (!d.ok) { alert("Delete failed: " + (d.error || "unknown")); return; }
  document.getElementById(cardId).remove();
  refreshCount();
}

async function aiGen(cardId, path) {
  const card = document.getElementById(cardId);
  const err = card.querySelector(".err");
  const btn = card.querySelector(".btn-ai");
  err.textContent = "";
  btn.disabled = true; btn.textContent = "Generating...";
  try {
    const model = document.getElementById("ai-model").value;
    const r = await fetch("/ai", {method:"POST", headers:{"Content-Type":"application/json"},
                                   body: JSON.stringify({path, model})});
    const d = await r.json();
    if (!r.ok || d.error) { err.textContent = d.error || ("HTTP " + r.status); return; }
    card.querySelector(".f-title").value = d.title || "";
    card.querySelector(".f-description").value = d.description || "";
  } catch (e) {
    err.textContent = String(e);
  } finally {
    btn.disabled = false; btn.textContent = "Generate with AI";
  }
}

function readForm(card) {
  const g = sel => card.querySelector(sel);
  const title = g(".f-title").value.trim();
  const desc = g(".f-description").value.trim();
  const priceRaw = g(".f-price").value.trim();
  const sched = g(".f-schedule").value;
  const err = g(".err");
  if (!title) { err.textContent = "title required"; return null; }
  if (!desc) { err.textContent = "description required"; return null; }
  if (!sched) { err.textContent = "schedule required"; return null; }
  const out = {title, description: desc, scheduleTs: localInputToTs(sched)};
  if (priceRaw !== "") {
    const p = parseFloat(priceRaw);
    if (isNaN(p) || p < 0) { err.textContent = "price must be >= 0"; return null; }
    out.price = p;
  }
  const tierEl = g(".f-tier");
  if (tierEl && tierEl.value) out.tier = tierEl.value;
  const gals = [...card.querySelectorAll(".f-gallery:checked")].map(cb => cb.value);
  if (gals.length) out.galleries = gals;
  return out;
}

async function saveCard(cardId, path) {
  const card = document.getElementById(cardId);
  const fields = readForm(card);
  if (!fields) return;
  const entry = queue.find(e => e.cardId === cardId);
  if (entry) {                       // re-edit
    Object.assign(entry, {price: undefined, tier: undefined, galleries: undefined}, fields);
    if (entry.uuid) {
      const btn = card.querySelector(".btn-save");
      btn.disabled = true; btn.textContent = "Saving...";
      const r = await fetch("/update", {method:"POST", headers:{"Content-Type":"application/json"},
                                        body: JSON.stringify({uuid: entry.uuid, ...fields})});
      const d = await r.json();
      btn.disabled = false; btn.textContent = "Save changes";
      if (!r.ok || d.error) { card.querySelector(".err").textContent = d.error || ("HTTP " + r.status); return; }
    }
    card.querySelector(".name").lastChild.textContent = " " + fields.title;
  } else {                           // first save of a candidate
    queue.push({cardId, path, ...fields});
    card.classList.add("queued");
    const add = card.querySelector(".btn-add");
    add.textContent = "Edit"; add.disabled = false; add.onclick = () => openForm(cardId);
    if (!card.querySelector(".badge")) {
      const b = document.createElement("span"); b.className = "badge"; b.textContent = "queued";
      card.querySelector(".name").prepend(b, " ");
    }
  }
  card.querySelector(".form").style.display = "none";
  card.querySelector(".actions").style.display = "flex";
  refreshCount();
}

async function stageQueue() {
  const fresh = queue.filter(e => !e.uuid); // uuid-carrying ones are already persisted
  if (fresh.length === 0) return;
  const btn = document.getElementById("stage-btn");
  btn.disabled = true; btn.textContent = "Adding...";
  const r = await fetch("/stage", {method:"POST", headers:{"Content-Type":"application/json"},
                                    body: JSON.stringify({entries: fresh.map(({cardId, ...rest}) => rest)})});
  const d = await r.json();
  btn.textContent = "Add to publish pad";
  if (!r.ok || d.error) { alert("Add to pad failed: " + (d.error || ("HTTP " + r.status))); btn.disabled = false; return; }
  fresh.forEach((e, i) => {
    e.uuid = d.uuids[i]; // now indistinguishable from a pre-loaded pending entry
    // Persisted in publications.json now -> no RAM-only remove; drop Delete.
    const card = document.getElementById(e.cardId);
    const del = card && card.querySelector(".btn-del");
    if (del) del.remove();
  });
  refreshCount(); // recomputes "N on pad" and disables stage-btn (nothing fresh left)
}

async function publishQueue() {
  if (queue.length === 0) return;
  const btn = document.getElementById("publish-btn");
  btn.disabled = true; btn.textContent = "Publishing...";
  const entries = queue.map(({cardId, ...rest}) => rest);
  const r = await fetch("/publish", {method:"POST", headers:{"Content-Type":"application/json"},
                                      body: JSON.stringify({entries})});
  const d = await r.json();
  document.body.innerHTML = `<main style="padding:40px"><h1>Publish result</h1>
    <p>Published: ${d.published || 0}</p>
    <p>Failed: ${d.failed || 0}</p>
    ${d.error ? `<pre style="color:#ff8080">${d.error}</pre>` : ""}
    <p>You can close this tab.</p></main>`;
}

// Pre-queue entries already sitting in publications.json (state=unpublished).
for (const p of PENDING) {
  queue.push({cardId: p.cardId, uuid: p.uuid, path: p.path,
              title: p.title, description: p.description, scheduleTs: p.scheduleTs, price: p.price,
              tier: p.tier, galleries: p.galleries});
}
refreshCount();
</script>
</body></html>
"""


def _tier_gallery_fields(config):
    if not config.get("tiers") and not config.get("galleries"):
        return ""
    tiers = "".join(f'<option value="{html.escape(t, quote=True)}">{html.escape(t)}</option>'
                    for t in config.get("tiers", []))
    gals = "".join(f'<label><input type="checkbox" class="f-gallery" '
                   f'value="{html.escape(g, quote=True)}"> {html.escape(g)}</label>'
                   for g in config.get("galleries", []))
    return (f'<label>Tier <select class="f-tier"><option value="">(none)</option>{tiers}</select></label>'
            f'<div class="galleries"><span>Galleries</span>{gals}</div>')


def _preset_options(schedules):
    opts = "".join(f'<option value="{html.escape(s["name"], quote=True)}">'
                   f'{html.escape(s["name"])}</option>' for s in schedules)
    return opts + '<option value="__custom__">(custom)</option>'


def _card_form_html(cid, js_path, tg, save_label, preset_opts):
    """The shared edit form rendered on every card (candidate + pending). Only the
    save-button label differs between the two call sites."""
    return f"""  <div class="form">
    <label>Title <input type="text" class="f-title" maxlength="50"></label>
    <label>Description <textarea class="f-description"></textarea></label>
    <label>Price (optional) <input type="number" class="f-price" min="0" step="0.01"></label>
    <label>Schedule preset <select class="f-preset" onchange="applyPreset(this, '{cid}')">{preset_opts}</select></label>
    <label>Schedule <input type="datetime-local" class="f-schedule"></label>
    {tg}
    <button class="btn-ai" onclick="aiGen('{cid}', {js_path})">Generate with AI</button>
    <div class="err"></div>
    <div class="form-actions">
      <button class="btn-save" onclick="saveCard('{cid}', {js_path})">{save_label}</button>
      <button class="btn-cancel" onclick="closeForm('{cid}')">Cancel</button>
    </div>
  </div>"""


class GalleryHandler(BaseHTTPRequestHandler):
    thumb_dir = ""
    thumb_map: dict[str, str] = {}
    candidate_paths: list[str] = []      # new picks from picked/, add-able
    pending: list[dict] = []             # already-queued entries from publications.json
    existing_ts: list[int] = []
    ai_model = DEFAULT_MODEL
    openrouter_model = ""
    ai_timeout = 300
    json_path = "publications.json"
    config: dict = {}
    schedules: list = _schedule_data({})
    publish_done: dict | None = None

    def _build_page(self) -> str:
        cards = []
        pending_js = []
        tg = _tier_gallery_fields(self.config)
        preset_opts = _preset_options(self.schedules)
        zone = _zone(self.config.get("schedule", {}))
        # Pre-queued cards: entries already in publications.json (state=unpublished).
        for idx, e in enumerate(self.pending):
            thumb_path = self.thumb_map.get(e["path"], e["path"])
            rel = os.path.relpath(thumb_path, self.thumb_dir)
            safe_rel = html.escape(rel, quote=True)
            safe_title = html.escape(e["title"], quote=True)
            view_href = html.escape("/original?path=" + urllib.parse.quote(e["path"]), quote=True)
            ts = e.get("scheduleTs")
            sched = (datetime.fromtimestamp(ts, zone).strftime("%a %d %b %Y %H:%M")
                     if ts else "no schedule")
            cid = f"pending_{idx}"
            js_pathp = html.escape(json.dumps(e["path"]), quote=True)
            cards.append(f"""<div class="card queued" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_title}">
  <div class="name"><span class="badge">queued</span> {safe_title}</div>
  <div class="sched">{html.escape(sched)}</div>
  <div class="actions">
    <a class="btn-view" href="{view_href}" target="_blank" rel="noopener">View</a>
    <button class="btn-add" onclick="openForm('{cid}')">Edit</button>
  </div>
{_card_form_html(cid, js_pathp, tg, "Save changes", preset_opts)}
</div>""")
            pending_js.append({"cardId": cid, "uuid": e["uuid"], "path": e["path"],
                               "title": e["title"], "description": e.get("description"),
                               "scheduleTs": ts, "price": e.get("price"),
                               "tier": e.get("tier"), "galleries": e.get("galleries", [])})
        # Add-able cards: new picks from picked/.
        for idx, orig_path in enumerate(self.candidate_paths):
            thumb_path = self.thumb_map.get(orig_path, orig_path)
            rel = os.path.relpath(thumb_path, self.thumb_dir)
            filename = os.path.basename(orig_path)
            safe_name = html.escape(filename, quote=True)
            safe_rel = html.escape(rel, quote=True)
            js_path = html.escape(json.dumps(orig_path), quote=True)
            view_href = html.escape("/original?path=" + urllib.parse.quote(orig_path), quote=True)
            cid = f"card_{idx}"
            cards.append(f"""<div class="card" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_name}">
  <div class="name">{safe_name}</div>
  <div class="actions">
    <a class="btn-view" href="{view_href}" target="_blank" rel="noopener">View</a>
    <button class="btn-add" onclick="openForm('{cid}')">Add</button>
    <button class="btn-del" onclick="delCard('{cid}', {js_path})">Delete</button>
  </div>
{_card_form_html(cid, js_path, tg, "Save to queue", preset_opts)}
</div>""")
        # Labels for every instant that can land in a datetime-local input: slot
        # presets and pending scheduleTs (background occupancy is never rendered).
        # Rendered in the schedule TZ so the picker shows the right wall clock even
        # when the private window forces Date to UTC.
        slot_ts = [ts for p in self.schedules for ts in p.get("slots", [])]
        pending_ts = [e["scheduleTs"] for e in pending_js if e.get("scheduleTs")]
        labels = _ts_labels(zone, slot_ts + pending_ts)
        page = _PAGE_TMPL
        page = page.replace("__CARDS__", "\n".join(cards))
        page = page.replace("__PENDING__", json.dumps(pending_js))
        page = page.replace("__EXISTING_TS__", json.dumps(self.existing_ts))
        page = page.replace("__SCHEDULES__", json.dumps(self.schedules))
        page = page.replace("__LABELS__", json.dumps(labels))
        page = page.replace("__AI_MODEL__", html.escape(self.ai_model, quote=True))
        page = page.replace("__OPENROUTER_MODEL__", html.escape(self.openrouter_model, quote=True))
        return page

    def _json_body(self) -> dict:
        length = int(self.headers.get("Content-length", 0))
        return json.loads(self.rfile.read(length) or b"{}")

    def _send(self, code: int, obj: dict) -> None:
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-type", "application/json")
        self.send_header("Content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _guess_mime(self, path: str) -> str:
        return MIME.get(os.path.splitext(path)[1].lower(), "application/octet-stream")

    def do_GET(self):
        if self.path in ("/", "/index.html"):
            page = self._build_page().encode("utf-8")
            self.send_response(200)
            self.send_header("Content-type", "text/html; charset=utf-8")
            self.send_header("Content-length", str(len(page)))
            self.end_headers()
            self.wfile.write(page)
        elif self.path.startswith("/thumbs/"):
            rel_path = self.path[len("/thumbs/"):]
            full_path = os.path.normpath(os.path.join(self.thumb_dir, rel_path))
            if full_path.startswith(os.path.normpath(self.thumb_dir)) and os.path.isfile(full_path):
                self.send_response(200)
                self.send_header("Content-type", self._guess_mime(full_path))
                self.end_headers()
                with open(full_path, "rb") as f:
                    self.wfile.write(f.read())
            else:
                self.send_response(404); self.end_headers()
        elif self.path.startswith("/original?"):
            # Full-res original for a card's thumbnail, opened in a new tab.
            # Allow-list = thumb_map keys (exactly the candidate + pending
            # originals); anything else 404s, so no arbitrary-path read.
            qs = urllib.parse.urlparse(self.path).query
            req_path = urllib.parse.parse_qs(qs).get("path", [""])[0]
            if req_path in self.thumb_map and os.path.isfile(req_path):
                self.send_response(200)
                self.send_header("Content-type", self._guess_mime(req_path))
                self.end_headers()
                with open(req_path, "rb") as f:
                    self.wfile.write(f.read())
            else:
                self.send_response(404); self.end_headers()
        else:
            self.send_response(404); self.end_headers()

    def do_POST(self):
        try:
            if self.path == "/delete":
                data = self._json_body()
                path = data.get("path", "")
                try:
                    os.remove(path)
                except OSError as e:
                    self._send(200, {"ok": False, "error": str(e)}); return
                # ponytail: gallery renders once; concurrent refresh vs. delete
                # would race this dict, but in practice the tab is opened once.
                self.__class__.thumb_map.pop(path, None)
                self._send(200, {"ok": True})

            elif self.path == "/ai":
                data = self._json_body()
                path = data.get("path", "")
                model = data.get("model") or self.ai_model
                if model not in (self.ai_model, self.openrouter_model):
                    self._send(400, {"error": f"model not allowed: {model}"}); return
                # Send the thumbnail to Claude, not the full-res original — same
                # visual info for a fraction of the tokens/latency.
                image_for_ai = self.thumb_map.get(path, path)
                log.debug("AI metadata: %s (model=%s, timeout=%ss)",
                          image_for_ai, model, self.ai_timeout)
                try:
                    title, desc = generate_metadata(image_for_ai, model, timeout=self.ai_timeout)
                except RuntimeError as e:
                    log.debug("AI metadata failed: %s", e)
                    self._send(400, {"error": str(e)}); return
                self._send(200, {"title": title, "description": desc})

            elif self.path == "/publish":
                data = self._json_body()
                entries = data.get("entries", []) or []
                if not entries:
                    self._send(400, {"error": "empty queue"}); return
                # Entries with a uuid are already in publications.json (the
                # pre-queued pending set) — publish them as-is. The rest are new
                # picks: append them first, then publish. Existing go first.
                existing = [e for e in entries if e.get("uuid")]
                new = [e for e in entries if not e.get("uuid")]
                try:
                    new_uuids = (write_publications(new, self.json_path, self.config)
                                 if new else [])
                except Exception as e:
                    self._send(400, {"error": f"schema/write failed: {e}"}); return

                all_entries = existing + new
                all_uuids = [e["uuid"] for e in existing] + new_uuids
                published, failed, err = publish_batch(all_entries, all_uuids, self.json_path)
                result = {"published": published, "failed": failed}
                if err:
                    result["error"] = err
                self.__class__.publish_done = result
                self._send(200, result)

            elif self.path == "/stage":
                data = self._json_body()
                entries = data.get("entries", []) or []
                if not entries:
                    self._send(400, {"error": "nothing to stage"}); return
                try:
                    uuids = write_publications(entries, self.json_path, self.config)
                except Exception as e:
                    self._send(400, {"error": f"schema/write failed: {e}"}); return
                # No publish_done set → serve loop keeps running, session stays alive.
                self._send(200, {"staged": len(uuids), "uuids": uuids})

            elif self.path == "/update":
                data = self._json_body()
                u = data.get("uuid")
                if not u:
                    self._send(400, {"error": "missing uuid"}); return
                with open(self.json_path) as f:
                    pubs = json.load(f)
                try:
                    apply_update(pubs, u, data)
                except KeyError:
                    self._send(404, {"error": f"uuid not found: {u}"}); return
                try:
                    validate_publications(pubs, self.config)
                except Exception as e:
                    self._send(400, {"error": f"invalid: {e}"}); return
                _atomic_write_json(self.json_path, pubs)
                self._send(200, {"ok": True})
            else:
                self.send_response(404); self.end_headers()
        except Exception as e:
            try:
                self._send(500, {"error": str(e)})
            except Exception:
                pass

    def log_message(self, format, *args):
        log.debug("http %s - %s", self.address_string(), format % args)


# ---------------------------------------------------------------------------
# Publications.json write
# ---------------------------------------------------------------------------

def write_publications(entries: list[dict], json_path: str, config: dict | None = None) -> list[str]:
    """Append entries as state=unpublished, validate, atomic write. Returns their UUIDs in order.
    config: tier/gallery allow-list; defaults to a fresh load_config(cwd) for standalone callers."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        data = []

    new_uuids = []
    now = int(time.time())
    for i, e in enumerate(entries):
        path = e["path"]
        sha = compute_sha512(path)
        if not e.get("scheduleTs"):
            raise ValueError("scheduleTs required")
        ts = int(e["scheduleTs"])
        apparition = {
            "platformName": "deviantart",
            "state": STATE_UNPUBLISHED,
            "urlElsePublicationName": e["title"],
            "apparitionTimestampIfDifferentThanSubmission": ts,
        }
        price = e.get("price")
        _set_or_pop(apparition, "priceIfNotFree", float(price) if price not in (None, "") else None)
        _set_or_pop(apparition, "tier", e.get("tier") or None)
        _set_or_pop(apparition, "galleries", list(e["galleries"]) if e.get("galleries") else None)
        u = str(uuid.uuid4())
        new_uuids.append(u)
        data.append({
            "uuid": u,
            "submissionTimestamp": now,
            "description": e.get("description", ""),
            "files": [{
                "basename": os.path.basename(path),
                "sha512sum": sha,
            }],
            "apparitions": [apparition],
        })

    validate_publications(data, config if config is not None else load_config(Path.cwd()))
    _atomic_write_json(json_path, data)
    return new_uuids


def _set_or_pop(d, k, v):
    if v in (None, [], ""):
        d.pop(k, None)
    else:
        d[k] = v


def apply_update(pubs, uuid_, fields):
    """Patch the publication (and its DA apparition) with uuid_ in place.
    Cleared price/tier/galleries are removed. Raises KeyError if not found."""
    for p in pubs:
        if p.get("uuid") == uuid_:
            break
    else:
        raise KeyError(uuid_)
    p["description"] = fields.get("description", p.get("description", ""))
    app = deviantart_apparition(p)
    app["urlElsePublicationName"] = fields["title"]
    app["apparitionTimestampIfDifferentThanSubmission"] = int(fields["scheduleTs"])
    price = fields.get("price")
    _set_or_pop(app, "priceIfNotFree", float(price) if price not in (None, "") else None)
    _set_or_pop(app, "tier", fields.get("tier") or None)
    _set_or_pop(app, "galleries", list(fields["galleries"]) if fields.get("galleries") else None)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def serve(thumb_dir: str, thumb_map: dict[str, str], candidate_paths: list[str],
          pending: list[dict], args, config: dict) -> dict | None:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.candidate_paths = candidate_paths
    GalleryHandler.pending = pending
    GalleryHandler.existing_ts = _existing_ts(args.json)
    GalleryHandler.ai_model = args.ai_model
    GalleryHandler.openrouter_model = args.openrouter_model
    GalleryHandler.ai_timeout = args.ai_timeout
    GalleryHandler.json_path = args.json
    GalleryHandler.config = config          # was: load_config(Path.cwd())
    GalleryHandler.schedules = _schedule_data(config["schedule"])
    GalleryHandler.publish_done = None

    server = ThreadingHTTPServer(("127.0.0.1", args.port), GalleryHandler)
    server.timeout = 0.5
    url = f"http://127.0.0.1:{args.port}"
    print(f"Gallery: {url}")
    try:
        subprocess.Popen(
            ["firefox", "--private-window", "--no-remote", url],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    except FileNotFoundError:
        print("(open the URL yourself; firefox not found)")

    try:
        while GalleryHandler.publish_done is None:
            server.handle_request()
    except KeyboardInterrupt:
        print("\nShutting down...")
    finally:
        server.server_close()
    return GalleryHandler.publish_done


def main() -> int:
    parser = argparse.ArgumentParser(description="Unified publish workflow (gallery + AI + Chromium DA).")
    parser.add_argument("-n", type=int, default=10)
    parser.add_argument("--data-dir", default=None,
                        help="publication database dir (publications.json + images + browser session); default: CWD")
    parser.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    parser.add_argument("--ai-model", default=DEFAULT_MODEL)
    parser.add_argument("--openrouter-model",
                        # ponytail: free :free ids churn on OpenRouter; this is the current
                        # free model with both vision and structured_outputs. Override via flag.
                        default="openrouter/google/gemma-4-26b-a4b-it:free",
                        help="free vision model for the OpenRouter option; needs $OPENROUTER_KEY")
    parser.add_argument("--ai-timeout", type=int, default=300,
                        help="seconds to wait for an AI title/description (default: 300)")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="debug logging (HTTP requests, AI/llm calls, publish steps)")
    args = parser.parse_args()

    setup_logging(args.verbose)

    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    config = load_config(data_dir)
    configure(args.data_dir, config)  # points da_publish + echo helpers at the db dir
    args.json = args.json or str(data_dir / "publications.json")
    publicable_dirs = [str(data_dir / d) for d in config["publicable"]]

    print("Finding unpublished images...")
    candidates = find_candidates(publicable_dirs, args.json, args.n)
    pending = load_pending_entries(args.json)
    log.debug("found %d candidate(s), %d pending", len(candidates), len(pending))
    if not candidates and not pending:
        print("Nothing to publish (no new picks, no pending queue).")
        return 0

    msg = f"{len(candidates)} new pick(s)"
    if pending:
        msg += f", {len(pending)} already queued"
    print(f"Found {msg}. Generating thumbnails...")
    with tempfile.TemporaryDirectory(prefix="publish-next-") as thumb_dir:
        thumb_map = generate_thumbnails(candidates + [e["path"] for e in pending], thumb_dir)
        log.debug("generated %d thumbnail(s) in %s", len(thumb_map), thumb_dir)
        result = serve(thumb_dir, thumb_map, candidates, pending, args, config)

    if result is None:
        print("No publish action taken.")
        return 0
    print(f"Done. published={result.get('published',0)} failed={result.get('failed',0)}")
    if result.get("error"):
        print(f"error: {result['error']}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    if "--selfcheck" in sys.argv:
        _selfcheck()
    else:
        sys.exit(main())
