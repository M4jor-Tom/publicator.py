#!/usr/bin/env python3
"""Unified publish workflow: gallery + AI metadata + Playwright Chromium batch.

Single entrypoint. Scans picked/, opens a dark gallery on 127.0.0.1:PORT, lets
you delete images or add them to a queue (with AI-generated title/description
and per-entry schedule override), then writes to publications.json and drives a
Chromium persistent context through the DeviantArt submission flow.
"""

import argparse
import base64
import hashlib
import html
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from echo_first_unpublished_publication_data import find_art_path, format_schedule
from publish_deviantart import pick_schedule, set_checkbox, type_tags

PKG = Path(__file__).resolve().parent  # code assets (schema) travel with the package
# Runtime state lives in the caller's CWD (the app is invoked from the Art data dir).
SESSION_DIR = Path.cwd() / ".deviantart-session"
IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tiff", ".mp4"}
MIME = {
    ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png",
    ".webp": "image/webp", ".gif": "image/gif", ".mp4": "video/mp4",
}
STATE_UNPUBLISHED = "unpublished"
STATE_PUBLISHED = "published_or_scheduled"

ANTHROPIC_URL = "https://api.anthropic.com/v1/messages"


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


def find_candidates(directory: str, json_path: str, limit: int) -> list[str]:
    publicated = load_publicated_hashes(json_path)
    images = collect_images(directory)
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
# Slot computation (mirrors client-side JS)
# ---------------------------------------------------------------------------

def _next_tuesday_8pm_after(timestamp: int) -> datetime:
    dt = datetime.fromtimestamp(timestamp, tz=timezone.utc)
    days_ahead = (1 - dt.weekday()) % 7  # Python: Mon=0, Tue=1
    if days_ahead == 0:
        days_ahead = 7
    nxt = dt + timedelta(days=days_ahead)
    return nxt.replace(hour=20, minute=0, second=0, microsecond=0)


def compute_next_slot(max_existing_ts: int, added_count: int) -> int:
    """Slot timestamp (UTC) for the `added_count`-th (0-indexed) new entry.
    Packing: 2 per Tuesday 20:00 UTC, starting the Tuesday after max_existing_ts."""
    base = _next_tuesday_8pm_after(max_existing_ts)
    weeks = added_count // 2
    slot = base + timedelta(days=7 * weeks)
    return int(slot.timestamp())


def _max_existing_ts(json_path: str) -> int:
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return int(time.time())
    max_ts = 0
    for entry in data:
        for app in entry.get("apparitions", []):
            ts = app.get("apparitionTimestampIfDifferentThanSubmission")
            if ts and ts > max_ts:
                max_ts = ts
    return max_ts or int(time.time())


def _selfcheck() -> None:
    anchor = int(datetime(2026, 1, 4, tzinfo=timezone.utc).timestamp())
    s0 = compute_next_slot(anchor, 0)
    s1 = compute_next_slot(anchor, 1)
    s2 = compute_next_slot(anchor, 2)
    s3 = compute_next_slot(anchor, 3)
    assert s0 == s1, f"first two share slot, got {s0} vs {s1}"
    assert s2 == s0 + 7 * 24 * 3600, f"third rolls a week, got {s2 - s0}"
    assert s3 == s2, f"fourth pairs with third, got {s3} vs {s2}"
    dt = datetime.fromtimestamp(s0, tz=timezone.utc)
    assert dt.weekday() == 1, f"slot must be Tuesday UTC, got {dt}"
    assert dt.hour == 20, f"slot must be 20:00 UTC, got {dt}"
    print("selfcheck OK")


# ---------------------------------------------------------------------------
# AI: Claude vision → (title, description)
# ---------------------------------------------------------------------------

_AI_PROMPT = (
    "Look at this artwork and reply with a strict JSON object, nothing else, "
    'shape {"title": "...", "description": "..."}. '
    "Title: <= 50 characters, evocative, no hashtags, no emojis. "
    "Description: 2-3 sentences, artist voice, no hashtags, no emojis."
)


def _media_type(image_path: str) -> str:
    return MIME.get(os.path.splitext(image_path)[1].lower(), "image/jpeg")


def generate_metadata(image_path: str, provider: str = "claude",
                      model: str = "claude-opus-4-7") -> tuple[str, str]:
    if provider != "claude":
        raise NotImplementedError(f"provider {provider!r} not implemented")

    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        raise RuntimeError("ANTHROPIC_API_KEY not set")

    with open(image_path, "rb") as f:
        b64 = base64.b64encode(f.read()).decode("ascii")

    payload = {
        "model": model,
        "max_tokens": 512,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image", "source": {
                    "type": "base64",
                    "media_type": _media_type(image_path),
                    "data": b64,
                }},
                {"type": "text", "text": _AI_PROMPT},
            ],
        }],
    }
    req = urllib.request.Request(
        ANTHROPIC_URL,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "x-api-key": api_key,
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=90) as resp:
            body = json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", errors="replace")[:400]
        raise RuntimeError(f"Anthropic API {e.code}: {detail}") from e
    except urllib.error.URLError as e:
        raise RuntimeError(f"Anthropic API unreachable: {e}") from e

    try:
        text = body["content"][0]["text"]
    except (KeyError, IndexError, TypeError):
        raise RuntimeError(f"Unexpected API response shape: {body!r}"[:400])

    # Strip fenced code block if present, then find the JSON object.
    text = text.strip()
    m = re.search(r"\{.*\}", text, re.DOTALL)
    if not m:
        raise RuntimeError(f"No JSON object in AI reply: {text!r}"[:400])
    try:
        parsed = json.loads(m.group(0))
    except json.JSONDecodeError as e:
        raise RuntimeError(f"AI reply not valid JSON: {e}: {text!r}"[:400]) from e

    title = str(parsed.get("title", "")).strip()
    description = str(parsed.get("description", "")).strip()
    if not title or not description:
        raise RuntimeError(f"AI reply missing title/description: {parsed!r}")
    return title, description


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
  #publish-btn { padding: 10px 24px; font-size: 16px; background: #4caf50; color: white;
                 border: none; border-radius: 6px; cursor: pointer; }
  #publish-btn:disabled { background: #444; color: #888; cursor: not-allowed; }
  #queue-count { font-size: 15px; color: #aaa; }
  main { padding: 20px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 16px; }
  .card { background: #2a2a2a; border-radius: 8px; padding: 10px; display: flex; flex-direction: column; gap: 8px; }
  .card img { max-width: 100%; max-height: 240px; border-radius: 4px; object-fit: contain; background: #000; }
  .card .name { font-size: 13px; word-break: break-all; color: #ccc; }
  .card.queued { opacity: 0.5; border: 1px solid #4caf50; }
  .badge { display: inline-block; padding: 2px 8px; background: #4caf50; color: #fff;
           border-radius: 4px; font-size: 12px; }
  .actions { display: flex; gap: 8px; }
  .actions button { flex: 1; padding: 8px; border: none; border-radius: 4px; cursor: pointer;
                    font-size: 14px; }
  .btn-add { background: #2196f3; color: white; }
  .btn-del { background: #b03030; color: white; }
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
  <button id="publish-btn" onclick="publishQueue()" disabled>Publish</button>
</header>
<main>
  <div class="grid" id="gallery">__CARDS__</div>
</main>
<script>
const INITIAL_MAX_TS = __INITIAL_MAX_TS__;
const AI_PROVIDER = "__AI_PROVIDER__";
const AI_MODEL = "__AI_MODEL__";

const queue = []; // [{cardId, path, title, description, price, scheduleTs}]

function nextTuesday8pmUTC(afterTs) {
  const d = new Date(afterTs * 1000);
  let days = (2 - d.getUTCDay() + 7) % 7; // JS: Sun=0, Tue=2
  if (days === 0) days = 7;
  d.setUTCDate(d.getUTCDate() + days);
  d.setUTCHours(20, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}
function slotForIndex(maxTs, idx) {
  const base = nextTuesday8pmUTC(maxTs);
  return base + Math.floor(idx / 2) * 7 * 24 * 3600;
}
function tsToLocalInput(ts) {
  // datetime-local wants local wall time as YYYY-MM-DDTHH:MM
  const d = new Date(ts * 1000);
  const pad = n => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
function localInputToTs(s) {
  return Math.floor(new Date(s).getTime() / 1000);
}

function refreshCount() {
  document.getElementById("queue-count").textContent = queue.length + " queued";
  document.getElementById("publish-btn").disabled = queue.length === 0;
}

function openForm(cardId) {
  const card = document.getElementById(cardId);
  const form = card.querySelector(".form");
  const actions = card.querySelector(".actions");
  const dt = form.querySelector(".f-schedule");
  dt.value = tsToLocalInput(slotForIndex(INITIAL_MAX_TS, queue.length));
  form.style.display = "flex";
  actions.style.display = "none";
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
    const r = await fetch("/ai", {method:"POST", headers:{"Content-Type":"application/json"},
                                   body: JSON.stringify({path, provider: AI_PROVIDER, model: AI_MODEL})});
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

function saveCard(cardId, path) {
  const card = document.getElementById(cardId);
  const title = card.querySelector(".f-title").value.trim();
  const desc = card.querySelector(".f-description").value.trim();
  const priceRaw = card.querySelector(".f-price").value.trim();
  const sched = card.querySelector(".f-schedule").value;
  const err = card.querySelector(".err");
  if (!title) { err.textContent = "title required"; return; }
  if (!desc) { err.textContent = "description required"; return; }
  if (!sched) { err.textContent = "schedule required"; return; }
  const entry = {cardId, path, title, description: desc,
                 scheduleTs: localInputToTs(sched)};
  if (priceRaw !== "") {
    const p = parseFloat(priceRaw);
    if (isNaN(p) || p < 0) { err.textContent = "price must be >= 0"; return; }
    entry.price = p;
  }
  queue.push(entry);
  card.classList.add("queued");
  card.querySelector(".form").style.display = "none";
  const actions = card.querySelector(".actions");
  actions.style.display = "flex";
  actions.querySelector(".btn-add").textContent = "Queued";
  actions.querySelector(".btn-add").disabled = true;
  const nameEl = card.querySelector(".name");
  if (!card.querySelector(".badge")) {
    const b = document.createElement("span");
    b.className = "badge"; b.textContent = "queued";
    nameEl.prepend(b, " ");
  }
  refreshCount();
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
</script>
</body></html>
"""


class GalleryHandler(BaseHTTPRequestHandler):
    thumb_dir = ""
    thumb_map: dict[str, str] = {}
    initial_max_ts = 0
    ai_provider = "claude"
    ai_model = "claude-opus-4-7"
    json_path = "publications.json"
    schema_path = "publicationsSchema.json"
    publish_done: dict | None = None

    def _build_page(self) -> str:
        cards = []
        for idx, (orig_path, thumb_path) in enumerate(self.thumb_map.items()):
            rel = os.path.relpath(thumb_path, self.thumb_dir)
            filename = os.path.basename(orig_path)
            safe_name = html.escape(filename, quote=True)
            safe_rel = html.escape(rel, quote=True)
            js_path = html.escape(json.dumps(orig_path), quote=True)
            cid = f"card_{idx}"
            cards.append(f"""<div class="card" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_name}">
  <div class="name">{safe_name}</div>
  <div class="actions">
    <button class="btn-add" onclick="openForm('{cid}')">Add</button>
    <button class="btn-del" onclick="delCard('{cid}', {js_path})">Delete</button>
  </div>
  <div class="form">
    <label>Title <input type="text" class="f-title" maxlength="50"></label>
    <label>Description <textarea class="f-description"></textarea></label>
    <label>Price (optional) <input type="number" class="f-price" min="0" step="0.01"></label>
    <label>Schedule <input type="datetime-local" class="f-schedule"></label>
    <button class="btn-ai" onclick="aiGen('{cid}', {js_path})">Generate with AI</button>
    <div class="err"></div>
    <div class="form-actions">
      <button class="btn-save" onclick="saveCard('{cid}', {js_path})">Save to queue</button>
      <button class="btn-cancel" onclick="closeForm('{cid}')">Cancel</button>
    </div>
  </div>
</div>""")
        page = _PAGE_TMPL
        page = page.replace("__CARDS__", "\n".join(cards))
        page = page.replace("__INITIAL_MAX_TS__", str(self.initial_max_ts))
        page = page.replace("__AI_PROVIDER__", self.ai_provider)
        page = page.replace("__AI_MODEL__", self.ai_model)
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
                provider = data.get("provider") or self.ai_provider
                model = data.get("model") or self.ai_model
                # Send the thumbnail to Claude, not the full-res original — same
                # visual info for a fraction of the tokens/latency.
                image_for_ai = self.thumb_map.get(path, path)
                try:
                    title, desc = generate_metadata(image_for_ai, provider, model)
                except (RuntimeError, NotImplementedError) as e:
                    self._send(400, {"error": str(e)}); return
                self._send(200, {"title": title, "description": desc})

            elif self.path == "/publish":
                data = self._json_body()
                entries = data.get("entries", []) or []
                if not entries:
                    self._send(400, {"error": "empty queue"}); return
                try:
                    new_uuids = write_publications(entries, self.json_path, self.schema_path)
                except Exception as e:
                    self._send(400, {"error": f"schema/write failed: {e}"}); return

                published, failed, err = publish_batch(entries, new_uuids, self.json_path)
                result = {"published": published, "failed": failed}
                if err:
                    result["error"] = err
                self.__class__.publish_done = result
                self._send(200, result)
            else:
                self.send_response(404); self.end_headers()
        except Exception as e:
            try:
                self._send(500, {"error": str(e)})
            except Exception:
                pass

    def log_message(self, format, *args):
        pass


# ---------------------------------------------------------------------------
# Publications.json write
# ---------------------------------------------------------------------------

def write_publications(entries: list[dict], json_path: str, schema_path: str) -> list[str]:
    """Append entries as state=unpublished, validate, atomic write. Returns their UUIDs in order."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        data = []

    max_ts = _max_existing_ts(json_path)

    new_uuids = []
    now = int(time.time())
    for i, e in enumerate(entries):
        path = e["path"]
        sha = compute_sha512(path)
        ts = int(e.get("scheduleTs") or compute_next_slot(max_ts, i))
        apparition = {
            "platformName": "deviantart",
            "urlElsePublicationName": e["title"],
            "apparitionTimestampIfDifferentThanSubmission": ts,
        }
        if "price" in e and e["price"] is not None:
            apparition["priceIfNotFree"] = float(e["price"])
        u = str(uuid.uuid4())
        new_uuids.append(u)
        data.append({
            "uuid": u,
            "state": STATE_UNPUBLISHED,
            "submissionTimestamp": now,
            "description": e.get("description", ""),
            "files": [{
                "basename": os.path.basename(path),
                "sha512sum": sha,
            }],
            "apparitions": [apparition],
        })

    from jsonschema import validate
    with open(schema_path, "r", encoding="utf-8") as s:
        schema = json.load(s)
    validate(instance=data, schema=schema)

    _atomic_write_json(json_path, data)
    return new_uuids


def _atomic_write_json(path: str, data) -> None:
    d = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(prefix=".pub-", dir=d)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=4)
        os.replace(tmp, path)
    except Exception:
        try: os.unlink(tmp)
        except OSError: pass
        raise


def mark_state(json_path: str, target_uuid: str, state: str) -> None:
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)
    for p in data:
        if p.get("uuid") == target_uuid:
            p["state"] = state
            break
    _atomic_write_json(json_path, data)


# ---------------------------------------------------------------------------
# Playwright Chromium: DA submission
# ---------------------------------------------------------------------------
# set_checkbox / type_tags / pick_schedule imported from publish_deviantart.py.


def _resolve_art(path: str) -> str:
    """Prefer the client-supplied path; fall back to find_art_path if it's webp."""
    p = Path(path)
    if p.suffix.lower() == ".webp":
        # ponytail: DA rejects webp; look for a sibling non-webp via existing helper.
        return str(find_art_path(p.name))
    return str(p.resolve())


def _da_submit_one(page, art_path: str, title: str, schedule_str: str) -> None:
    page.goto("https://www.deviantart.com", wait_until="domcontentloaded", timeout=45000)
    page.get_by_role("link", name="Submit").first.click()
    page.wait_for_load_state("domcontentloaded", timeout=30000)

    with page.expect_file_chooser(timeout=15000) as fc:
        page.get_by_text("Upload Your Art", exact=True).click()
    fc.value.set_files(art_path)

    page.get_by_label("Title", exact=False).first.fill(title)
    set_checkbox(page, "matureContent", True)
    set_checkbox(page, "isAiGenerated", True)
    type_tags(page)
    pick_schedule(page, schedule_str)
    set_checkbox(page, "matureContent", True)

    page.get_by_text("Schedule", exact=True).last.click()
    page.wait_for_timeout(5000)


def _wait_for_login(page, timeout_s: int = 120) -> bool:
    page.goto("https://www.deviantart.com", wait_until="domcontentloaded", timeout=45000)
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        if page.get_by_role("link", name="Submit").count() > 0:
            return True
        page.wait_for_timeout(2000)
        try:
            page.reload(wait_until="domcontentloaded", timeout=30000)
        except Exception:
            pass
    return False


def publish_batch(entries: list[dict], new_uuids: list[str],
                  json_path: str) -> tuple[int, int, str | None]:
    """Drive Chromium through DA submission for each entry. Returns (ok, failed, err)."""
    from playwright.sync_api import sync_playwright

    SESSION_DIR.mkdir(parents=True, exist_ok=True)
    has_session = any(SESSION_DIR.iterdir())
    headless = has_session
    if not has_session:
        print("Log in to DeviantArt in the browser window, then leave the tab open — "
              "publish will proceed once we detect you're signed in.", flush=True)

    published = 0
    failed = 0
    err: str | None = None

    with sync_playwright() as p:
        ctx = p.chromium.launch_persistent_context(str(SESSION_DIR), headless=headless)
        page = ctx.pages[0] if ctx.pages else ctx.new_page()

        if not has_session:
            if not _wait_for_login(page, timeout_s=120):
                ctx.close()
                return 0, len(entries), "login timeout — no Submit link after 120s"

        for entry, u in zip(entries, new_uuids):
            try:
                art = _resolve_art(entry["path"])
                schedule_str = format_schedule(int(entry["scheduleTs"]))
                _da_submit_one(page, art, entry["title"], schedule_str)
                mark_state(json_path, u, STATE_PUBLISHED)
                published += 1
                print(f"published: {entry['title']}", flush=True)
            except Exception as e:
                failed += 1
                err = f"{entry.get('title', entry.get('path'))}: {e}"
                print(f"AUTOMATION FAILED: {err}", file=sys.stderr, flush=True)
                break

        ctx.close()

    return published, failed, err


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def serve(thumb_dir: str, thumb_map: dict[str, str], args) -> dict | None:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.initial_max_ts = _max_existing_ts(args.json)
    GalleryHandler.ai_provider = args.ai_provider
    GalleryHandler.ai_model = args.ai_model
    GalleryHandler.json_path = args.json
    GalleryHandler.schema_path = args.schema
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
    parser.add_argument("--picked-dir", default="picked")
    parser.add_argument("--json", default="publications.json")
    parser.add_argument("--schema", default=str(PKG / "publicationsSchema.json"))
    parser.add_argument("--ai-provider", default="claude")
    parser.add_argument("--ai-model", default="claude-opus-4-7")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()

    print("Finding unpublished images...")
    candidates = find_candidates(args.picked_dir, args.json, args.n)
    if not candidates:
        print("No unpublished images found.")
        return 0

    print(f"Found {len(candidates)} candidates. Generating thumbnails...")
    with tempfile.TemporaryDirectory(prefix="publish-next-") as thumb_dir:
        thumb_map = generate_thumbnails(candidates, thumb_dir)
        result = serve(thumb_dir, thumb_map, args)

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
