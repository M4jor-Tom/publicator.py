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

import html
import json
import logging
import os
import subprocess
import urllib.parse
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from publicator.config import validate_publications
from publicator.deviantart import configure, load_pending_entries, publish_batch
from publicator.images import guess_mime
from publicator.llm_meta import DEFAULT_MODEL, generate_metadata
from publicator.scheduling import existing_ts, schedule_data, ts_labels, zone
from publicator.store import apply_update, atomic_write_json, write_publications

log = logging.getLogger("publicator.gallery")

# Runtime state (publications.json, images, browser session) resolves against the
# publication database dir (--data-dir, default CWD). da_publish.configure() owns
# the DATA_DIR/session paths; main() calls it once args are parsed.


# ---------------------------------------------------------------------------
# HTTP: gallery + queue endpoints
# ---------------------------------------------------------------------------

PAGE_TEMPLATE = r"""<!DOCTYPE html>
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

// >>> scheduler core (sliced verbatim by tests/test_scheduling.py) >>>
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
  return slot ?? p.slots[p.slots.length - 1];  // ponytail: horizon full -> reuse last; widen SLOT_HORIZON_WEEKS
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
  // scheduleTs drives local occupancy/display; `schedule` (the naive wall clock)
  // is what the server persists, resolved in the schedule TZ — so a custom time
  // typed in a UTC-spoofed private window still lands on the intended Paris hour.
  const out = {title, description: desc, schedule: sched, scheduleTs: localInputToTs(sched)};
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


def tier_gallery_fields(config):
    if not config.get("tiers") and not config.get("galleries"):
        return ""
    tiers = "".join(f'<option value="{html.escape(t, quote=True)}">{html.escape(t)}</option>'
                    for t in config.get("tiers", []))
    gals = "".join(f'<label><input type="checkbox" class="f-gallery" '
                   f'value="{html.escape(g, quote=True)}"> {html.escape(g)}</label>'
                   for g in config.get("galleries", []))
    return (f'<label>Tier <select class="f-tier"><option value="">(none)</option>{tiers}</select></label>'
            f'<div class="galleries"><span>Galleries</span>{gals}</div>')


def preset_options(schedules):
    opts = "".join(f'<option value="{html.escape(s["name"], quote=True)}">'
                   f'{html.escape(s["name"])}</option>' for s in schedules)
    return opts + '<option value="__custom__">(custom)</option>'


def card_form_html(cid, js_path, tg, save_label, preset_opts):
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
    schedules: list = schedule_data({})
    publish_done: dict | None = None

    def _build_page(self) -> str:
        cards = []
        pending_js = []
        tg = tier_gallery_fields(self.config)
        preset_opts = preset_options(self.schedules)
        tz = zone(self.config.get("schedule", {}))
        # Pre-queued cards: entries already in publications.json (state=unpublished).
        for idx, e in enumerate(self.pending):
            thumb_path = self.thumb_map.get(e["path"], e["path"])
            rel = os.path.relpath(thumb_path, self.thumb_dir)
            safe_rel = html.escape(rel, quote=True)
            safe_title = html.escape(e["title"], quote=True)
            view_href = html.escape("/original?path=" + urllib.parse.quote(e["path"]), quote=True)
            ts = e.get("scheduleTs")
            sched = (datetime.fromtimestamp(ts, tz).strftime("%a %d %b %Y %H:%M")
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
{card_form_html(cid, js_pathp, tg, "Save changes", preset_opts)}
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
{card_form_html(cid, js_path, tg, "Save to queue", preset_opts)}
</div>""")
        # Labels for every instant that can land in a datetime-local input: slot
        # presets and pending scheduleTs (background occupancy is never rendered).
        # Rendered in the schedule TZ so the picker shows the right wall clock even
        # when the private window forces Date to UTC.
        slot_ts = [ts for p in self.schedules for ts in p.get("slots", [])]
        pending_ts = [e["scheduleTs"] for e in pending_js if e.get("scheduleTs")]
        labels = ts_labels(tz, slot_ts + pending_ts)
        page = PAGE_TEMPLATE
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
                self.send_header("Content-type", guess_mime(full_path))
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
                self.send_header("Content-type", guess_mime(req_path))
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
                    apply_update(pubs, u, data, zone(self.config.get("schedule", {})))
                except KeyError:
                    self._send(404, {"error": f"uuid not found: {u}"}); return
                try:
                    validate_publications(pubs, self.config)
                except Exception as e:
                    self._send(400, {"error": f"invalid: {e}"}); return
                atomic_write_json(self.json_path, pubs)
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
# Main
# ---------------------------------------------------------------------------

def serve(thumb_dir: str, thumb_map: dict[str, str], candidate_paths: list[str],
          pending: list[dict], args, config: dict) -> dict | None:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.candidate_paths = candidate_paths
    GalleryHandler.pending = pending
    GalleryHandler.existing_ts = existing_ts(args.json)
    GalleryHandler.ai_model = args.ai_model
    GalleryHandler.openrouter_model = args.openrouter_model
    GalleryHandler.ai_timeout = args.ai_timeout
    GalleryHandler.json_path = args.json
    GalleryHandler.config = config          # was: load_config(Path.cwd())
    GalleryHandler.schedules = schedule_data(config["schedule"])
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
