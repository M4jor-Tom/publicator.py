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
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from da_publish import (
    STATE_UNPUBLISHED,
    _atomic_write_json,
    configure,
    load_pending_entries,
    publish_batch,
)
from llm_meta import DEFAULT_MODEL, generate_metadata

PKG = Path(__file__).resolve().parent  # code assets (schema) travel with the package
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
  .card .sched { font-size: 12px; color: #aaa; }
  .card.queued { opacity: 0.6; border: 1px solid #4caf50; }
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
const AI_MODEL = "__AI_MODEL__";
const PENDING = __PENDING__; // already-queued entries from publications.json

const queue = []; // [{cardId, path, title, description, price, scheduleTs, uuid?}]

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
                                   body: JSON.stringify({path, model: AI_MODEL})});
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

function unqueuePending(cardId) {
  const i = queue.findIndex(e => e.cardId === cardId);
  if (i >= 0) queue.splice(i, 1);
  const el = document.getElementById(cardId);
  if (el) el.remove();
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

// Pre-queue entries already sitting in publications.json (state=unpublished).
for (const p of PENDING) {
  queue.push({cardId: p.cardId, uuid: p.uuid, path: p.path,
              title: p.title, scheduleTs: p.scheduleTs});
}
refreshCount();
</script>
</body></html>
"""


class GalleryHandler(BaseHTTPRequestHandler):
    thumb_dir = ""
    thumb_map: dict[str, str] = {}
    candidate_paths: list[str] = []      # new picks from picked/, add-able
    pending: list[dict] = []             # already-queued entries from publications.json
    initial_max_ts = 0
    ai_model = DEFAULT_MODEL
    json_path = "publications.json"
    schema_path = "publicationsSchema.json"
    publish_done: dict | None = None

    def _build_page(self) -> str:
        cards = []
        pending_js = []
        # Pre-queued cards: entries already in publications.json (state=unpublished).
        for idx, e in enumerate(self.pending):
            thumb_path = self.thumb_map.get(e["path"], e["path"])
            rel = os.path.relpath(thumb_path, self.thumb_dir)
            safe_rel = html.escape(rel, quote=True)
            safe_title = html.escape(e["title"], quote=True)
            ts = e.get("scheduleTs")
            sched = (datetime.fromtimestamp(ts).strftime("%a %d %b %Y %H:%M")
                     if ts else "no schedule")
            cid = f"pending_{idx}"
            cards.append(f"""<div class="card queued" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_title}">
  <div class="name"><span class="badge">queued</span> {safe_title}</div>
  <div class="sched">{html.escape(sched)}</div>
  <div class="actions">
    <button class="btn-del" onclick="unqueuePending('{cid}')">Remove from queue</button>
  </div>
</div>""")
            pending_js.append({"cardId": cid, "uuid": e["uuid"], "path": e["path"],
                               "title": e["title"], "scheduleTs": ts})
        # Add-able cards: new picks from picked/.
        for idx, orig_path in enumerate(self.candidate_paths):
            thumb_path = self.thumb_map.get(orig_path, orig_path)
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
        page = page.replace("__PENDING__", json.dumps(pending_js))
        page = page.replace("__INITIAL_MAX_TS__", str(self.initial_max_ts))
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
                model = data.get("model") or self.ai_model
                # Send the thumbnail to Claude, not the full-res original — same
                # visual info for a fraction of the tokens/latency.
                image_for_ai = self.thumb_map.get(path, path)
                try:
                    title, desc = generate_metadata(image_for_ai, model)
                except RuntimeError as e:
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
                    new_uuids = (write_publications(new, self.json_path, self.schema_path)
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


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def serve(thumb_dir: str, thumb_map: dict[str, str], candidate_paths: list[str],
          pending: list[dict], args) -> dict | None:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.candidate_paths = candidate_paths
    GalleryHandler.pending = pending
    GalleryHandler.initial_max_ts = _max_existing_ts(args.json)
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
    parser.add_argument("--data-dir", default=None,
                        help="publication database dir (publications.json + images + browser session); default: CWD")
    parser.add_argument("--picked-dir", default=None, help="default: <data-dir>/picked")
    parser.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    parser.add_argument("--schema", default=str(PKG / "publicationsSchema.json"))
    parser.add_argument("--ai-model", default=DEFAULT_MODEL)
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()

    data_dir = configure(args.data_dir)  # points da_publish + echo helpers at the db dir
    args.json = args.json or str(data_dir / "publications.json")
    args.picked_dir = args.picked_dir or str(data_dir / "picked")

    print("Finding unpublished images...")
    candidates = find_candidates(args.picked_dir, args.json, args.n)
    pending = load_pending_entries(args.json)
    if not candidates and not pending:
        print("Nothing to publish (no new picks, no pending queue).")
        return 0

    msg = f"{len(candidates)} new pick(s)"
    if pending:
        msg += f", {len(pending)} already queued"
    print(f"Found {msg}. Generating thumbnails...")
    with tempfile.TemporaryDirectory(prefix="publish-next-") as thumb_dir:
        thumb_map = generate_thumbnails(candidates + [e["path"] for e in pending], thumb_dir)
        result = serve(thumb_dir, thumb_map, candidates, pending, args)

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
