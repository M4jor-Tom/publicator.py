# Publish Pad + Hardcoded Schema Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the configurable `--schema` flag (always use the bundled schema) and add an "Add to publish pad" button that persists the RAM queue to `publications.json` as `state=unpublished` without driving DeviantArt, leaving the gallery session open.

**Architecture:** `write_publications()` already appends the queue as `state=unpublished`; today it only runs inside `/publish` right before the Firefox drive. Task 1 makes the schema path a module constant and drops it from `write_publications`'s signature and the CLI. Task 2 exposes the write half through a new non-terminal `/stage` endpoint plus a header button, promoting staged entries to "existing" (uuid-carrying) in the client queue so a later Publish never double-writes them.

**Tech Stack:** Python 3 stdlib (`http.server`, `argparse`, `json`, `uuid`), `jsonschema`, vanilla browser JS in a template string. No new dependencies.

## Global Constraints

- Single file in scope: `publish_next.py`. Do not touch `validate.py` or `llm_meta.py` (their `--schema` usage is unrelated).
- Schema file is `publicationsSchema.json` (plural), read package-relative via `PKG`, never CWD-relative.
- Staged/persisted state string is `STATE_UNPUBLISHED` (imported from `da_publish`), never a literal `"unpublished"`.
- DeviantArt rejects `.webp` for publications — unrelated to this change, do not add webp handling.
- Self-checks run via `python publish_next.py --selfcheck` (no pytest in this repo); keep them assert-based inside `_selfcheck()`.

---

### Task 1: Hardcode the schema path, drop `--schema`

**Files:**
- Modify: `publish_next.py` (constant near line 48; `write_publications` line ~591; `GalleryHandler.schema_path` line ~401; `serve()` line ~652; `/publish` handler line ~562; argparse line ~684; `_selfcheck()` line ~165)

**Interfaces:**
- Produces: `SCHEMA_PATH: Path` module constant; `write_publications(entries: list[dict], json_path: str) -> list[str]` (returns new uuids in order, appends each entry with `state=STATE_UNPUBLISHED`).
- Consumes: existing `PKG`, `STATE_UNPUBLISHED`, `_atomic_write_json`, `compute_next_slot`, `_max_existing_ts`.

- [ ] **Step 1: Write the failing self-check**

In `_selfcheck()`, before the final `print("selfcheck OK")`, add a block that exercises the new two-arg signature end to end:

```python
    # write_publications: appends state=unpublished, returns one uuid per entry,
    # validates against the bundled SCHEMA_PATH (no --schema flag).
    import tempfile as _tf
    with _tf.TemporaryDirectory() as _d:
        _jp = os.path.join(_d, "publications.json")
        _img = os.path.join(_d, "pic.png")
        with open(_img, "wb") as _f:
            _f.write(b"\x89PNG\r\n\x1a\n")  # bytes are enough for sha512
        _uuids = write_publications(
            [{"path": _img, "title": "t", "description": "d", "scheduleTs": s0}],
            _jp,
        )
        assert len(_uuids) == 1, _uuids
        with open(_jp) as _f:
            _rows = json.load(_f)
        assert len(_rows) == 1 and _rows[0]["uuid"] == _uuids[0], _rows
        assert _rows[0]["state"] == STATE_UNPUBLISHED, _rows[0]
```

- [ ] **Step 2: Run the self-check to verify it fails**

Run: `python publish_next.py --selfcheck`
Expected: FAIL — `TypeError: write_publications() missing 1 required positional argument: 'schema_path'` (signature still takes three args).

- [ ] **Step 3: Add the constant and drop the schema plumbing**

Near line 48, right after `PKG = ...`:

```python
SCHEMA_PATH = PKG / "publicationsSchema.json"  # bundled code asset, not configurable
```

Change `write_publications` signature and its schema read. Header line ~591:

```python
def write_publications(entries: list[dict], json_path: str) -> list[str]:
```

Inside it, replace the `with open(schema_path, ...)` block (line ~629):

```python
    from jsonschema import validate
    with open(SCHEMA_PATH, "r", encoding="utf-8") as s:
        schema = json.load(s)
    validate(instance=data, schema=schema)
```

In the `/publish` handler (line ~562), drop the third argument:

```python
                    new_uuids = (write_publications(new, self.json_path)
                                 if new else [])
```

Delete the `schema_path = "publicationsSchema.json"` class attribute from `GalleryHandler` (line ~401).

Delete `GalleryHandler.schema_path = args.schema` from `serve()` (line ~652).

Delete the argparse line (line ~684): `parser.add_argument("--schema", default=str(PKG / "publicationsSchema.json"))`.

- [ ] **Step 4: Run the self-check to verify it passes**

Run: `python publish_next.py --selfcheck`
Expected: PASS — prints `selfcheck OK`.

- [ ] **Step 5: Confirm no dangling `schema` / `--schema` references remain**

Run: `rg -n "schema_path|args\.schema|--schema" publish_next.py`
Expected: no matches (the only remaining `schema` token is the local `schema` variable inside `write_publications` and the `SCHEMA_PATH` constant).

- [ ] **Step 6: Commit**

```bash
git add publish_next.py
git commit -m "refactor(publish-next): hardcode bundled schema, drop --schema flag"
```

---

### Task 2: "Add to publish pad" button + `/stage` endpoint

**Files:**
- Modify: `publish_next.py` (`_PAGE_TMPL` header + `<script>` blocks; `do_POST` line ~519)

**Interfaces:**
- Consumes: `write_publications(entries, json_path)` from Task 1; existing `self._json_body()`, `self._send()`, `self.json_path`.
- Produces: `POST /stage` → `200 {"staged": int, "uuids": [str, ...]}` on success, `400 {"error": str}` on empty/failed; does **not** set `GalleryHandler.publish_done` (session stays alive). Client `stageQueue()` function; `#stage-btn` and `#stage-status` DOM ids.

- [ ] **Step 1: Add the `/stage` endpoint**

In `do_POST`, add a branch after the `/publish` block (before the final `else` that 404s):

```python
            elif self.path == "/stage":
                data = self._json_body()
                entries = data.get("entries", []) or []
                if not entries:
                    self._send(400, {"error": "nothing to stage"}); return
                try:
                    uuids = write_publications(entries, self.json_path)
                except Exception as e:
                    self._send(400, {"error": f"schema/write failed: {e}"}); return
                # No publish_done set → serve loop keeps running, session stays alive.
                self._send(200, {"staged": len(uuids), "uuids": uuids})
```

- [ ] **Step 2: Add the header button and status span**

In `_PAGE_TMPL`, in the `<header>` (line ~231), add the button and a status span immediately before the existing Publish button:

```html
  <span id="stage-status"></span>
  <button id="stage-btn" onclick="stageQueue()" disabled>Add to publish pad</button>
  <button id="publish-btn" onclick="publishQueue()" disabled>Publish</button>
```

Reuse `#publish-btn` styling for the new button by extending the CSS selector (line ~197): change `#publish-btn {` to `#publish-btn, #stage-btn {` and `#publish-btn:disabled {` to `#publish-btn:disabled, #stage-btn:disabled {`.

- [ ] **Step 3: Enable the button only when something fresh is queued**

In `refreshCount()` (line ~271), add a line so the button reflects unstaged entries:

```javascript
function refreshCount() {
  document.getElementById("queue-count").textContent = queue.length + " queued";
  document.getElementById("publish-btn").disabled = queue.length === 0;
  document.getElementById("stage-btn").disabled = queue.filter(e => !e.uuid).length === 0;
}
```

- [ ] **Step 4: Implement `stageQueue()`**

Add this function to the `<script>` block, next to `publishQueue()` (line ~365):

```javascript
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
    e.uuid = d.uuids[i]; // promote to "existing" so a later Publish won't re-write it
    const card = document.getElementById(e.cardId);
    if (!card) return;
    const badge = card.querySelector(".badge");
    if (badge) badge.textContent = "on pad";
    const del = card.querySelector(".btn-del");
    if (del) del.disabled = true; // persisted row must not be deletable from disk here
  });
  const staged = queue.filter(e => e.uuid).length;
  document.getElementById("stage-status").textContent = staged + " on pad";
  refreshCount(); // fresh count now 0 → button disables itself
}
```

- [ ] **Step 5: Manual syntax check**

Run: `python -c "import ast, pathlib; ast.parse(pathlib.Path('publish_next.py').read_text())"`
Expected: no output (parses clean). Then `python publish_next.py --selfcheck` still prints `selfcheck OK`.

- [ ] **Step 6: Commit**

```bash
git add publish_next.py
git commit -m "feat(publish-next): Add to publish pad button, stages queue without publishing"
```

---

### Task 3: Live UI verification

**Files:** none (verification only).

**Data dir:** code lives in this repo (`publicator.py`); the publication database
(`picked/`, `publications.json`, `.deviantart-login/`, `.deviantart-session/`) lives in
the sibling **`/home/theta/.vault/repos/Art/`**. Pass it explicitly with `--data-dir`.
That `publications.json` is real and git-tracked, so staging writes real
`state=unpublished` rows into it — discard the test rows afterward with
`git -C /home/theta/.vault/repos/Art checkout publications.json` (do NOT commit them here).

- [ ] **Step 1: Drive the gallery with the playwright-cli skill**

Launch against the real data dir:

```bash
python publish_next.py --data-dir /home/theta/.vault/repos/Art --port 8765
```

(There are already images under `../Art/picked/`.) Then verify in the browser:
- `Add to publish pad` is disabled until a card is saved to the queue, then enables.
- Clicking it: the card badge flips to `on pad`, its Delete button disables, `#stage-status` shows `N on pad`, and the gallery tab stays open (server did not exit).
- `/home/theta/.vault/repos/Art/publications.json` now contains N new rows with `state` = unpublished (check on disk).
- Saving another fresh card re-enables the button; clicking Publish afterward reports the right counts without duplicating the staged rows in `publications.json`. (Publish drives DeviantArt for real — only run this bullet if you actually intend to post, otherwise stop after confirming the button re-enables.)

- [ ] **Step 2: Clean up and record the result**

Discard the test rows: `git -C /home/theta/.vault/repos/Art checkout publications.json`.
Note pass/fail per bullet in the session. No commit in this repo (verification only).

---

## Self-Review

**Spec coverage:**
- Drop `--schema`, hardcode bundled schema → Task 1 (constant + signature + all call-site deletions). ✓
- `write_publications(entries, json_path)` two-arg signature → Task 1 Step 3. ✓
- `/stage` endpoint, non-terminal → Task 2 Step 1. ✓
- Header button next to Publish, reused styling → Task 2 Step 2. ✓
- `stage-btn` disabled when nothing fresh queued → Task 2 Step 3. ✓
- `stageQueue()` zips uuids back, badge → `on pad`, disable Delete, `#stage-status` → Task 2 Step 4. ✓
- Later Publish sends staged as existing, no double-write → covered by uuid promotion (Task 2 Step 4) + existing `/publish` split; verified in Task 3 Step 1. ✓
- Self-check on new signature asserting `state=STATE_UNPUBLISHED` + returned uuid → Task 1 Step 1. ✓
- Live UI verification → Task 3. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases"; every code step shows real code. ✓

**Type consistency:** `write_publications(entries, json_path) -> list[str]` used identically in Task 1 (definition, `/publish` call) and Task 2 (`/stage` call). `#stage-btn`, `#stage-status`, `stageQueue()`, `d.uuids`/`d.staged` consistent across Task 2 steps and Task 3. Badge text `on pad` and status `N on pad` consistent. ✓
