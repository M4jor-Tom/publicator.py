"""Pure server-rendered four-phase workflow; no client schedule allocation."""
import html
import json
import os
import urllib.parse
from datetime import datetime

from publicator.entries import effective_schedule_ts, has_delivery_success
from publicator.images import Search
from publicator.llm_meta import TODO_GENERATE
from publicator.scheduling import zone
from publicator.webui.calendar_view import render_calendar

PAGE_TEMPLATE = r"""<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Publish next</title><style>
body {font-family:sans-serif;background:#1a1a1a;color:#eee;margin:0}
header {position:sticky;top:0;background:#111;padding:12px;z-index:10;display:flex;flex-wrap:wrap;gap:8px}
main {padding:20px} button,input,textarea,select {font:inherit}
button {cursor:pointer;padding:6px} button:disabled {cursor:default}
a {color:#8ac} .grid {display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:16px}
.card {background:#2a2a2a;padding:12px;border-radius:8px;display:flex;flex-direction:column;gap:8px}
.card img {max-width:100%;max-height:240px;object-fit:contain}
label {display:block} textarea {display:block;width:95%;min-height:90px}
input[type=text] {width:95%} .status,.warning {color:#e0a800} .error {color:#ff9090}
[hidden] {display:none!important} table.platforms {border-collapse:collapse;width:100%}
.platforms th,.platforms td {border:1px solid #555;padding:12px;text-align:left;vertical-align:top}
.platforms img {width:90px;max-height:90px;object-fit:contain} fieldset {margin:8px 0}
.tab.on {outline:2px solid #8ac} .name {overflow-wrap:anywhere}
  .month { scroll-margin-top: 70px; }
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

  button.ev { background: none; border: 0; padding: 0; cursor: pointer;
              font-family: inherit; }
  .evpop { background: #2a2a2a; color: #eee; border: 1px solid #444; border-radius: 8px;
           padding: 12px; max-width: min(38rem, 92vw); max-height: 85vh; overflow: auto; }
  .evpop::backdrop { background: rgba(0, 0, 0, .55); }
  .evpop .evtitle { font-size: 15px; margin-bottom: .3rem; word-break: break-word; }
  .evpop > a { color: #8ac; font-size: 13px; }
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

  .prompt.exact a { color: #1a4d80; }
  .prompt.nearest a { color: #8ac; }
  .groupsplit { grid-column: 1 / -1; margin: 1rem 0 .3rem; padding: .4rem;
                background: #fff3cd; color: #7a5b00; border-left: 3px solid #e0a800;
                font-size: .9rem; }

  .promptsearch, .skipped { grid-column: 1 / -1; }
  .promptsearch { margin: .5rem 0; display: flex; gap: .4rem; align-items: center; }
  .promptsearch input { flex: 1; max-width: 30rem; padding: .3rem; }
  .promptsearch a { color: #8ac; }

  .skipped { color: #e0a800; font-size: .85rem; margin: .2rem 0 .6rem; }

</style></head><body>
__BODY__
<script>
const phases = ['pick', 'schedule', 'describe', 'publish', 'calendar'];
const dirty = new Map(), timers = new Map();
let saves = Promise.resolve();
const status = document.getElementById('status');
function report(message, error=false) {
  status.textContent = message; status.classList.toggle('error', error);
}
async function post(endpoint, data) {
  const r = await fetch(endpoint, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(data)});
  const d = await r.json();
  if (!r.ok) throw new Error(d.error || ('HTTP ' + r.status));
  return d;
}
function values(card) {
  return Object.fromEntries([...card.querySelectorAll('[data-field][data-dirty]')].map(e => [e.dataset.field, e.value]));
}
function saveDraft(card) {
  const uuid = card.dataset.uuid, draft = values(card);
  clearTimeout(timers.get(uuid)); timers.delete(uuid);
  const job = saves.catch(() => {}).then(async () => {
    card.querySelector('.status').textContent = 'Saving…';
    try {
      await post('/metadata', {uuid, ...draft});
      for (const [field, value] of Object.entries(draft)) {
        const input = card.querySelector('[data-field="'+field+'"]');
        if (input.value === value) delete input.dataset.dirty;
      }
      if (!card.querySelector('[data-dirty]')) dirty.delete(uuid);
      card.querySelector('.status').textContent = dirty.has(uuid) ? 'Unsaved' : 'Saved';
    } catch (e) {
      card.querySelector('.status').textContent = 'Save failed: ' + e.message;
      throw e;
    }
  });
  saves = job;
  job.catch(e => report(e.message, true));
  return job;
}
async function flushDrafts() {
  for (const card of [...dirty.values()]) saveDraft(card);
  await saves;
  if (dirty.size) throw new Error('Unsaved drafts; retry saving before leaving this phase.');
}
function reveal(name) {
  if (!phases.includes(name)) name = 'pick';
  for (const phase of phases) {
    document.getElementById(phase + '-tab').hidden = phase !== name;
    document.getElementById('tab-' + phase).classList.toggle('on', phase === name);
    document.getElementById('tab-' + phase).setAttribute('aria-pressed', String(phase === name));
  }
  history.replaceState(null, '', '#' + name);
  if (name === 'calendar') document.querySelector('.month.now')?.scrollIntoView({block:'start'});
}
async function showTab(name) {
  try { await flushDrafts(); reveal(name); } catch(e) { report(e.message, true); }
}
window.addEventListener('beforeunload', e => {
  if (dirty.size) { e.preventDefault(); e.returnValue = ''; }
});
document.addEventListener('input', e => {
  if (!e.target.matches('[data-field]')) return;
  const card = e.target.closest('[data-uuid]');
  e.target.dataset.dirty = 'true';
  dirty.set(card.dataset.uuid, card); card.querySelector('.status').textContent = 'Unsaved';
  clearTimeout(timers.get(card.dataset.uuid));
  timers.set(card.dataset.uuid, setTimeout(() => saveDraft(card), 350));
});
function refreshSelection() {
  const cells = [...document.querySelectorAll('.platform-cell:not(:disabled)')];
  const checked = cells.filter(c => c.checked).length;
  const all = document.getElementById('select-all');
  all.checked = cells.length > 0 && checked === cells.length;
  all.indeterminate = checked > 0 && checked < cells.length;
  all.disabled = cells.length === 0;
}
async function refresh() { await flushDrafts(); location.reload(); }
document.addEventListener('change', async e => {
  const el = e.target, row = el.closest('[data-uuid]');
  try {
    if (el.matches('.platform-cell')) {
      await post('/selection', {uuid:row.dataset.uuid, platform:el.dataset.platform, selected:el.checked});
      await refresh();
    } else if (el.id === 'select-all') {
      await post('/selection/all', {selected:el.checked}); await refresh();
    } else if (el.matches('.f-preset')) {
      await post('/schedule', {uuid:row.dataset.uuid, scheduleProfile:el.value}); await refresh();
    } else if (el.matches('.f-schedule')) {
      await post('/schedule', {uuid:row.dataset.uuid, schedule:el.value}); await refresh();
    }
  } catch (e) { report(e.message, true); if (el.matches('.platform-cell,#select-all')) el.checked = !el.checked; }
});
document.addEventListener('click', async e => {
  const btn = e.target.closest('[data-action]'); if (!btn) return;
  const action = btn.dataset.action, card = btn.closest('[data-uuid]'), uuid = card?.dataset.uuid;
  if (action === 'tab') { await showTab(btn.dataset.phase); return; }
  btn.disabled = true;
  let disabledMetadata = [];
  try {
    await flushDrafts();
    if (action === 'pick' || action === 'delete') {
      if (action === 'delete' && !confirm('Delete ' + btn.dataset.path + ' from disk?')) return;
      await post('/' + action, {path:btn.dataset.path});
    } else if (action === 'reload') {
      const limit = Number(document.getElementById('search-limit').value);
      const paths = [...document.querySelectorAll('[name="search-path"]:checked')].map(e => e.value);
      await post('/search', {limit, paths});
    } else if (action === 'ai-batch') {
      const opt = document.getElementById('ai-model').selectedOptions[0];
      if (opt.dataset.source === 'local' && opt.dataset.installed === 'false' &&
          !confirm('Install ' + opt.dataset.ollama + ' and generate?')) return;
      const d = await post('/ai/batch', {source:opt.dataset.source,
        model:opt.dataset.model ? JSON.parse(opt.dataset.model) : null});
      sessionStorage.setItem('ai-batch-result', JSON.stringify(d));
      await refresh(); return;
    } else if (action === 'ai') {
      const opt = document.getElementById('ai-model').selectedOptions[0];
      if (opt.dataset.source === 'local' && opt.dataset.installed === 'false' &&
          !confirm('Install ' + opt.dataset.ollama + ' and generate?')) return;
      disabledMetadata = [...card.querySelectorAll('[data-field]')];
      disabledMetadata.forEach(field => { field.disabled = true; });
      const r = await fetch('/ai', {method:'POST', headers:{'Content-Type':'application/json'},
        body:JSON.stringify({uuid, source:opt.dataset.source, model:opt.dataset.model ? JSON.parse(opt.dataset.model) : null})});
      const d = await r.json();
      if (d.saved) {
        for (const field of ['title', 'description']) card.querySelector('[data-field="'+field+'"]').value = d[field];
        card.querySelector('.status').textContent = dirty.has(uuid) ? 'Unsaved' : (d.error ? 'Saved AI metadata pending' : 'Saved');
      }
      if (!r.ok) throw new Error(d.error || ('HTTP ' + r.status));
      report('Generated and saved; approve metadata after review.'); await refresh(); return;
    } else if (action === 'save-options') {
      const form = btn.closest('.options');
      await post('/options', {uuid, platform:'deviantart',
        price:form.querySelector('.f-price').value === '' ? null : Number(form.querySelector('.f-price').value),
        tiers:[...form.querySelectorAll('.f-tier:checked')].map(e => e.value),
        galleries:[...form.querySelectorAll('.f-gallery:checked')].map(e => e.value)});
    } else if (action === 'publish') {
      const d = await post('/publish', {});
      sessionStorage.setItem('publish-result', JSON.stringify(d));
    } else {
      const routes = {'approve-date':'/schedule/approve', 'suggest':'/schedule',
                      'approve-metadata':'/metadata/approve', 'return-schedule':'/schedule/return',
                      'return-metadata':'/metadata/return'};
      await post(routes[action], action === 'suggest' ? {uuid, scheduleIsManual:false} : {uuid});
    }
    report('Saved'); await refresh();
  } catch(e) { report(e.message, true); } finally {
    disabledMetadata.forEach(field => { field.disabled = false; });
    btn.disabled = false;
  }
});
document.querySelector('.promptsearch')?.addEventListener('submit', async e => {
  e.preventDefault(); try { await flushDrafts(); e.target.submit(); } catch(e) { report(e.message, true); }
});
document.getElementById('calendar-tab').addEventListener('click', async e => {
  const a = e.target.closest('a[href^="#"]'); if (!a) return;
  e.preventDefault(); a.closest('[popover]')?.hidePopover();
  try {
    await flushDrafts();
    const row = document.getElementById(a.getAttribute('href').slice(1));
    if (row) { reveal(row.closest('[data-phase]').dataset.phase); row.scrollIntoView({block:'center'}); }
  } catch(e) { report(e.message, true); }
});
const batchResult = sessionStorage.getItem('ai-batch-result');
if (batchResult) {
  sessionStorage.removeItem('ai-batch-result');
  try {
    const d = JSON.parse(batchResult);
    report('Batch descriptions: ' + d.succeeded + ' succeeded, ' + d.failed + ' failed.', d.failed > 0);
  } catch(e) { report(e.message, true); }
}
const previous = sessionStorage.getItem('publish-result');
if (previous) {
  try {
    const d = JSON.parse(previous);
    document.getElementById('publish-result').textContent =
      'Published: ' + d.published + '; failed: ' + d.failed + '; not attempted: ' + d.unattempted + '; still pending: ' + d.remaining.length +
      (d.error ? '\n' + d.error : '') + '\nRetry Publish selected to attempt only pending cells.';
  } catch(e) { report(e.message, true); }
}
refreshSelection(); reveal(location.hash.slice(1));
</script></body></html>"""


def escaped(value) -> str:
    return html.escape(str(value), quote=True)


def model_options(vision_models, statuses, ai_model, openrouter_model):
    options = []
    if vision_models:
        model = next((m for m in vision_models if m['openrouter_name'] == openrouter_model), vision_models[0])
        options.append(f'<option data-source="openrouter" data-model="{escaped(json.dumps(model))}">OpenRouter (free)</option>')
        for m in vision_models:
            status = statuses.get(m['ollama_name'], {})
            disabled = not status.get('fit') or not status.get('available')
            suffix = (status.get('reason') or 'Unavailable') if disabled else ('Install on use' if not status.get('installed') else '')
            options.append(f'<option data-source="local" data-model="{escaped(json.dumps(m))}" '
                           f'data-ollama="{escaped(m["ollama_name"])}" data-installed="{str(bool(status.get("installed"))).lower()}"'
                           f'{" disabled" if disabled else ""}>Local — {escaped(m["ollama_name"])} {escaped(suffix)}</option>')
    else:
        options.append(f'<option data-source="openrouter">OpenRouter (free) — {escaped(openrouter_model)}</option>')
    options.append(f'<option data-source="claude">Claude (subscription) — {escaped(ai_model)}</option>')
    return ''.join(options)


def render_page(*, thumb_map, candidates, publications, timeline, schedules, config,
                ai_model, openrouter_model, platforms, prompt_html=None, query='', skipped=0,
                maybe_candidates=None, search_enabled=True, truncated=False, lineage_active=False,
                vision_models=None, local_model_statuses=None, search=None,
                publicable_choices=()) -> str:
    tz = zone(config.get('schedule', {}))
    prompt_html = prompt_html or {}
    groups = {'schedule': [p for p in publications if p['state'] == 'PICKED'],
              'describe': [p for p in publications if p['state'] == 'SCHEDULED'],
              'publish': [p for p in publications if p['state'] in ('DESCRIBED', 'APPEARED')]}
    counts = {'pick': len(candidates) + len(maybe_candidates or []), **{k:len(v) for k,v in groups.items()}}
    nav = ''.join(f'<button id="tab-{k}" class="tab" data-action="tab" data-phase="{k}" aria-pressed="false">'
                  f'{k.capitalize()} ({counts[k]})</button>' for k in counts)
    nav += '<button id="tab-calendar" class="tab" data-action="tab" data-phase="calendar" aria-pressed="false">Calendar</button>'
    body = '<header><strong>Publish next</strong>' + nav + '<span id="status" role="status" aria-live="polite"></span></header><main>'

    def image(path, title):
        if not path:
            return '<span>Artwork missing from disk</span>'
        href = escaped('/original?path=' + urllib.parse.quote(path))
        return f'<a href="{href}" target="_blank" rel="noopener"><img src="/thumbs/{escaped(thumb_map.get(path, ""))}" alt="{escaped(title)}"></a>'

    def button(action, label, disabled=False):
        return f'<button data-action="{action}"{" disabled" if disabled else ""}>{label}</button>'

    search = search or Search(25, tuple(publicable_choices))
    body += '<section id="pick-tab" class="grid" data-phase="pick">'
    body += '<div class="searchcontrols"><label>Candidate limit <select autocomplete="off" id="search-limit">'
    for limit, label in ((0, 'Empty'), (25, '25'), (50, '50'), (75, '75'), (100, '100')):
        body += f'<option value="{limit}"{" selected" if search.limit == limit else ""}>{label}</option>'
    body += '</select></label><fieldset><legend>Search paths</legend>'
    for path in publicable_choices:
        body += f'<label><input autocomplete="off" type="checkbox" name="search-path" value="{escaped(path)}"{" checked" if path in search.paths else ""}> {escaped(path)}</label>'
    body += '</fieldset>' + button('reload', 'Reload') + '</div>'
    if search_enabled:
        body += (f'<form class="promptsearch" method="get" action="/">'
                 f'<input autocomplete="off" type="search" name="prompt" aria-label="Search prompt text" value="{escaped(query)}">'
                 '<button type="submit">Search</button><a href="/">clear</a></form>')
        if skipped:
            body += f'<p class="skipped">{skipped} candidate{"" if skipped == 1 else "s"} skipped — their prompt was never archived.</p>'
        if truncated:
            body += '<p class="skipped">more matches exist than are shown — narrow your search to see the rest.</p>'
    if lineage_active and candidates:
        body += '<div class="groupsplit">images from this prompt</div>'
    for paths, hint in ((candidates, False), (maybe_candidates or [], True)):
        if hint and paths:
            body += '<div class="groupsplit">possibly from this prompt — their own prompt was never archived; filename hint only</div>'
        for i, path in enumerate(paths):
            body += (f'<article class="card candidate" id="candidate-{int(hint)}-{i}">{image(path, os.path.basename(path))}'
                     f'<div class="name">{escaped(os.path.basename(path))}</div>{prompt_html.get(path, "")}'
                     f'<button data-action="pick" data-path="{escaped(path)}">Pick</button>'
                     f'<button data-action="delete" data-path="{escaped(path)}">Delete from disk</button></article>')
    body += '</section>'
    occupancy = {}
    for pub in publications:
        ts = effective_schedule_ts(pub)
        if ts is not None:
            occupancy[ts] = occupancy.get(ts, 0) + 1
    anchors = {}
    for phase in ('schedule', 'describe'):
        body += f'<section id="{phase}-tab" class="grid" data-phase="{phase}" hidden>'
        if phase == 'describe':
            body += '<label>AI provider <select autocomplete="off" id="ai-model">' + model_options(vision_models, local_model_statuses or {}, ai_model, openrouter_model) + '</select></label>'
            body += button('ai-batch', 'Describe all marked')
        for pub in groups[phase]:
            uuid = pub['uuid']; anchors[uuid] = 'publication-' + uuid
            ts = pub.get('apparitionTimestampIfDifferentThanSubmission')
            label = datetime.fromtimestamp(ts, tz).strftime('%Y-%m-%dT%H:%M') if ts else ''
            body += f'<article class="card" id="publication-{escaped(uuid)}" data-uuid="{escaped(uuid)}">'
            body += image(pub.get('path', ''), pub['title'] or os.path.basename(pub.get('path', ''))) + f'<div>{escaped(label)} ({escaped(tz.key)})</div>'
            if phase == 'schedule':
                opts = ''.join(f'<option value="{escaped(p["name"])}"{" selected" if p["name"] == pub.get("scheduleProfile") else ""}>{escaped(p["name"])}</option>' for p in schedules)
                if not any(p['name'] == pub.get('scheduleProfile') for p in schedules):
                    opts = '<option value="" selected disabled>Custom / unassigned</option>' + opts
                body += f'<label>Preset <select autocomplete="off" class="f-preset">{opts}</select></label>'
                body += f'<label>Date ({escaped(tz.key)}) <input autocomplete="off" class="f-schedule" type="datetime-local" value="{escaped(label)}"></label>'
                body += '<span>' + ('Manual pin' if pub.get('scheduleIsManual') else 'Automatic suggestion') + '</span>'
                if not ts:
                    body += '<span class="warning">No slot available; choose a date before approval.</span>'
                if any(ts in p['slots'] and occupancy.get(ts, 0) > p['per_slot'] for p in schedules):
                    body += '<span class="warning">Over capacity: explicit dates remain as chosen.</span>'
                body += button('suggest', 'Use suggested date') + button('approve-date', 'Approve date', not ts)
            else:
                marked = any(TODO_GENERATE in pub.get(k, '') for k in ('title', 'description'))
                if marked:
                    body += '<span class="warning">AI metadata pending</span>'
                body += f'<label>Title <input autocomplete="off" type="text" data-field="title" value="{escaped(pub["title"])}"></label>'
                body += f'<label>Description <textarea autocomplete="off" data-field="description">{escaped(pub["description"])}</textarea></label>'
                body += button('ai', 'Generate with AI')
                body += button('approve-metadata', 'Approve metadata') + button('return-schedule', 'Return to scheduling')
                body += '<span class="status" role="status" aria-live="polite">Saved</span>'
            body += '</article>'
        body += '</section>'
    body += '<section id="publish-tab" data-phase="publish" hidden><label><input autocomplete="off" id="select-all" type="checkbox"> Publish all on all (selection only)</label>'
    body += button('publish', 'Publish selected / Retry') + '<pre id="publish-result" role="status" aria-live="polite"></pre>'
    body += '<table class="platforms"><caption>Platform selections</caption><thead><tr><th scope="col">Publication</th>'
    body += ''.join(f'<th scope="col">{escaped(label)}</th>' for label in platforms.values()) + '</tr></thead><tbody>'
    for pub in groups['publish']:
        uuid = pub['uuid']; anchors[uuid] = 'publication-' + uuid
        ts = effective_schedule_ts(pub)
        date = datetime.fromtimestamp(ts, tz).strftime('%Y-%m-%d %H:%M') if ts else 'No date'
        body += f'<tr id="publication-{escaped(uuid)}" data-uuid="{escaped(uuid)}"><th scope="row">'
        body += image(pub.get('path', ''), pub['title']) + f'<div>{escaped(pub["title"])}</div><div>{escaped(date)} ({escaped(tz.key)})</div>'
        if not has_delivery_success(pub):
            body += button('return-metadata', 'Return to metadata') + button('return-schedule', 'Return to scheduling')
        unregistered = [a['platformName'] for a in pub.get('apparitions', []) if a['platformName'] not in platforms]
        if unregistered:
            body += '<div>Historical platforms (no registered publisher): ' + escaped(', '.join(unregistered)) + '</div>'
        body += '</th>'
        for name, platform_label in platforms.items():
            app = next((a for a in pub.get('apparitions', []) if a['platformName'] == name), None)
            locked = app is not None and app['state'] == 'published_or_scheduled'
            body += f'<td><label><input autocomplete="off" class="platform-cell" type="checkbox" data-platform="{escaped(name)}" aria-label="{escaped(pub["title"] + " on " + platform_label)}"'
            body += (' checked disabled' if locked else ' checked' if app else '') + '>'
            body += (' Published / scheduled' if locked else ' Selected' if app else ' Select') + '</label>'
            if app and not locked and name == 'deviantart':
                body += '<div class="options">'
                body += f'<label>Price (optional) <input autocomplete="off" type="number" class="f-price" min="0" step="0.01" value="{escaped(app.get("priceIfNotFree", ""))}"></label>'
                for key, cls in (('tiers', 'f-tier'), ('galleries', 'f-gallery')):
                    selected = app.get(key, [])
                    body += f'<fieldset><legend>{key.capitalize()}</legend>'
                    for option in config.get(key, []):
                        body += f'<label><input autocomplete="off" type="checkbox" class="{cls}" value="{escaped(option)}"{" checked" if option in selected else ""}> {escaped(option)}</label>'
                    body += '</fieldset>'
                body += button('save-options', 'Save platform options') + '</div>'
            body += '</td>'
        body += '</tr>'
    body += '</tbody></table></section><section id="calendar-tab" data-phase="calendar" hidden>'
    body += render_calendar(timeline, tz=tz, anchors=anchors, prompts=prompt_html) + '</section></main>'
    # Insert arbitrary escaped user/prompt text once, last; never interpolate it into JS.
    return PAGE_TEMPLATE.replace('__BODY__', body)
