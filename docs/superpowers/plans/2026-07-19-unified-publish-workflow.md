# Unified Publish Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Merge `publish_next.py` + `name_publications.py` + `publish_deviantart.py` into a single command `publish_next.py` with an enhanced browser UI (delete/add-with-form/AI-generate) and separate Playwright Chromium publish.

**Architecture:** Single-process stdlib HTTP server serves gallery, receives queue via POST, then invokes Playwright Chromium (persistent context at `./.deviantart-session/`) in-process to publish each entry sequentially.

**Tech Stack:** Python 3.11+, stdlib `http.server`, `playwright` (chromium), `anthropic` SDK, `jsonschema`.

## Global Constraints

- All existing `publications.json` entries and schema stay valid.
- No new production dependencies beyond `anthropic` and `playwright` (already used).
- Session dir `./.deviantart-session/` must be gitignored.
- Publish batch stops on first failure; unpublished entries stay `state="unpublished"`.
- Delete = `os.remove` on original path (no trash).
- Default AI provider = Claude; scaffold provider dispatch but only implement Claude.
- No `.webp` submitted to DeviantArt (existing rule).

---

### Task 1: Extract & test slot computation

**Files:**
- Modify: `publish_next.py` (extract `compute_next_slot`)

**Interfaces:**
- Produces: `compute_next_slot(max_existing_ts: int, added_count: int) -> int`
  — returns unix timestamp UTC for the slot the `added_count`-th newly-added
  entry should get (0-indexed). Packing: 2 per Tue 8pm UTC slot, starting the
  Tuesday after `max_existing_ts`.

Steps:
- [ ] Refactor: extract from `compute_slots`. Keep `get_next_tuesday_8pm_after` helper.
- [ ] Add `if __name__ == "__main__"` assert block (only if run as `python -c` test), OR add 3 `assert`s in a `_selfcheck()` gated by env var to avoid interfering with CLI. Simpler: add `test_publish_next.py` with plain asserts.

### Task 2: Add .gitignore entry and delete name_publications.py

- [ ] Append `.deviantart-session/` to `.gitignore`.
- [ ] `git rm name_publications.py`.

### Task 3: AI generation module

**Files:**
- Modify: `publish_next.py` (add `generate_metadata`)

**Interface:**
- `generate_metadata(image_path: str, provider: str = "claude", model: str = "claude-opus-4-7") -> tuple[str, str]`
  returns `(title, description)`. Raises `RuntimeError` with human-readable message on failure (missing key, API error).

Implementation:
- Read image, base64-encode.
- Use `anthropic.Anthropic()` with `ANTHROPIC_API_KEY` from env.
- Prompt: ask for a JSON `{"title": "...", "description": "..."}` — title ≤ 50 chars, description 2-3 sentences, no hashtags.
- Parse response JSON; raise on bad shape.
- `provider != "claude"` → `NotImplementedError`.

### Task 4: Rewrite HTTP handler

**Files:**
- Modify: `publish_next.py`

**New/changed routes:**
- `GET /` — new page with per-card Delete + Add-form (inline expand). Form fields: title, description, price (optional), datetime-local. "Generate AI" button. "Add to queue" button.
- `POST /delete` — body `{ path: str }` → `os.remove(path)` → `{ ok: true }` or `{ ok: false, error }`.
- `POST /ai` — body `{ path: str }` → returns `{ title, description }` or 400 `{ error }`.
- `POST /publish` — body `{ entries: [ { path, title, description, price?, scheduleTs } ] }` → writes to publications.json, runs publish batch, returns `{ published: n, failed: n, error? }`.

Client-side JS:
- Track in-memory queue array on the page.
- Each card has "Delete", "Add" (opens inline form), "Generate AI" (fills title+desc), "Save to queue" (validates + pushes to queue array + collapses card).
- Sidebar/header shows queue count + "Publish" button. Publish disabled if queue empty.
- Default schedule computed client-side: server injects `initialMaxTs` and a JS function pre-computes slots as user adds.

**JS slot fn (mirrors Python):**

```js
function nextTuesday8pmUTC(afterTs) {
  const d = new Date(afterTs * 1000);
  let days = (2 - d.getUTCDay() + 7) % 7; // Tue = 2 in JS
  if (days === 0) days = 7;
  d.setUTCDate(d.getUTCDate() + days);
  d.setUTCHours(20, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}
function slotForIndex(maxTs, idx) {
  let ts = nextTuesday8pmUTC(maxTs);
  const week = 7 * 24 * 3600;
  return ts + Math.floor(idx / 2) * week;
}
```

Note: Python uses `weekday()` where Monday=0, Tuesday=1. JS `getUTCDay()` has Sunday=0, Tuesday=2. Formulas are correct as shown.

### Task 5: Playwright Chromium publish

**Files:**
- Modify: `publish_next.py`

**Interface:**
- `publish_batch(entries: list[dict]) -> tuple[int, int, str | None]` returns `(published, failed, error_msg)`.
- Uses `chromium.launch_persistent_context("./.deviantart-session", headless=<auto>)`.
- Auto-headless: if session dir exists AND is non-empty → headless; else headed with a console message asking to log in, and after first `run()` completes, subsequent calls headless.
- On login-required detection (Submit link not visible after goto), relaunch headed one time.

**Port from `publish_deviantart.py`:**
- Selectors are mostly Firefox-tested; check they work on Chromium during dev. The DA UI uses standard React/DOM; no browser-engine-specific behavior expected.
- Keep `set_checkbox`, `type_tags`, `open_schedule_menu`, `pick_schedule`, `parse_schedule` unchanged (already generic).
- Instead of reading first-unpublished from JSON, iterate `entries` list; for each: resolve `art` path (same rules — reject webp, prefer jpeg-target).

### Task 6: Wire main()

- [ ] `main()` orchestrates: parse args → find candidates → thumbnails → serve → wait for `/publish` result → print summary.
- [ ] Support `--ai-provider claude --ai-model claude-opus-4-7 --port 8765`.

### Task 7: Manual verification

- [ ] Run `python publish_next.py -n 3` end-to-end against a test image dir.
- [ ] Verify browser UI: add form, AI generate (if `ANTHROPIC_API_KEY` set — skip if not), delete, override schedule.
- [ ] Verify Playwright Chromium opens headed on first run, session persists.
- [ ] Verify publications.json validates against schema after write.
