# publicator.toml config + domain externalization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename `config.toml` → `publicator.toml` and make it the single home for the publicable dirs, the DeviantArt tags path, and the publishing cadence — removing the duplicated Tuesday-8pm slot math.

**Architecture:** `validate.load_config` becomes the one loader, returning `tiers/galleries/publicable/tags/schedule`. `publish_next` consumes `publicable` + `schedule` (schedule injected into the browser client only); `da_publish` consumes `tags`. Paths resolve relative to `--data-dir`; the config file is read from CWD (in practice CWD == data-dir == `../Art`).

**Tech Stack:** Python 3.11+ (stdlib `tomllib`), stdlib `http.server`, `jsonschema`. No new dependencies.

## Global Constraints

- No new dependencies; `tomllib` (stdlib, Python 3.11+) only.
- Tests use the repo's **assert-based selfcheck idiom** (`_selfcheck()` run via `--selfcheck` or `__main__`), not pytest. There is no pytest harness in this repo.
- `publicable` and `tags` paths resolve **relative to `--data-dir`**; the config file itself is read from **CWD** (`load_config(Path.cwd())`) — unchanged.
- Missing `publicator.toml` → all-empty defaults (`tags` → `None`, `schedule` → `{}`, lists → `[]`). Never crash on a missing file.
- Only `schedule.frequency = "weekly"` is implemented; any other value raises `NotImplementedError`.
- Conventional Commits. End every commit message with:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- `../Art` is a **separate git repo**. Edits to `../Art/publicator.toml` and `../Art/tags/` are committed inside `../Art`, not in this repo.

---

### Task 1: `load_config` reads `publicator.toml` and returns the enriched dict

**Files:**
- Modify: `validate.py:12-19` (`load_config`), `:31-35` (error strings), `:38-49` (`main` — add `--selfcheck`)
- Modify (separate repo): `../Art/config.toml` → `../Art/publicator.toml` (rename only)

**Interfaces:**
- Produces: `load_config(cwd=".") -> {"tiers": list[str], "galleries": list[str], "publicable": list[str], "tags": str | None, "schedule": dict}`

- [ ] **Step 1: Add the failing selfcheck.** Insert this function above `main()` in `validate.py`:

```python
def _selfcheck():
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        # missing file -> all-empty defaults
        assert load_config(d) == {
            "tiers": [], "galleries": [], "publicable": [], "tags": None, "schedule": {}
        }, load_config(d)
        (Path(d) / "publicator.toml").write_text(
            'publicable = ["picked"]\n'
            'tags = "tags/da.txt"\n'
            '[deviantart]\ntiers = ["T"]\ngalleries = ["G"]\n'
            '[schedule]\nfrequency = "weekly"\nday = "tuesday"\nhour = 20\nper_slot = 2\n'
        )
        c = load_config(d)
        assert c["tiers"] == ["T"] and c["galleries"] == ["G"], c
        assert c["publicable"] == ["picked"], c
        assert c["tags"] == "tags/da.txt", c
        assert c["schedule"] == {
            "frequency": "weekly", "day": "tuesday", "hour": 20, "per_slot": 2
        }, c
    print("validate selfcheck OK")
```

  Wire it into `main()` — add the arg and an early exit. Replace the body of `main()`:

```python
def main():
    ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
    ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
    ap.add_argument("--selfcheck", action="store_true", help="run offline self-checks, then exit")
    args = ap.parse_args()
    if args.selfcheck:
        _selfcheck(); return
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    data = json.loads((data_dir / "publications.json").read_text())
    validate_publications(data, load_config(Path.cwd()))
    print("Valid")
```

- [ ] **Step 2: Run it to confirm it fails.**

Run: `python validate.py --selfcheck`
Expected: `AssertionError` (current `load_config` returns only `tiers`/`galleries` and reads `config.toml`).

- [ ] **Step 3: Rewrite `load_config`** (`validate.py:12-19`):

```python
def load_config(cwd="."):
    """publicator.toml config (read from <cwd>). Missing file -> empty defaults.
    Returns tiers/galleries (DA allow-lists), publicable (candidate dirs),
    tags (DA tag-list path), schedule (cadence for the gallery client)."""
    f = Path(cwd) / "publicator.toml"
    cfg = tomllib.loads(f.read_text()) if f.exists() else {}
    da = cfg.get("deviantart", {})
    return {
        "tiers": list(da.get("tiers", [])),
        "galleries": list(da.get("galleries", [])),
        "publicable": list(cfg.get("publicable", [])),
        "tags": cfg.get("tags"),
        "schedule": dict(cfg.get("schedule", {})),
    }
```

  And update the two error strings in `validate_publications` (`:32`, `:35`): change `not in config.toml` → `not in publicator.toml`.

- [ ] **Step 4: Run the selfcheck to confirm it passes.**

Run: `python validate.py --selfcheck`
Expected: `validate selfcheck OK`

- [ ] **Step 5: Rename the live config file** (separate repo):

```bash
cd ../Art && git mv config.toml publicator.toml && cd -
```

- [ ] **Step 6: Verify live validation still passes** (existing tiers/galleries preserved):

Run: `python validate.py --data-dir ../Art`
Expected: `Valid`

- [ ] **Step 7: Commit** (this repo, then `../Art`):

```bash
git add validate.py
git commit -m "refactor(config): load_config reads publicator.toml + enriched dict

Returns publicable/tags/schedule alongside tiers/galleries; adds a --selfcheck.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"

cd ../Art && git commit -am "chore: rename config.toml -> publicator.toml

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>" && cd -
```

---

### Task 2: `publicable` dirs list in `publish_next` (drop `--picked-dir`)

**Files:**
- Modify: `publish_next.py:96-109` (`find_candidates`), `:803-815` (`serve`), `:814` (config load), `:839-866` (`main`), `:880` (serve call)
- Modify (separate repo): `../Art/publicator.toml` (add `publicable`)

**Interfaces:**
- Consumes: `load_config(...)["publicable"]` (Task 1)
- Produces: `find_candidates(directories: list[str], json_path: str, limit: int) -> list[str]`; `serve(thumb_dir, thumb_map, candidate_paths, pending, args, config) -> dict | None`

- [ ] **Step 1: Change `find_candidates` to take a list of dirs** (`publish_next.py:96-109`):

```python
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
```

- [ ] **Step 2: Thread config into `serve`** (`publish_next.py:803-815`). Change the signature and the config line:

```python
def serve(thumb_dir: str, thumb_map: dict[str, str], candidate_paths: list[str],
          pending: list[dict], args, config: dict) -> dict | None:
    GalleryHandler.thumb_dir = thumb_dir
    GalleryHandler.thumb_map = thumb_map
    GalleryHandler.candidate_paths = candidate_paths
    GalleryHandler.pending = pending
    GalleryHandler.initial_max_ts = _max_existing_ts(args.json)
    GalleryHandler.ai_model = args.ai_model
    GalleryHandler.openrouter_model = args.openrouter_model
    GalleryHandler.ai_timeout = args.ai_timeout
    GalleryHandler.json_path = args.json
    GalleryHandler.config = config          # was: load_config(Path.cwd())
    GalleryHandler.publish_done = None
```

- [ ] **Step 3: Update `main`** — remove `--picked-dir`, load config once, resolve dirs, pass config to `serve`.

  Delete the arg line (`publish_next.py:844`):
  ```python
  parser.add_argument("--picked-dir", default=None, help="default: <data-dir>/picked")
  ```
  Replace lines `:861-866` with:
```python
    data_dir = configure(args.data_dir)  # points da_publish + echo helpers at the db dir
    args.json = args.json or str(data_dir / "publications.json")
    config = load_config(Path.cwd())
    publicable_dirs = [str(data_dir / d) for d in config["publicable"]]

    print("Finding unpublished images...")
    candidates = find_candidates(publicable_dirs, args.json, args.n)
```
  Change the `serve` call (`:880`) to pass `config`:
```python
        result = serve(thumb_dir, thumb_map, candidates, pending, args, config)
```

- [ ] **Step 4: Add `publicable` to the live config** (separate repo). Add to the top of `../Art/publicator.toml`:

```toml
publicable = ["picked"]
```

- [ ] **Step 5: Verify candidates are discovered.**

Run: `python publish_next.py --data-dir ../Art -n 3 --port 8799` then Ctrl-C after the "Found N new pick(s)" line prints (or "Nothing to publish" if the queue is drained).
Expected: it reports candidates found from `../Art/picked` (same count as before the change); no `--picked-dir` in `--help`.

- [ ] **Step 6: Commit** (both repos):

```bash
git add publish_next.py
git commit -m "feat(publish): scan publicable dirs from publicator.toml, drop --picked-dir

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"

cd ../Art && git commit -am "chore: add publicable dirs to publicator.toml

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>" && cd -
```

---

### Task 3: `tags` path in `da_publish` + move `tags/` into `../Art`

**Files:**
- Modify: `da_publish.py:45-54` (imports + constants), `:70-77` (`configure`), `:298-299` (`_step_add_tags` guard), `:509-514` (`_selfcheck`)
- Move: `tags/` (this repo) → `../Art/tags/`
- Modify (separate repo): `../Art/publicator.toml` (add `tags`)

**Interfaces:**
- Consumes: `load_config(...)["tags"]` (Task 1); `DATA_DIR` (module global, set by `configure`)
- Produces: module global `TAGS_FILE: Path | None` resolved as `DATA_DIR / config["tags"]`

- [ ] **Step 1: Add `load_config` import** to `da_publish.py`. After the `echo_first_unpublished_publication_data` import block (ends `:50`), add:

```python
from validate import load_config
```

- [ ] **Step 2: Replace the module-level tags constant** (`da_publish.py:52-54`):

```python
PKG = Path(__file__).resolve().parent  # code assets (SKILL.md) travel with the package
TAGS_FILE = None                       # resolved by configure() from publicator.toml [tags], under DATA_DIR
SKILL_MD = PKG / ".claude/skills/publish-deviantart/SKILL.md"
```

- [ ] **Step 3: Resolve `TAGS_FILE` in `configure`** (`da_publish.py:70-77`):

```python
def configure(data_dir) -> Path:
    """Point runtime state at the publication database dir.
    Returns the resolved DATA_DIR."""
    global DATA_DIR, LOGIN_DIR, SESSION_DIR, TAGS_FILE
    DATA_DIR = set_data_dir(data_dir)  # also points the echo helpers (find_art_path) at it
    LOGIN_DIR = DATA_DIR / ".deviantart-login"
    SESSION_DIR = DATA_DIR / ".deviantart-session"
    tags = load_config(Path.cwd()).get("tags")
    TAGS_FILE = DATA_DIR / tags if tags else None
    return DATA_DIR
```

- [ ] **Step 4: Guard `_step_add_tags`** (`da_publish.py:298-299`). Insert at the top of the function, before the `tags = [...]` line:

```python
    if TAGS_FILE is None:
        raise RuntimeError("no 'tags' path configured in publicator.toml")
    tags = [t.strip() for t in TAGS_FILE.read_text().splitlines() if t.strip()]
```

- [ ] **Step 5: Add the failing selfcheck.** Append to `da_publish._selfcheck` (after the existing asserts, before its end):

```python
    # TAGS_FILE is resolved from publicator.toml [tags], under DATA_DIR
    import tempfile, os as _os
    _cwd0 = _os.getcwd()
    with tempfile.TemporaryDirectory() as _d:
        _os.chdir(_d)
        try:
            (Path(_d) / "publicator.toml").write_text('tags = "tags/da.txt"\n')
            configure(_d)
            assert TAGS_FILE == Path(_d) / "tags/da.txt", TAGS_FILE
            (Path(_d) / "publicator.toml").write_text("")   # no [tags] key
            configure(_d)
            assert TAGS_FILE is None, TAGS_FILE
        finally:
            _os.chdir(_cwd0)
    print("da_publish selfcheck OK")
```

- [ ] **Step 6: Run it to confirm it fails.**

Run: `python da_publish.py --selfcheck`
Expected: fails — either `NameError`/`ImportError` before Steps 1-3 land, or (with them applied) passes. Run *before* applying Steps 1-4 to see the failure, then apply and re-run.

  (If executing top-to-bottom, Steps 1-4 are already applied; in that case Step 6 confirms the new selfcheck **passes** and Step 7 is the fail/pass gate for the move.)

- [ ] **Step 7: Run the selfcheck to confirm it passes.**

Run: `python da_publish.py --selfcheck`
Expected: `da_publish selfcheck OK`

- [ ] **Step 8: Move the `tags/` dir into `../Art`.**

```bash
mkdir -p ../Art/tags
git mv tags/da.txt tags/da_vr.txt tags/pixiv.txt ../Art/tags/ 2>/dev/null \
  || { mv tags/*.txt ../Art/tags/ && rmdir tags && git add -A; }
```

  `git mv` will refuse across repos, so the fallback `mv` + `git add -A` records the deletion here; the additions are committed in `../Art` (Step 10).

- [ ] **Step 9: Add `tags` to the live config** (separate repo). Add to `../Art/publicator.toml`:

```toml
tags = "tags/da.txt"
```

- [ ] **Step 10: Verify the tags file resolves against the live data-dir.**

Run:
```bash
python -c "import da_publish; p=da_publish.configure('../Art'); print(da_publish.TAGS_FILE, da_publish.TAGS_FILE.exists())"
```
Expected: prints `.../Art/tags/da.txt True`.

- [ ] **Step 11: Commit** (both repos):

```bash
git add da_publish.py
git commit -m "feat(publish): resolve DA tags path from publicator.toml; move tags/ to ../Art

tags/ (da.txt, da_vr.txt, pixiv.txt) is account content, not tooling; it now
lives in the ../Art data domain and is pointed at by publicator.toml [tags].

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"

cd ../Art && git add tags/ publicator.toml && git commit -m "chore: add tags/ + tags path to publicator.toml

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>" && cd -
```

---

### Task 4: `[schedule]` client-only; delete the Python cadence duplication

**Files:**
- Modify: `publish_next.py:32` (import), `:130-148` (delete slot fns, add `_schedule_js`), `:166-213` (`_selfcheck`), `:281` + `:286-297` (JS template), `:505-516` (handler class attr), `:568-574` (template replace), `:803-815` (`serve` sets schedule), `:738`/`:745` (`write_publications`)
- Modify (separate repo): `../Art/publicator.toml` (add `[schedule]`)

**Interfaces:**
- Consumes: `load_config(...)["schedule"]` (Task 1); `config` passed to `serve` (Task 2)
- Produces: `_schedule_js(schedule: dict) -> {"day": int, "hour": int, "per_slot": int}` (`day` is a JS `getUTCDay` index); injected page constant `SCHEDULE`

- [ ] **Step 1: Delete the Python slot math and add `_schedule_js`.** Replace `publish_next.py:130-148` (the `_next_tuesday_8pm_after` + `compute_next_slot` block, keeping the section header comment) with:

```python
# ---------------------------------------------------------------------------
# Schedule cadence — consumed by the browser client only (see _build_page).
# ---------------------------------------------------------------------------

_JS_DAY = {"sunday": 0, "monday": 1, "tuesday": 2, "wednesday": 3,
           "thursday": 4, "friday": 5, "saturday": 6}


def _schedule_js(schedule: dict) -> dict:
    """Cadence for the gallery client: {day: <JS getUTCDay index>, hour, per_slot}.
    Only frequency='weekly' is implemented."""
    freq = schedule.get("frequency", "weekly")
    if freq != "weekly":
        raise NotImplementedError(f"schedule.frequency {freq!r} not implemented (only 'weekly')")
    day = str(schedule.get("day", "tuesday")).lower()
    if day not in _JS_DAY:
        raise ValueError(f"schedule.day {day!r} invalid")
    return {"day": _JS_DAY[day], "hour": int(schedule.get("hour", 20)),
            "per_slot": int(schedule.get("per_slot", 2))}
```

- [ ] **Step 2: Trim the now-unused import** (`publish_next.py:32`):

```python
from datetime import datetime, timezone
```

- [ ] **Step 3: Require `scheduleTs` in `write_publications`.** In `publish_next.py`, delete line `:738` (`max_ts = _max_existing_ts(json_path)` — it fed only the deleted fallback) and replace line `:745`:

```python
        if not e.get("scheduleTs"):
            raise ValueError("scheduleTs required")
        ts = int(e["scheduleTs"])
```

- [ ] **Step 4: Add the `SCHEDULE` constant to the JS template.** After `publish_next.py:281` (`const INITIAL_MAX_TS = __INITIAL_MAX_TS__;`) add:

```javascript
const SCHEDULE = __SCHEDULE__;
```

  Replace the two slot functions in the template (`publish_next.py:286-297`):

```javascript
function nextSlotBase(afterTs) {
  const d = new Date(afterTs * 1000);
  let days = (SCHEDULE.day - d.getUTCDay() + 7) % 7;
  if (days === 0) days = 7;
  d.setUTCDate(d.getUTCDate() + days);
  d.setUTCHours(SCHEDULE.hour, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}
function slotForIndex(maxTs, idx) {
  const base = nextSlotBase(maxTs);
  return base + Math.floor(idx / SCHEDULE.per_slot) * 7 * 24 * 3600;
}
```

- [ ] **Step 5: Inject the schedule.** Add a class attribute to `GalleryHandler` (near `publish_next.py:515`, alongside `config`):

```python
    schedule: dict = {"day": 2, "hour": 20, "per_slot": 2}
```

  Add the template replacement in `_build_page` (after `publish_next.py:571`):

```python
        page = page.replace("__SCHEDULE__", json.dumps(self.schedule))
```

  Set it in `serve` (after the `GalleryHandler.config = config` line from Task 2):

```python
    GalleryHandler.schedule = _schedule_js(config["schedule"])
```

- [ ] **Step 6: Rewrite `_selfcheck`.** Replace `publish_next.py:166-213` (from `def _selfcheck` through the final `apply_update` assert block) with:

```python
def _selfcheck() -> None:
    # schedule cadence for the client: weekly maps a day-name -> JS getUTCDay index
    sj = _schedule_js({"frequency": "weekly", "day": "tuesday", "hour": 20, "per_slot": 2})
    assert sj == {"day": 2, "hour": 20, "per_slot": 2}, sj
    try:
        _schedule_js({"frequency": "monthly"}); assert False, "monthly not rejected"
    except NotImplementedError:
        pass
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
```

- [ ] **Step 7: Run the selfcheck to confirm it passes.**

Run: `python publish_next.py --selfcheck`
Expected: `publish_next selfcheck OK`

- [ ] **Step 8: Add `[schedule]` to the live config** (separate repo). Append to `../Art/publicator.toml`:

```toml
[schedule]
frequency = "weekly"
day = "tuesday"
hour = 20
per_slot = 2
```

- [ ] **Step 9: Browser verification.** Start the gallery and confirm the schedule input defaults to the next Tuesday 20:00 (UTC, shown in local time) as cards are added:

Run: `python publish_next.py --data-dir ../Art -n 3 --port 8799`
Expected: opening a candidate's form pre-fills **Schedule** with the next Tuesday-20:00 slot; adding a second card keeps the same slot; a third rolls a week (per_slot=2). Publishing/staging writes with the client-supplied `scheduleTs`. Ctrl-C to stop.

- [ ] **Step 10: Commit** (both repos):

```bash
git add publish_next.py
git commit -m "refactor(schedule): cadence lives in publicator.toml, consumed by the client only

Deletes the duplicated Python Tuesday-8pm slot math; the browser is the sole
default source. write_publications now requires scheduleTs. Only weekly is
implemented (others raise).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"

cd ../Art && git commit -am "chore: add [schedule] cadence to publicator.toml

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>" && cd -
```

---

## Self-Review

**Spec coverage:**
- Rename `config.toml` → `publicator.toml` — Task 1 (code + live file).
- `load_config` enriched loader — Task 1.
- `publicable` list, drop `--picked-dir` — Task 2.
- `tags` path + move `tags/` to `../Art` — Task 3.
- `[schedule]` client-only, delete Python cadence, require `scheduleTs`, weekly-only-raises — Task 4.
- Testing (assert selfchecks + browser check) — each task's steps.
- Out-of-scope items (monthly, per-account tags, absolute paths) — untouched, as specified.

All spec sections map to a task.

**Placeholder scan:** No TBD/TODO; every code step carries full code.

**Type consistency:** `find_candidates(directories: list[str], ...)` (Task 2) matches the `main` call; `serve(..., config)` signature (Task 2) matches the `serve` call and Task 4's `GalleryHandler.schedule` set inside it; `_schedule_js` return shape `{day, hour, per_slot}` matches the JS `SCHEDULE.day/hour/per_slot` usage; `TAGS_FILE` (`Path | None`) consistent across Task 3 steps.

## Notes for the executor

- Run all three selfchecks green before the browser check: `python validate.py --selfcheck`, `python da_publish.py --selfcheck`, `python publish_next.py --selfcheck`.
- Selfchecks and manual runs need the nix dev env (`jsonschema`, `firefox`) — run inside `nix develop` if imports fail.
- `../Art` commits land on `../Art`'s current branch; this repo's work stays on `feature/publicator-toml-config`.
