# Schedule Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `publicator.toml` define several named weekly schedule profiles and let the gallery UI pick one per card (via a preset selector) to pre-fill the datepicker with that profile's next open slot.

**Architecture:** All work is in `publish_next.py` (config→client cadence list, occupancy-based slot math in the browser JS, a preset `<select>` per card) plus a self-check lock in `validate.py`. No schema change: a card's profile is inferred from its timestamp's UTC weekday+hour. Backward-compatible with the old flat `[schedule]` table.

**Tech Stack:** Python 3.11 (stdlib `tomllib`, `http.server`), vanilla browser JS, pytest, per-module `--selfcheck`.

## Global Constraints

- Times are UTC; `hour` is 0–23. `day` is a lowercase weekday name.
- Only `frequency = "weekly"` is implemented; anything else raises `NotImplementedError`.
- `per_slot` must be `>= 1`.
- Profiles must have distinct `(day, hour)` pairs (membership is inferred from the timestamp) — colliding profiles raise `ValueError` at load.
- No new persisted field; `scheduleTs` is stored exactly as today.
- DeviantArt does not accept `.webp` for publications (unrelated to this change, but the repo rule).
- Run the suite with `nix develop -c pytest`; run module self-checks with `python <module>.py --selfcheck`.

---

### Task 1: Schedule profiles — config parsing, occupancy background, UI

**Files:**
- Modify: `publish_next.py`
  - `_JS_DAY` block (~135) — keep.
  - Replace `_schedule_js` (~139-152) with `_schedules_js`.
  - Replace `_max_existing_ts` (~155-167) with `_existing_ts`.
  - `_selfcheck` schedule asserts (~170-181).
  - Client JS constants + slot math in `_PAGE_TMPL` (~292-318, ~333-342).
  - `_card_form_html` (~499-514) + new `_preset_options` helper.
  - `_build_page` (~531-588): compute preset options, pass to both `_card_form_html` calls, rename the two template `.replace` names.
  - `GalleryHandler` class attrs (~522, ~528).
  - `serve` (~823, ~829).
- Modify: `validate.py` — `_selfcheck` (~44-64): add a profiles round-trip assertion.
- Test: `test_publish_next.py` — rename `initial_max_ts`→`existing_ts` in the two existing tests; add `test_page_renders_schedule_presets`.

**Interfaces:**
- Produces:
  - `_schedules_js(schedule: dict) -> list[dict]` → `[{"name": str, "day": int (JS getUTCDay), "hour": int, "per_slot": int}, ...]`. Reads `schedule["profiles"]`; falls back to the flat `day`/`hour`/`per_slot`/`frequency` keys as one `"default"` profile, or a built-in Tuesday-20:00-per_slot-2 default when the dict is empty. Validates weekly-only, valid day, `per_slot >= 1`, distinct `(day, hour)`.
  - `_existing_ts(json_path: str) -> list[int]` → timestamps of apparitions whose `state != "unpublished"` (already-scheduled/published background). Missing/invalid file → `[]`.
  - `_preset_options(schedules: list[dict]) -> str` → `<option>` HTML for the preset select, ending with `<option value="__custom__">(custom)</option>`.
  - `GalleryHandler.schedules: list`, `GalleryHandler.existing_ts: list[int]`.
  - Browser JS globals `SCHEDULES`, `EXISTING_TS`; JS `nextSlotForProfile(p)`, `presetForTs(ts)`, `applyPreset(sel, cardId)`.
- Consumes: `STATE_UNPUBLISHED` (already imported from `da_publish`), `_JS_DAY`.

- [ ] **Step 1: Lock profiles pass-through in `validate.py` self-check (write the assertion first)**

In `validate.py` `_selfcheck`, after the existing flat-config block (right before `print("validate selfcheck OK")`), add a second config write + assertion:

```python
        (Path(d) / "publicator.toml").write_text(
            '[[schedule.profiles]]\nname = "free"\nday = "tuesday"\nhour = 20\nper_slot = 2\n'
            '[[schedule.profiles]]\nname = "paid"\nday = "friday"\nhour = 20\nper_slot = 1\n'
        )
        c = load_config(d)
        assert c["schedule"] == {"profiles": [
            {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
            {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1},
        ]}, c
```

- [ ] **Step 2: Run the validate self-check — expect PASS (behavior lock, `load_config` already passes the table through)**

Run: `python validate.py --selfcheck`
Expected: `validate selfcheck OK` (if it fails, `load_config` needs `"schedule": dict(cfg.get("schedule", {}))` — it already has this, so it should pass).

- [ ] **Step 3: Rewrite `publish_next._selfcheck` schedule asserts to target the new `_schedules_js` (test-first)**

Replace the schedule block at the top of `_selfcheck` (the `sj = _schedule_js(...)` through the `per_slot=0` try/except, ~171-181) with:

```python
    # schedule cadence: profiles -> client list; weekly day-name -> JS getUTCDay index
    assert _schedules_js({"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2}]}) == \
        [{"name": "free", "day": 2, "hour": 20, "per_slot": 2}]
    # flat keys -> single "default" profile
    assert _schedules_js({"day": "friday", "hour": 18, "per_slot": 1}) == \
        [{"name": "default", "day": 5, "hour": 18, "per_slot": 1}]
    # empty -> built-in default (Tuesday 20:00, per_slot 2)
    assert _schedules_js({}) == [{"name": "default", "day": 2, "hour": 20, "per_slot": 2}]
    for bad, exc in [
        ({"profiles": [{"frequency": "monthly"}]}, NotImplementedError),
        ({"profiles": [{"day": "tuesday", "per_slot": 0}]}, ValueError),
        ({"profiles": [{"name": "a", "day": "tuesday", "hour": 20},
                       {"name": "b", "day": "tuesday", "hour": 20}]}, ValueError),
    ]:
        try:
            _schedules_js(bad); assert False, f"{bad} not rejected"
        except exc:
            pass
```

- [ ] **Step 4: Run the publish self-check — expect FAIL (`_schedules_js` not defined)**

Run: `python publish_next.py --selfcheck`
Expected: FAIL with `NameError: name '_schedules_js' is not defined`.

- [ ] **Step 5: Implement `_schedules_js` (replace `_schedule_js`)**

Replace the whole `_schedule_js` function with:

```python
_DEFAULT_PROFILE = {"name": "default", "day": "tuesday", "hour": 20, "per_slot": 2}


def _schedules_js(schedule: dict) -> list[dict]:
    """Cadence list for the gallery client: [{name, day: <JS getUTCDay index>,
    hour, per_slot}, ...]. Reads schedule['profiles']; a flat day/hour/per_slot
    config becomes one 'default' profile, an empty dict the built-in default.
    Only frequency='weekly' is implemented; profiles need distinct (day, hour)."""
    raw = schedule.get("profiles")
    if raw is None:
        overrides = {k: schedule[k] for k in ("day", "hour", "per_slot", "frequency")
                     if k in schedule}
        raw = [{**_DEFAULT_PROFILE, **overrides}]
    out, seen = [], set()
    for p in raw:
        freq = p.get("frequency", "weekly")
        if freq != "weekly":
            raise NotImplementedError(f"schedule.frequency {freq!r} not implemented (only 'weekly')")
        day = str(p.get("day", "tuesday")).lower()
        if day not in _JS_DAY:
            raise ValueError(f"schedule.day {day!r} invalid")
        hour = int(p.get("hour", 20))
        per_slot = int(p.get("per_slot", 2))
        if per_slot < 1:
            raise ValueError(f"schedule.per_slot must be >= 1, got {per_slot}")
        key = (_JS_DAY[day], hour)
        if key in seen:
            raise ValueError(f"schedule profiles collide on (day={day}, hour={hour})")
        seen.add(key)
        out.append({"name": str(p.get("name", "default")),
                    "day": _JS_DAY[day], "hour": hour, "per_slot": per_slot})
    return out
```

- [ ] **Step 6: Implement `_existing_ts` (replace `_max_existing_ts`)**

Replace the whole `_max_existing_ts` function with:

```python
def _existing_ts(json_path: str) -> list[int]:
    """Timestamps of already-scheduled apparitions (state != unpublished) — the
    occupancy background the client packs new slots around. Unpublished (pending)
    entries are excluded; they ride in the client queue instead."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return []
    out = []
    for entry in data:
        for app in entry.get("apparitions", []):
            ts = app.get("apparitionTimestampIfDifferentThanSubmission")
            if ts and app.get("state") != STATE_UNPUBLISHED:
                out.append(int(ts))
    return out
```

- [ ] **Step 7: Update the `GalleryHandler` class attrs**

In the `GalleryHandler` class body, change:
- `initial_max_ts = 0` → `existing_ts: list[int] = []`
- `schedule: dict = _schedule_js({})` → `schedules: list = _schedules_js({})`

- [ ] **Step 8: Update `serve()` wiring**

In `serve()` change:
- `GalleryHandler.initial_max_ts = _max_existing_ts(args.json)` → `GalleryHandler.existing_ts = _existing_ts(args.json)`
- `GalleryHandler.schedule = _schedule_js(config["schedule"])` → `GalleryHandler.schedules = _schedules_js(config["schedule"])`

- [ ] **Step 9: Add the `_preset_options` helper (next to `_tier_gallery_fields`)**

```python
def _preset_options(schedules):
    opts = "".join(f'<option value="{html.escape(s["name"], quote=True)}">'
                   f'{html.escape(s["name"])}</option>' for s in schedules)
    return opts + '<option value="__custom__">(custom)</option>'
```

- [ ] **Step 10: Add the preset `<select>` to `_card_form_html`**

Change the signature to `def _card_form_html(cid, js_path, tg, save_label, preset_opts):` and insert one label immediately **above** the existing `<label>Schedule …</label>` line:

```python
    <label>Schedule preset <select class="f-preset" onchange="applyPreset(this, '{cid}')">{preset_opts}</select></label>
    <label>Schedule <input type="datetime-local" class="f-schedule"></label>
```

- [ ] **Step 11: Wire preset options through `_build_page`**

In `_build_page`, next to `tg = _tier_gallery_fields(self.config)` add:

```python
        preset_opts = _preset_options(self.schedules)
```

Pass `preset_opts` as the 5th arg to **both** `_card_form_html(...)` calls (pending cards and candidate cards). Then change the two template replacements:
- `page.replace("__INITIAL_MAX_TS__", str(self.initial_max_ts))` → `page.replace("__EXISTING_TS__", json.dumps(self.existing_ts))`
- `page.replace("__SCHEDULE__", json.dumps(self.schedule))` → `page.replace("__SCHEDULES__", json.dumps(self.schedules))`

- [ ] **Step 12: Replace the client JS constants + slot math in `_PAGE_TMPL`**

Replace the block from `const INITIAL_MAX_TS = __INITIAL_MAX_TS__;` through the `localInputToTs` function (the constants, `nextSlotBase`, `slotForIndex`, `tsToLocalInput`, `localInputToTs`) with:

```javascript
const EXISTING_TS = __EXISTING_TS__;   // already-scheduled background timestamps
const SCHEDULES = __SCHEDULES__;       // [{name, day, hour, per_slot}, ...]
const PENDING = __PENDING__;           // already-queued entries from publications.json

const queue = []; // [{cardId, path, title, description, price, scheduleTs, tier, galleries, uuid?}]

function profileByName(name) {
  return SCHEDULES.find(s => s.name === name) || SCHEDULES[0];
}
function presetForTs(ts) {              // profile whose (day, hour) matches ts, else custom
  if (!ts) return SCHEDULES[0].name;
  const d = new Date(ts * 1000);
  const m = SCHEDULES.find(s => s.day === d.getUTCDay() && s.hour === d.getUTCHours());
  return m ? m.name : "__custom__";
}
function nextWeekday(afterTs, day, hour) {
  const d = new Date(afterTs * 1000);
  let days = (day - d.getUTCDay() + 7) % 7;
  if (days === 0) days = 7;             // ponytail: skip same-day; hand-edit if you want today
  d.setUTCDate(d.getUTCDate() + days);
  d.setUTCHours(hour, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}
function occupancyTs(p) {                // background + queue, restricted to p's (weekday, hour)
  const all = EXISTING_TS.concat(queue.map(e => e.scheduleTs).filter(Boolean));
  return all.filter(ts => {
    const d = new Date(ts * 1000);
    return d.getUTCDay() === p.day && d.getUTCHours() === p.hour;
  });
}
function nextSlotForProfile(p) {         // earliest FUTURE slot with room (< per_slot)
  const taken = occupancyTs(p);
  let slot = nextWeekday(Math.floor(Date.now() / 1000), p.day, p.hour);
  while (taken.filter(ts => ts === slot).length >= p.per_slot) slot += 7 * 24 * 3600;
  return slot;
}
function tsToLocalInput(ts) {
  const d = new Date(ts * 1000);
  const pad = n => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
function localInputToTs(s) {
  return Math.floor(new Date(s).getTime() / 1000);
}
function applyPreset(sel, cardId) {      // preset change -> refill the picker
  if (sel.value === "__custom__") return;
  const card = document.getElementById(cardId);
  card.querySelector(".f-schedule").value = tsToLocalInput(nextSlotForProfile(profileByName(sel.value)));
}
```

- [ ] **Step 13: Update `openForm` to drive the preset select**

In `openForm`, replace the `if (entry) { … } else { … }` schedule block with:

```javascript
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
```

- [ ] **Step 14: Update the two existing tests + add the preset render test**

In `test_publish_next.py`, change both `H.initial_max_ts = 0` (and `H.initial_max_ts = 0; …`) to `H.existing_ts = []`. Then append:

```python
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
```

- [ ] **Step 15: Run the publish self-check — expect PASS**

Run: `python publish_next.py --selfcheck`
Expected: `publish_next selfcheck OK`.

- [ ] **Step 16: Run the full suite + self-checks — expect PASS**

Run: `nix develop -c pytest -q && python validate.py --selfcheck && python publish_next.py --selfcheck && python da_publish.py --selfcheck`
Expected: pytest green (incl. `test_page_renders_schedule_presets`), all three self-checks OK.

- [ ] **Step 17: Commit**

```bash
git add publish_next.py validate.py test_publish_next.py
git commit -m "feat(schedule): multiple named schedule profiles with per-card preset selector"
```

---

### Task 2: Browser verification of the preset UI + close the TODO

**Files:**
- Create (scratch, not committed): a temp data dir with `picked/` + two images + a `publicator.toml` with two profiles.
- Modify: `docs/todos.md` — check off the first Schedule bullet.

**Interfaces:**
- Consumes: the gallery served by `python publish_next.py --data-dir <scratch> --port <PORT>`.

- [ ] **Step 1: Build a scratch data dir**

```bash
D="/tmp/claude-1000/-home-theta--vault-repos-publicator-py/632f6b5e-00f8-4e0a-bf73-26dac05fde45/scratchpad/sched-verify"
mkdir -p "$D/picked"
printf '\x89PNG\r\n\x1a\n' > "$D/picked/a.png"   # header is enough for thumbnail fallback
cp "$D/picked/a.png" "$D/picked/b.png"
cat > "$D/publicator.toml" <<'TOML'
publicable = ["picked"]
[[schedule.profiles]]
name = "free"
day = "tuesday"
hour = 20
per_slot = 2
[[schedule.profiles]]
name = "paid"
day = "friday"
hour = 20
per_slot = 1
TOML
```

- [ ] **Step 2: Launch the gallery (background) against the scratch dir**

Run (background): `python publish_next.py --data-dir "$D" --json "$D/publications.json" --port 8799`
The gallery serves at `http://127.0.0.1:8799`. (Firefox may auto-open; ignore it — drive it with playwright-cli instead.)

- [ ] **Step 3: Verify with the browser (playwright-cli skill)**

Using the `playwright-cli` skill, open `http://127.0.0.1:8799`, click **Add** on a card, then confirm:
- a **Schedule preset** `<select>` shows options `free`, `paid`, `(custom)`;
- with `free` selected the datepicker holds the next **Tuesday 20:00** (local rendering of the UTC slot);
- switching to `paid` refills the datepicker with the next **Friday 20:00**;
- editing the datepicker by hand and reopening the card keeps the hand-set time and flips the preset to `(custom)` if off-cadence.

Capture a screenshot as evidence. Stop the server when done.

- [ ] **Step 4: Close the TODO**

In `docs/todos.md`, strike through the first Schedule sub-bullet:
`    - ~~Instead of 1 unique schedule (ex: 2 every tuesday), several profiles (ex: 2 free every tuesdays, 1 paid every friday)~~`
(Leave the second bullet — "Propose creation of schedules from the UI" — untouched; it was the intended drift.)

- [ ] **Step 5: Commit**

```bash
git add docs/todos.md
git commit -m "docs: mark multi-profile schedule done"
```

---

## Self-Review

**Spec coverage:**
- Config format (profiles array) → Task 1 Steps 1, 5; validate lock Step 1-2.
- Backward-compat flat/absent fallback → Step 5 (`_schedules_js`), Step 3 asserts.
- Preset selector UI, keep datepicker → Steps 9-13.
- Earliest-open-slot occupancy per profile → Step 12 (`nextSlotForProfile`/`occupancyTs`).
- Background = non-`unpublished` timestamps, list not max → Step 6 (`_existing_ts`), Step 8, Step 11.
- Distinct `(day, hour)` validation → Step 5 + Step 3 collision assert.
- Re-edit pre-selects matching profile / `(custom)` → Step 12 (`presetForTs`) + Step 13.
- No schema change → confirmed (no new field written anywhere).
- Self-checks updated (both modules) → Steps 1, 3.
- UI browser verification (global CLAUDE.md rule) → Task 2.
- Out-of-scope items (UI creation, free/paid routing, GH Actions, split-refactor) → not planned. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases" — every code step carries real code. ✓

**Type consistency:** `_schedules_js`→`list[dict]` used by class attr `schedules` and `serve`; `_existing_ts`→`list[int]` used by `existing_ts`/`serve`/`_build_page`; JS `SCHEDULES`/`EXISTING_TS`/`nextSlotForProfile`/`presetForTs`/`applyPreset`/`profileByName`/`occupancyTs`/`nextWeekday` all defined in Step 12 and consumed in Steps 12-13; `_preset_options`/`_card_form_html(…, preset_opts)` consistent across Steps 9-11. ✓
