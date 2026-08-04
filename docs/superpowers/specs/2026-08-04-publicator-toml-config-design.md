# Rename config.toml → publicator.toml + externalize `../Art`'s domain

## Problem

`config.toml` only carried DeviantArt tier/gallery allow-lists, but several
facts that belong to the **data domain** (the sibling `../Art` repo) are still
hardcoded in the tooling:

- the source image dir name `picked` (`publish_next.py`), overridable only via
  a `--picked-dir` flag;
- the DeviantArt tag list `tags/da.txt`, loaded from the **code package**
  (`da_publish.py:53`, `PKG / "tags/da.txt"`);
- the publishing cadence "Tuesday 20:00 UTC, 2 per slot", hardcoded **twice** —
  once in Python (`_next_tuesday_8pm_after`/`compute_next_slot`) and once in the
  client JS (`nextTuesday8pmUTC`/`slotForIndex`).

Rename the file to `publicator.toml` and make it the single home for these
domain/policy values.

### Why the cadence was in two places

A server can't call a browser function, so each side reimplemented "next
Tuesday 8pm":

- **Client JS** prefills the `datetime-local` input as cards are added
  (`publish_next.py:329`) — pure UI convenience.
- **Python** `compute_next_slot` is a *fallback* in `write_publications`
  (`:745`): `e.get("scheduleTs") or compute_next_slot(...)`.

But the form **requires** a schedule (`readForm:382`), so every entry always
carries `scheduleTs` — the Python fallback is dead on the real path. Resolution:
the cadence lives in config, **only the client** consumes it, and the server
requires `scheduleTs`.

## Architecture

`load_config` (in `validate.py`) becomes the single loader for `publicator.toml`,
returning tiers/galleries **plus** `publicable`, `tags`, and `schedule`.
`validate_publications` still reads only tiers/galleries. `publish_next` consumes
`publicable` + `schedule`; `da_publish` consumes `tags`. Paths resolve relative
to `--data-dir` (as `picked`/`publications.json` already do); the config file
itself is still read from CWD (in practice CWD == data-dir == `../Art`).

## `publicator.toml` layout (lives in `../Art/`)

```toml
publicable = ["picked"]          # dirs scanned for candidates, relative to data-dir
tags = "tags/da.txt"             # DA tag list, relative to data-dir

[deviantart]                     # unchanged
tiers = ["Exclusive Content"]
galleries = ["Featured"]

[schedule]
frequency = "weekly"             # only "weekly" implemented; other values raise
day = "tuesday"
hour = 20                        # UTC, 24h
per_slot = 2                     # posts packed per slot
```

## Changes

### A. Rename (in `../Art`, separate repo)
- `git mv config.toml publicator.toml`, add the new keys above.
- The whole `publicator.py/tags/` dir (`da.txt`, `da_vr.txt`, `pixiv.txt`) moves
  to `../Art/tags/` — it is account content, not tooling.

### B. `validate.py`
- `load_config` reads `publicator.toml`; returns the enriched dict
  (`tiers`, `galleries`, `publicable`, `tags`, `schedule`).
- Missing file → all-empty (`tags` → `None`, `schedule` → `{}`), unchanged
  tolerance.
- Error strings in `validate_publications` say `publicator.toml`.

### C. `publish_next.py` — publicable dirs
- Remove `--picked-dir` (arg + default + `find_candidates` call).
- `find_candidates(dirs: list[str], json_path, limit)` — collect images across
  all dirs, shuffle, take `limit`. Empty list → `[]`.
- `main` loads config once, resolves `[data_dir / d for d in config["publicable"]]`,
  passes config through to `serve` (instead of `serve` re-loading it).

### D. `da_publish.py` — tags path
- Delete `TAGS_FILE = PKG / "tags/da.txt"` (module level).
- `configure(data_dir)` loads config (`load_config(Path.cwd())`) and sets
  `TAGS_FILE = DATA_DIR / config["tags"]` — single resolution point, since
  `configure` already derives data-dir paths. `PKG` stays (used by `SKILL_MD`).
- `tags` unset in config → `TAGS_FILE` unset → tag steps fail with a clear error.

### E. `publish_next.py` — schedule (client-only), delete Python cadence
- Read `[schedule]`; assert `frequency == "weekly"` else `NotImplementedError`
  (reserved for monthly/etc.). Map `day` name → JS `getUTCDay` index.
- Inject `__SCHEDULE__` (JSON: `{day, hour, per_slot}`) into the page next to
  `__INITIAL_MAX_TS__`.
- JS: `nextTuesday8pmUTC`/`slotForIndex` → generic, driven by injected
  `SCHEDULE` (rename to `nextSlotBase`/`slotForIndex`).
- **Delete** `_next_tuesday_8pm_after` and `compute_next_slot`.
- `write_publications:745` → `ts = int(e["scheduleTs"])`; raise `ValueError` if
  `scheduleTs` missing (`da_publish`/`echo` read the timestamp from JSON, so are
  unaffected).

## Testing

- `publish_next._selfcheck`: replace `compute_next_slot` calls with literal
  Tue-8pm anchor timestamps; drop the weekday/hour asserts (now client concern);
  keep the `write_publications`/`apply_update` round-trip and `/original` path
  checks. Add a check that `write_publications` raises when `scheduleTs` is absent.
- `validate.py` runs green against the renamed `../Art/publicator.toml`.
- Manual browser check: gallery still defaults the schedule input to the next
  Tuesday 20:00 slot from the injected `[schedule]`; candidates appear from the
  `publicable` dirs; a publish/stage writes with the client-supplied `scheduleTs`.

## Out of scope

- Implementing `monthly`/other frequencies (structure reserved; value raises).
- Per-account/platform tag selection (`da_vr.txt`/`pixiv.txt` move but stay
  unreferenced by code).
- Making `publicable`/`tags` absolute-path capable (data-dir-relative only).
