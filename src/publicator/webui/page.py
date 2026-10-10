"""The gallery page: HTML template + a pure render function.

No handler, no socket, no I/O beyond the paths it is handed — which is what
makes render_page testable without spinning up an HTTP server.
"""

import html
import json
import os
import urllib.parse
from datetime import datetime

from publicator.llm_meta import TODO_GENERATE
from publicator.scheduling import ts_labels, zone
from publicator.webui.calendar_view import render_calendar

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
  .form input, .form textarea, .promptsearch input { background: #1a1a1a; color: #eee; border: 1px solid #444;
                                border-radius: 4px; padding: 6px; font-family: inherit; font-size: 13px; }
  .form fieldset { border: 0; margin: 0; padding: 0; }
  .form legend { padding: 0; font-size: 12px; color: #aaa; }
  .form fieldset label { display: inline-flex; align-items: center; gap: 4px; margin-right: 8px; }
  .form input[type=checkbox] { padding: 0; }
  .form textarea { min-height: 70px; resize: vertical; }
  .form-actions { display: flex; gap: 6px; margin-top: 4px; }
  .form-actions button, .ai-actions button { flex: 1; padding: 8px; border: none; border-radius: 4px;
                                               cursor: pointer; font-size: 13px; }
  .ai-actions { display: flex; gap: 6px; }
  .btn-ai { background: #7b3ec1; color: white; }
  .btn-ai-later { background: #66522f; color: white; }
  .ai-pending { color: #e0a800; font-size: 12px; }
  .btn-save { background: #4caf50; color: white; }
  .btn-cancel { background: #555; color: white; }
  .err { color: #ff8080; font-size: 12px; min-height: 14px; }
  .tab { padding: 8px 16px; background: #222; color: #aaa; border: 1px solid #333;
         border-radius: 6px; cursor: pointer; font-size: 14px; }
  .tab.on { background: #333; color: #fff; }
  .month { scroll-margin-top: 70px; }   /* clear the sticky header when scrolled to */
  .month h3 { margin: 24px 0 8px; font-size: 16px; color: #ccc; }
  .month.now h3 { color: #4caf50; }
  table.cal { border-collapse: collapse; width: 100%; max-width: 1100px; }
  table.cal th { font-size: 12px; color: #888; font-weight: normal; padding: 4px; }
  .day { border: 1px solid #333; vertical-align: top; height: 78px; width: 14.28%;
         padding: 2px; background: #222; }
  .day.other { background: #1a1a1a; border-color: #262626; }
  .day.today { border: 2px solid #4caf50; }
  .day .num { font-size: 11px; color: #777; display: block; }
  .ev { display: inline-block; margin: 1px; line-height: 0; font-size: 11px; color: #ccc; }
  .ev img { width: 46px; height: 46px; object-fit: cover; border-radius: 3px; }
  .ev.past { opacity: 0.55; }
  .ev.upcoming img { outline: 2px solid #2196f3; }
  /* A thumbnail whose prompt is known is a popover trigger, not a link; strip
     the UA button chrome so it stays pixel-identical to its <a> siblings.
     Not `font: inherit` — that shorthand would also reset the 11px .ev sets
     for the no-thumbnail (title-only) case. */
  button.ev { background: none; border: 0; padding: 0; cursor: pointer;
              font-family: inherit; }
  .evpop { background: #2a2a2a; color: #eee; border: 1px solid #444; border-radius: 8px;
           padding: 12px; max-width: min(38rem, 92vw); max-height: 85vh; overflow: auto; }
  .evpop::backdrop { background: rgba(0, 0, 0, .55); }
  .evpop .evtitle { font-size: 15px; margin-bottom: .3rem; word-break: break-word; }
  .evpop > a { color: #8ac; font-size: 13px; }   /* ~7.18:1 on #2a2a2a */
  .prompt { margin: .4rem 0; font-size: .85rem; text-align: left; }
  .prompt > summary { cursor: pointer; padding: .2rem .4rem; border-radius: 3px; }
  .prompt.exact > summary { background: #eef4ee; color: #2c4a2c; }
  .prompt.nearest > summary { background: #fff3cd; color: #7a5b00; font-weight: 600; }
  .prompt.nearest { border-left: 3px solid #e0a800; padding-left: .4rem; }
  .prompt .warn { color: #e0a800; margin: .3rem 0; }
  .prompt pre { white-space: pre-wrap; word-break: break-word; background: #f7f7f7;
                color: #222; padding: .4rem; border-radius: 3px; max-height: 20rem;
                overflow: auto; }
  .prompt ul { list-style: none; padding-left: 0; }
  .prompt .lin { font-family: monospace; }
  .prompt .n { color: #999; font-size: .8rem; }
  /* Lineage links render on two different backdrops: inside <summary> for
     Exact (pale #eef4ee) and inside a plain <li> for Nearest (the card's dark
     #2a2a2a). No single colour clears WCAG AA 4.5:1 on both, so two rules:
     #1a4d80 is 7.78:1 on #eef4ee, #8ac is 5.92:1 on #2a2a2a. */
  .prompt.exact a { color: #1a4d80; }
  .prompt.nearest a { color: #8ac; }
  .groupsplit { grid-column: 1 / -1; margin: 1rem 0 .3rem; padding: .4rem;
                background: #fff3cd; color: #7a5b00; border-left: 3px solid #e0a800;
                font-size: .9rem; }
  /* .grid is display:grid, and __PROMPTSEARCH__ expands to two siblings
     (form + banner) as the first children - without this they'd each get
     pinned to one 280px card-sized track instead of spanning the row. */
  .promptsearch, .skipped { grid-column: 1 / -1; }
  .promptsearch { margin: .5rem 0; display: flex; gap: .4rem; align-items: center; }
  .promptsearch input { flex: 1; max-width: 30rem; padding: .3rem; }
  .promptsearch a { color: #8ac; }   /* UA default link colour is ~1.85:1 on #1a1a1a; this is ~7.18:1 */
  /* #7a5b00 (the .prompt.nearest summary text) only works on its own light
     #fff3cd chip; on the page's #1a1a1a background it's ~2.8:1. #e0a800 is the
     same warning hue already used elsewhere in this file directly on the dark
     background (.prompt .warn) and clears WCAG AA there. */
  .skipped { color: #e0a800; font-size: .85rem; margin: .2rem 0 .6rem; }
</style>
</head><body>
<header>
  <h1>Publish next</h1>
  <button id="tab-gallery" class="tab on" onclick="showTab('gallery')">Gallery</button>
  <button id="tab-calendar" class="tab" onclick="showTab('calendar')">Calendar</button>
  <span id="queue-count">0 queued</span>
  <select id="ai-model" title="AI model for Generate with AI">__MODEL_OPTIONS__</select>
  <span id="stage-status"></span>
  <button id="stage-btn" onclick="stageQueue()" disabled>Add to publish pad</button>
  <button id="publish-btn" onclick="publishQueue()" disabled>Publish</button>
</header>
<main>
  <div id="gallery-tab" class="grid">__PROMPTSEARCH__
__CARDS__</div>
  <div id="calendar-tab" hidden>__CALENDAR__</div>
</main>
<script>
const EXISTING_TS = __EXISTING_TS__;   // already-scheduled background instants (occupancy)
const SCHEDULES = __SCHEDULES__;       // [{name, per_slot, default_tiers, slots:[ts,...]}]  slots: upcoming instants
const LABELS = __LABELS__;             // {ts: "YYYY-MM-DDTHH:MM"} wall clock in the schedule TZ, browser-TZ-proof
const PENDING = __PENDING__;           // already-queued entries from publications.json
const TODO_GENERATE = __TODO_GENERATE__;

const queue = []; // [{cardId, path, title, description, price, scheduleTs, tiers, galleries, uuid?}]
const aiFailures = new Map(); // cardId -> hosted providers that failed this session
function isAiPending(e) { return e && (e.title === TODO_GENERATE || e.description === TODO_GENERATE); }
// >>> AI provider fallback core
const HOSTED_AI_SOURCES = ["claude", "openrouter"];
function aiFailureCanDefer(failed, source, canDefer, pending) {
  if (!canDefer || !HOSTED_AI_SOURCES.includes(source)) return false;
  failed.add(source);
  return !pending && HOSTED_AI_SOURCES.every(s => failed.has(s));
}
// <<< AI provider fallback core
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
function setTierChecks(form, tiers) {
  const selected = new Set(tiers || []);
  form.querySelectorAll(".f-tier").forEach(cb => { cb.checked = selected.has(cb.value); });
}
function applyPreset(sel, cardId) {      // preset change -> refill the picker and tier defaults
  if (sel.value === "__custom__") return;
  const card = document.getElementById(cardId);
  const profile = profileByName(sel.value);
  card.querySelector(".f-schedule").value = tsToLocalInput(nextSlotForProfile(profile));
  setTierChecks(card.querySelector(".form"), profile.default_tiers);
}

function showTab(name) {                 // two tabs, one page: no routing, no reload
  for (const t of ["gallery", "calendar"]) {
    document.getElementById(t + "-tab").hidden = (t !== name);
    document.getElementById("tab-" + t).classList.toggle("on", t === name);
  }
  if (name === "calendar") document.querySelector(".month.now")?.scrollIntoView({block: "start"});
}
// A scheduled entry in the calendar links to its gallery card (#pending_N) —
// reveal the gallery first, or the anchor would jump inside a hidden tab.
document.getElementById("calendar-tab").addEventListener("click", e => {
  const a = e.target.closest('a[href^="#"]');
  if (!a) return;
  // The link may live inside an event's popover (top layer, but still a DOM
  // descendant of this tab, which is why the click reaches here at all).
  a.closest("[popover]")?.hidePopover();
  showTab("gallery");
  document.getElementById(a.getAttribute("href").slice(1))?.scrollIntoView({block: "center"});
});

function refreshCount() {
  const onPad = queue.filter(e => e.uuid).length; // entries persisted to publications.json
  const needsAi = queue.filter(isAiPending).length;
  document.getElementById("queue-count").textContent = queue.length + " queued";
  document.getElementById("publish-btn").disabled = queue.length === 0 || needsAi > 0;
  document.getElementById("stage-btn").disabled = queue.length === onPad; // nothing fresh to stage
  document.getElementById("stage-status").textContent = [
    onPad ? onPad + " on pad" : "", needsAi ? needsAi + " need AI metadata" : ""
  ].filter(Boolean).join(" · ");
}

function openForm(cardId) {
  const card = document.getElementById(cardId);
  const form = card.querySelector(".form");
  const entry = queue.find(e => e.cardId === cardId);
  const q = sel => form.querySelector(sel);
  const presetEl = q(".f-preset");
  if (entry) {
    q(".f-title").value = entry.title === TODO_GENERATE ? "" : (entry.title || "");
    q(".f-description").value = entry.description === TODO_GENERATE ? "" : (entry.description || "");
    q(".f-price").value = (entry.price == null) ? "" : entry.price;
    q(".f-schedule").value = entry.scheduleTs ? tsToLocalInput(entry.scheduleTs) : "";
    if (presetEl) presetEl.value = presetForTs(entry.scheduleTs);
    setTierChecks(form, entry.tiers);
    form.querySelectorAll(".f-gallery").forEach(cb => cb.checked = (entry.galleries || []).includes(cb.value));
  } else {
    const p = SCHEDULES[0];
    if (presetEl) presetEl.value = p.name;
    q(".f-schedule").value = tsToLocalInput(nextSlotForProfile(p));
    setTierChecks(form, p.default_tiers);
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
  const later = card.querySelector(".btn-ai-later");
  err.textContent = ""; later.hidden = true;
  btn.disabled = true; btn.textContent = "Generating...";
  try {
    const modelOption = document.getElementById("ai-model").selectedOptions[0];
    const source = modelOption.dataset.source;
    const model = modelOption.dataset.model ? JSON.parse(modelOption.dataset.model) : null;
    if (source === "local" && modelOption.dataset.installed === "false") {
      btn.textContent = `Installing ${modelOption.dataset.ollama}… then generating`;
    }
    const r = await fetch("/ai", {method:"POST", headers:{"Content-Type":"application/json"},
                                   body: JSON.stringify({path, source, model})});
    const d = await r.json();
    if (!r.ok || d.error) {
      err.textContent = d.error || ("HTTP " + r.status);
      const failed = aiFailures.get(cardId) || new Set();
      later.hidden = !aiFailureCanDefer(
        failed, source, d.can_defer === true,
        isAiPending(queue.find(e => e.cardId === cardId)));
      if (d.can_defer === true) aiFailures.set(cardId, failed);
      return;
    }
    aiFailures.delete(cardId);
    card.querySelector(".f-title").value = d.title || "";
    card.querySelector(".f-description").value = d.description || "";
  } catch (e) {
    err.textContent = String(e);
  } finally {
    btn.disabled = false; btn.textContent = "Generate with AI";
  }
}

async function markAiLater(cardId, path) {
  const card = document.getElementById(cardId);
  const title = card.querySelector(".f-title");
  const description = card.querySelector(".f-description");
  if ((title.value.trim() || description.value.trim()) &&
      (title.value !== TODO_GENERATE || description.value !== TODO_GENERATE) &&
      !confirm("AI generation later will replace the current title and description. Continue?")) return;
  title.value = description.value = TODO_GENERATE;
  if (!await saveCard(cardId, path)) return;

  const entry = queue.find(e => e.cardId === cardId);
  if (!entry.uuid) {
    const {cardId: _cardId, ...data} = entry;
    try {
      const r = await fetch("/stage", {method:"POST", headers:{"Content-Type":"application/json"},
                                        body: JSON.stringify({entries:[data]})});
      const d = await r.json();
      if (!r.ok || d.error) throw new Error(d.error || ("HTTP " + r.status));
      entry.uuid = d.uuids[0];
      card.querySelector(".btn-del")?.remove();
    } catch (e) {
      openForm(cardId);
      card.querySelector(".err").textContent = "Could not save AI marker: " + String(e);
      return;
    }
  }
  card.querySelector(".btn-ai-later").hidden = true;
  card.querySelector(".err").textContent = "";
  refreshCount();
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
  const tiers = [...card.querySelectorAll(".f-tier:checked")].map(cb => cb.value);
  if (tiers.length) out.tiers = tiers;
  const gals = [...card.querySelectorAll(".f-gallery:checked")].map(cb => cb.value);
  if (gals.length) out.galleries = gals;
  return out;
}

async function saveCard(cardId, path) {
  const card = document.getElementById(cardId);
  const fields = readForm(card);
  if (!fields) return false;
  const entry = queue.find(e => e.cardId === cardId);
  if (entry) {                       // re-edit
    Object.assign(entry, {price: undefined, tiers: undefined, galleries: undefined}, fields);
    if (entry.uuid) {
      const btn = card.querySelector(".btn-save");
      btn.disabled = true; btn.textContent = "Saving...";
      const r = await fetch("/update", {method:"POST", headers:{"Content-Type":"application/json"},
                                        body: JSON.stringify({uuid: entry.uuid, ...fields})});
      const d = await r.json();
      btn.disabled = false; btn.textContent = "Save changes";
      if (!r.ok || d.error) { card.querySelector(".err").textContent = d.error || ("HTTP " + r.status); return false; }
    }
    card.querySelector(".name").lastChild.textContent = " " + (isAiPending(entry) ? entry.path.split("/").pop() : fields.title);
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
  card.querySelector(".ai-pending").hidden = !isAiPending(queue.find(e => e.cardId === cardId));
  card.querySelector(".form").style.display = "none";
  card.querySelector(".actions").style.display = "flex";
  refreshCount();
  return true;
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
  refreshCount(); // recomputes queue/pad/AI counts and disables stage-btn when nothing is fresh
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
              tiers: p.tiers, galleries: p.galleries});
}
refreshCount();
</script>
</body></html>
"""


def tier_gallery_fields(config):
    if not config.get("tiers") and not config.get("galleries"):
        return ""
    tiers = "".join(f'<label><input type="checkbox" class="f-tier" '
                    f'value="{html.escape(t, quote=True)}"> {html.escape(t)}</label>'
                    for t in config.get("tiers", []))
    gals = "".join(f'<label><input type="checkbox" class="f-gallery" '
                   f'value="{html.escape(g, quote=True)}"> {html.escape(g)}</label>'
                   for g in config.get("galleries", []))
    tier_field = f'<fieldset class="tiers"><legend>Tiers</legend>{tiers}</fieldset>' if tiers else ""
    return (tier_field + f'<div class="galleries"><span>Galleries</span>{gals}</div>')


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
    <div class="ai-actions">
      <button class="btn-ai" onclick="aiGen('{cid}', {js_path})">Generate with AI</button>
      <button class="btn-ai-later" hidden onclick="markAiLater('{cid}', {js_path})">Mark for later</button>
    </div>
    <div class="err"></div>
    <div class="form-actions">
      <button class="btn-save" onclick="saveCard('{cid}', {js_path})">{save_label}</button>
      <button class="btn-cancel" onclick="closeForm('{cid}')">Cancel</button>
    </div>
  </div>"""


def render_page(*, thumb_map, candidates, pending, existing_ts, timeline,
                schedules, config, ai_model, openrouter_model,
                prompt_html=None, query="", skipped=0,
                maybe_candidates=None, search_enabled=True, truncated=False,
                lineage_active=False, vision_models=None,
                local_model_statuses=None) -> str:
    """The whole page as HTML — gallery tab + calendar tab. Pure: no handler, no
    socket, no I/O beyond the paths it is handed, which is what makes it testable.
    `thumb_map` is {art path: cached thumbnail name} (see images.thumb_name).
    `prompt_html` is {art path: prompt HTML block} — pre-rendered by the server
    so this module stays unaware of prompts. `search_enabled` defaults True so
    existing callers keep working; the server passes False when there is no
    prompt archive, so a data dir that never opted into the feature renders no
    search box at all. `lineage_active` labels the exact-results group — it
    only makes sense once a lineage filter narrowed the page, so the normal
    (unfiltered) gallery view is unchanged."""
    cards = []
    pending_js = []
    tg = tier_gallery_fields(config)
    prompt_html = prompt_html or {}
    preset_opts = preset_options(schedules)
    tz = zone(config.get("schedule", {}))
    # Pre-queued cards: entries already in publications.json (state=unpublished).
    for idx, e in enumerate(pending):
        safe_rel = html.escape(thumb_map.get(e["path"], ""), quote=True)
        ai_pending = e.get("title") == TODO_GENERATE or e.get("description") == TODO_GENERATE
        safe_title = html.escape(os.path.basename(e["path"]) if ai_pending else e["title"], quote=True)
        view_href = html.escape("/original?path=" + urllib.parse.quote(e["path"]), quote=True)
        ts = e.get("scheduleTs")
        sched = (datetime.fromtimestamp(ts, tz).strftime("%a %d %b %Y %H:%M")
                 if ts else "no schedule")
        cid = f"pending_{idx}"
        js_pathp = html.escape(json.dumps(e["path"]), quote=True)
        cards.append(f"""<div class="card queued" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_title}">
  <div class="name"><span class="badge">queued</span> {safe_title}</div>
  <div class="ai-pending"{"" if ai_pending else " hidden"}>AI metadata pending</div>
  <div class="sched">{html.escape(sched)}</div>
  <div class="actions">
    <a class="btn-view" href="{view_href}" target="_blank" rel="noopener">View</a>
    <button class="btn-add" onclick="openForm('{cid}')">Edit</button>
  </div>
{prompt_html.get(e["path"], "")}
{card_form_html(cid, js_pathp, tg, "Save changes", preset_opts)}
</div>""")
        pending_js.append({"cardId": cid, "uuid": e["uuid"], "path": e["path"],
                           "title": e["title"], "description": e.get("description"),
                           "scheduleTs": ts, "price": e.get("price"),
                           "tiers": e.get("tiers", []),
                           "galleries": e.get("galleries", [])})
    # Add-able cards: new picks from picked/.
    def candidate_card(idx, orig_path):
        filename = os.path.basename(orig_path)
        safe_name = html.escape(filename, quote=True)
        safe_rel = html.escape(thumb_map.get(orig_path, ""), quote=True)
        js_path = html.escape(json.dumps(orig_path), quote=True)
        view_href = html.escape("/original?path=" + urllib.parse.quote(orig_path),
                                quote=True)
        cid = f"card_{idx}"
        return f"""<div class="card" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_name}">
  <div class="name">{safe_name}</div>
  <div class="ai-pending" hidden>AI metadata pending</div>
  <div class="actions">
    <a class="btn-view" href="{view_href}" target="_blank" rel="noopener">View</a>
    <button class="btn-add" onclick="openForm('{cid}')">Add</button>
    <button class="btn-del" onclick="delCard('{cid}', {js_path})">Delete</button>
  </div>
{prompt_html.get(orig_path, "")}
{card_form_html(cid, js_path, tg, "Save to queue", preset_opts)}
</div>"""

    # Spec §6 names both lists; only label this one when a lineage filter is
    # active AND it actually has results — matching the "possibly from this
    # prompt" header below, which is gated the same way. An empty labelled
    # section with no text explaining why it's empty reads as a layout
    # glitch, not a deliberate "zero exact matches" signal.
    if lineage_active and candidates:
        cards.append('<div class="groupsplit">images from this prompt</div>')
    for idx, orig_path in enumerate(candidates):
        cards.append(candidate_card(idx, orig_path))
    # Two lists, never merged: images whose digest IS a version of this lineage,
    # and images that only share its basename because their own prompt was never
    # archived. Merging them would re-conflate exactly what the type separates.
    if maybe_candidates:
        cards.append('<div class="groupsplit">possibly from this prompt — '
                     "their own prompt was never archived, so this is a "
                     "filename hint only</div>")
        for idx, orig_path in enumerate(maybe_candidates, start=len(candidates)):
            cards.append(candidate_card(idx, orig_path))
    # Labels for every instant that can land in a datetime-local input: slot
    # presets and pending scheduleTs (background occupancy is never rendered).
    # Rendered in the schedule TZ so the picker shows the right wall clock even
    # when the private window forces Date to UTC.
    slot_ts = [ts for p in schedules for ts in p.get("slots", [])]
    pending_ts = [e["scheduleTs"] for e in pending_js if e.get("scheduleTs")]
    labels = ts_labels(tz, slot_ts + pending_ts)
    page = PAGE_TEMPLATE
    # anchors: uuid -> card id, how a scheduled entry links back to its card.
    anchors = {e["uuid"]: e["cardId"] for e in pending_js}
    page = page.replace("__CALENDAR__", render_calendar(timeline, tz=tz, anchors=anchors,
                                                        prompts=prompt_html))
    page = page.replace("__PENDING__", json.dumps(pending_js))
    page = page.replace("__TODO_GENERATE__", json.dumps(TODO_GENERATE))
    page = page.replace("__EXISTING_TS__", json.dumps(existing_ts))
    page = page.replace("__SCHEDULES__", json.dumps(schedules))
    page = page.replace("__LABELS__", json.dumps(labels))
    if vision_models:
        options = []
        router_model = next((m for m in vision_models
                             if m["openrouter_name"] == openrouter_model), vision_models[0])
        posted = html.escape(json.dumps(router_model), quote=True)
        options.append(f'<option data-source="openrouter" data-model="{posted}" '
                       f'value="openrouter:{html.escape(router_model["openrouter_name"], quote=True)}" selected>'
                       'OpenRouter (free)</option>')
        for model in vision_models:
            posted = html.escape(json.dumps(model), quote=True)
            status = (local_model_statuses or {}).get(model["ollama_name"], {})
            reason = html.escape(str(status.get("reason", "")), quote=True)
            disabled = not status.get("fit") or not status.get("available")
            label = f'Local — {model["ollama_name"]}'
            if disabled:
                label += f' ({html.escape(str(status.get("reason") or "Unavailable"))})'
            elif not status.get("installed"):
                label += " (Install on use)"
            attrs = (f' disabled title="{reason}"' if disabled else "")
            options.append(f'<option data-source="local" data-model="{posted}" '
                           f'data-ollama="{html.escape(model["ollama_name"], quote=True)}" '
                           f'data-installed="{str(bool(status.get("installed"))).lower()}"{attrs}>'
                           f'{label}</option>')
        options.append(f'<option data-source="claude" value="claude">'
                       f'Claude (subscription) — {html.escape(ai_model)}</option>')
        model_options = "".join(options)
    else:
        model_options = (f'<option data-source="openrouter" value="{html.escape(openrouter_model, quote=True)}">'
                        f'OpenRouter (free)</option><option data-source="claude" '
                        f'value="{html.escape(ai_model, quote=True)}">Claude (subscription)</option>')
    page = page.replace("__MODEL_OPTIONS__", model_options)
    # Server-side search: a plain GET form, no JS — the same reason scheduling
    # is server-side. Unarchived prompts are excluded from matching and the
    # count is stated, so the 86% gap stays visible instead of quietly
    # shrinking the result set. `skipped` counts Nearest only (never Unknown,
    # which is mostly non-prompt-bearing files - counting those would produce
    # a huge number that buries the real gap), so the wording says "skipped",
    # not "not searched": it names what the number actually measures.
    banner = (f'<p class="skipped">{skipped} candidate'
              f'{"" if skipped == 1 else "s"} skipped — their prompt was never archived.</p>'
              if skipped else "")
    # A broad needle can match far more than SEARCH_LIMIT files; silently
    # showing only the first batch would contradict the same
    # gaps-stay-visible principle the skipped banner exists for.
    if truncated:
        banner += ('<p class="skipped">more matches exist than are shown — '
                   "narrow your search to see the rest.</p>")
    # search_enabled is False when the data dir has no [prompts] section — the
    # feature must be inert then, not just prompt-less: rendering the form
    # anyway would let a submit run search() (archive is None -> no results)
    # and empty the gallery with no explanation.
    search = (f'<form class="promptsearch" method="get" action="/">'
              f'<input type="search" name="prompt" placeholder="search prompt text"'
              f' value="{html.escape(query, quote=True)}">'
              f'<button type="submit">Search</button>'
              f'<a href="/">clear</a></form>{banner}') if search_enabled else ""
    page = page.replace("__PROMPTSEARCH__", search)
    # __CARDS__ is replaced LAST: a card can embed arbitrary prompt-file text
    # (prompt_html), and every other placeholder is a literal `__NAME__`
    # token that html.escape does not touch — replacing them first would let
    # a prompt whose text happens to contain e.g. "__PROMPTSEARCH__" get that
    # placeholder's markup spliced into its own <pre>.
    page = page.replace("__CARDS__", "\n".join(cards))
    return page
