"""The gallery page: HTML template + a pure render function.

No handler, no socket, no I/O beyond the paths it is handed — which is what
makes render_page testable without spinning up an HTTP server.
"""

import html
import json
import os
import urllib.parse
from datetime import datetime

from publicator.scheduling import ts_labels, zone

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


def render_page(*, thumb_dir, thumb_map, candidates, pending, existing_ts,
                schedules, config, ai_model, openrouter_model) -> str:
    """The whole gallery page as HTML. Pure: no handler, no socket, no I/O
    beyond the paths it is handed — which is what makes it testable."""
    cards = []
    pending_js = []
    tg = tier_gallery_fields(config)
    preset_opts = preset_options(schedules)
    tz = zone(config.get("schedule", {}))
    # Pre-queued cards: entries already in publications.json (state=unpublished).
    for idx, e in enumerate(pending):
        thumb_path = thumb_map.get(e["path"], e["path"])
        rel = os.path.relpath(thumb_path, thumb_dir)
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
    for idx, orig_path in enumerate(candidates):
        thumb_path = thumb_map.get(orig_path, orig_path)
        rel = os.path.relpath(thumb_path, thumb_dir)
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
    slot_ts = [ts for p in schedules for ts in p.get("slots", [])]
    pending_ts = [e["scheduleTs"] for e in pending_js if e.get("scheduleTs")]
    labels = ts_labels(tz, slot_ts + pending_ts)
    page = PAGE_TEMPLATE
    page = page.replace("__CARDS__", "\n".join(cards))
    page = page.replace("__PENDING__", json.dumps(pending_js))
    page = page.replace("__EXISTING_TS__", json.dumps(existing_ts))
    page = page.replace("__SCHEDULES__", json.dumps(schedules))
    page = page.replace("__LABELS__", json.dumps(labels))
    page = page.replace("__AI_MODEL__", html.escape(ai_model, quote=True))
    page = page.replace("__OPENROUTER_MODEL__", html.escape(openrouter_model, quote=True))
    return page
