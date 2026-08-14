# Standard Python project layout + `publish_next.py` split

**Date:** 2026-08-14
**Status:** approved, ready for implementation

## Problem

Every module sits at the repo root as a loose script, and `publish_next.py` has
grown to 1046 lines carrying five unrelated jobs: candidate discovery, schedule
cadence math, the served HTML page, the HTTP handler, and the publications.json
writes. A file that large is hard to read, hard to test in isolation (the page
test has to call `GalleryHandler._build_page(H)` with the class as `self` to
avoid constructing a socket-backed handler), and hard to edit reliably.

## Goal

A standard `src/` layout with a real `publicator` package and a `tests/` tree,
with `publish_next.py` split along its existing seams. No behaviour change: the
`nix run` app names, their CLI flags, and the DeviantArt publish flow stay
exactly as they are.

## Non-goals

- Repackaging as an installable application (no `buildPythonApplication`, no
  console-script entry points). The flake keeps running modules directly.
- Splitting `deviantart.py` (ex-`da_publish.py`) internally. It moves and is
  renamed; its 619 lines are left alone.
- Any change to the DeviantArt submit steps, the `STEPS` ↔ SKILL.md contract, or
  the scheduling semantics (server-side, TZ-anchored, browser reads no clock).

## Target tree

```
publicator.py/
├── pyproject.toml
├── flake.nix
├── CLAUDE.md · AGENTS.md
├── .claude/skills/publish-deviantart/SKILL.md
├── src/publicator/
│   ├── __init__.py             setup_logging
│   ├── config.py               ← validate.py
│   ├── publicationsSchema.json ← moved (resolved via __file__)
│   ├── entries.py              ← echo_first_unpublished_publication_data.py, + STATE_* constants
│   ├── images.py               ← publish_next.py
│   ├── scheduling.py           ← publish_next.py
│   ├── store.py                ← publish_next.py + atomic_write_json from da_publish.py
│   ├── deviantart.py           ← da_publish.py (internals untouched)
│   ├── llm_meta.py             ← unchanged
│   ├── webui/
│   │   ├── __init__.py
│   │   ├── page.py             ← publish_next.py (template + rendering)
│   │   └── server.py           ← publish_next.py (GalleryHandler + serve)
│   └── apps/
│       ├── __init__.py
│       ├── publish_next.py     argparse main() only
│       ├── da_publish.py
│       ├── validate.py
│       ├── echo_first.py
│       └── pw_daemon.py
└── tests/
    ├── test_scheduling.py
    ├── test_store.py
    ├── test_page.py
    ├── test_config.py
    ├── test_deviantart.py
    └── test_llm_meta.py
```

## Module contents

| Module | Holds |
|---|---|
| `__init__.py` | `setup_logging(verbose)` (was `da_publish.py:35`) |
| `config.py` | `SCHEMA`, `load_config`, `validate_publications` |
| `entries.py` | `DATA_DIR`, `set_data_dir`, `pubs_file`, `load_pubs`, `first_unpublished`, `deviantart_apparition`, `find_art_path`, `format_schedule`, `STATE_UNPUBLISHED`, `STATE_PUBLISHED` |
| `images.py` | `IMAGE_EXTENSIONS`, `MIME`, `guess_mime`, `compute_sha512`, `load_publicated_hashes`, `collect_images`, `find_candidates`, `generate_thumbnails` |
| `scheduling.py` | `DEFAULT_TZ`, `SLOT_HORIZON_WEEKS`, `WEEKDAY`, `zone`, `schedule_profiles`, `profile_slots`, `schedule_data`, `ts_labels`, `existing_ts` |
| `store.py` | `atomic_write_json`, `resolve_ts`, `write_publications`, `apply_update`, `set_or_pop` |
| `deviantart.py` | everything from `da_publish.py` except `main()`, `_selfcheck()` and `_atomic_write_json` |
| `webui/page.py` | `PAGE_TEMPLATE`, `tier_gallery_fields`, `preset_options`, `card_form_html`, `render_page` |
| `webui/server.py` | `GalleryHandler`, `serve` |
| `apps/*.py` | argparse + `main()` for each entrypoint |

## Import graph (one-way, no cycles)

```
config   entries   images   llm_meta        (leaves, stdlib + jsonschema only)
   \        |  \      /
    \       |   \    /
     `----> store <-'            store: config + entries + images
            ^   ^
 scheduling-'   |                scheduling: entries (STATE_UNPUBLISHED only)
                |
        deviantart                deviantart: store + entries + config
                |
        webui.page                page: scheduling
                |
        webui.server               server: page + store + scheduling + images
                |                          + llm_meta + deviantart
            apps.*
```

`STATE_UNPUBLISHED` / `STATE_PUBLISHED` move to `entries.py` so `scheduling.py`
does not have to import the Playwright-carrying module for a string constant.

## Naming

Names that cross a module boundary become public (leading underscore dropped):
`_schedule_data` → `schedule_data`, `_existing_ts` → `existing_ts`, `_zone` →
`zone`, `_ts_labels` → `ts_labels`, `_schedule_profiles` → `schedule_profiles`,
`_profile_slots` → `profile_slots`, `_PAGE_TMPL` → `PAGE_TEMPLATE`,
`_atomic_write_json` → `atomic_write_json`, `_resolve_ts` → `resolve_ts`,
`_set_or_pop` → `set_or_pop`, `_tier_gallery_fields` → `tier_gallery_fields`,
`_preset_options` → `preset_options`, `_card_form_html` → `card_form_html`.
Module-private helpers keep their underscore.

## The one boundary change

`GalleryHandler._build_page` becomes a pure function in `page.py`:

```python
def render_page(*, thumb_dir, thumb_map, candidates, pending, existing_ts,
                schedules, config, ai_model, openrouter_model) -> str
```

`GalleryHandler` keeps its class attributes exactly as today; its `_build_page`
shrinks to an adapter that forwards them. This removes the need for the
`GalleryHandler._build_page(H)` class-as-`self` trick in the page tests, and
lets `page.py` be tested without a socket. Everything else is code motion.

## Flake

App names and flags are unchanged. Each app's shell text becomes:

```
export PYTHONPATH=${src}/src
exec python -m publicator.apps.<module> "$@"
```

`check-steps` runs `python -m publicator.apps.da_publish --check-steps`. The
devShell exports the same `PYTHONPATH` so `python -m publicator.apps.X` works
from a checkout. One app is **added**: `echo-first` (→ `publicator.apps.echo_first`),
because `SKILL.md` currently invokes `python ./echo_first_unpublished_publication_data.py`
and that path disappears with the move.

`deviantart.py` resolves the skill file from the repo root via
`Path(__file__).resolve().parents[2]` instead of its own directory.

## pyproject.toml

Metadata only — nothing installs it. `[project]` (name, version,
`requires-python = ">=3.12"`, dependencies for documentation value),
`[build-system]` setuptools, `[tool.setuptools.package-data]` carrying
`publicationsSchema.json`, and `[tool.pytest.ini_options]` with
`pythonpath = ["src"]` + `testpaths = ["tests"]` so pytest imports the package
without an install step.

## Tests

The three `_selfcheck()` functions and their `--selfcheck` flags are removed;
their assertions become pytest tests. `--check-steps` is untouched (it has a
real flake app). Coverage after the split is a superset of today's:

| File | Covers |
|---|---|
| `test_scheduling.py` | profile validation + 4 rejection cases, 52-slot 20:00-Paris invariant across DST, `existing_ts` filtering, the Node-driven client-scheduler regression test |
| `test_store.py` | `write_publications` round-trip, missing-schedule rejection, `apply_update` set/clear of price/tier/galleries, custom-time TZ resolution on both write paths |
| `test_page.py` | both AI models rendered, config tiers/galleries, schedule presets + injected `SCHEDULES`/`LABELS`, no unsubstituted placeholders, `/original` query decoding |
| `test_config.py` | `load_config` defaults, flat config, profiles round-trip, tier/gallery allow-list enforcement |
| `test_deviantart.py` | `parse_schedule` cases, `STEPS` callables + skill sync, premium/tier/gallery no-op guards, `TAGS_FILE` resolution, empty cookie DB |
| `test_llm_meta.py` | unchanged |

## Execution

Two phases, each independently green, so a regression bisects to one phase.

1. **Relocate.** Create `src/publicator/` + `tests/`, `git mv` every module
   (with its rename), move the schema, add `pyproject.toml`, rewrite imports,
   update the flake and docs. `publish_next.py` moves whole, still 1046 lines,
   self-checks still present. Gate: `pytest` green, apps run.
2. **Carve.** Split `publish_next.py` into `images`/`scheduling`/`store`/
   `webui.page`/`webui.server`/`apps.publish_next`, introduce `render_page`,
   convert the self-checks to tests and delete the flags. Gate: `pytest` green,
   apps run, gallery renders in a real browser.

## Verification

Baseline (recorded before any change): `pytest` **13 passed**; `publish_next`,
`da_publish`, `validate` self-checks and `check-steps` all OK.

After each phase:

- `nix develop -c pytest -q`
- `nix run .#check-steps` → 12 steps in sync
- `nix run .#validate -- --data-dir <tmp>` → `Valid`
- `nix run .#da-publish -- --data-dir <tmp>` → `nothing to publish`
- `nix run .#publish-next -- --data-dir <tmp>` (one image) → server binds,
  gallery HTML served
- phase 2 only: load that gallery in a real browser (playwright-cli), confirm
  the card renders and the schedule preset pre-fills the picker

## Risks

| Risk | Mitigation |
|---|---|
| `publicationsSchema.json` not found after the move | `SCHEMA` resolves via `__file__`; `test_config.py` loads it, and `nix run .#validate` proves it inside the nix store |
| `SKILL.md` unreachable from the new depth → silent `check-steps` skip | `check_steps()` prints "not found (skipping)"; the verification step asserts the "12 steps in sync" line instead of a bare exit code |
| `PYTHONPATH` clobbered in the devShell | append form: `PYTHONPATH="$PWD/src${PYTHONPATH:+:$PYTHONPATH}"` |
| Losing a self-check assertion during the conversion | phase 2 converts them file-by-file; the spec table above is the checklist |
| Stale references in docs/skill | grep for every old module name at the end; `SKILL.md` gets the new `echo-first` app |
