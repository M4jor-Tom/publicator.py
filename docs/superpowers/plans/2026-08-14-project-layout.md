# Standard project layout + `publish_next.py` split — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move every module into a standard `src/publicator/` package with a `tests/` tree, and split the 1046-line `publish_next.py` along its existing seams — with zero behaviour change to the `nix run` apps.

**Architecture:** Task 1 relocates everything (modules renamed into the package, `main()`s extracted to `publicator/apps/`, `--selfcheck` blocks converted to pytest files that already sit in their final shape). Tasks 2–6 then carve `publicator/publish_next.py` into `images` / `scheduling` / `store` / `webui.page` / `webui.server`, each carve being a symbol move plus a one-line import flip in the tests. Task 7 syncs docs and runs the end-to-end sweep.

**Tech Stack:** Python 3.12 (stdlib `http.server`, `zoneinfo`, `tomllib`), jsonschema, Playwright (Firefox), pytest, Nix flake apps. No linter.

**Spec:** `docs/superpowers/specs/2026-08-14-project-layout-design.md`

## Global Constraints

- Package root is `src/publicator/`; tests live in `tests/`. Nothing importable stays at the repo root.
- `nix run` app names and their CLI flags do not change: `publish-next`, `da-publish`, `validate`, `check-steps`, `pw-daemon`, `login`. One app is added: `echo-first`.
- Import direction is one-way, no cycles: `config`/`entries`/`images`/`llm_meta` → `store` → `deviantart` → `webui.page` → `webui.server` → `apps.*`. `scheduling` imports only `entries`.
- `--selfcheck` is removed from every CLI; its assertions become pytest tests. `--check-steps` stays exactly as it is (it has a flake app).
- Stored timestamps stay LOCAL wall-clock (e.g. 20:00 Europe/Paris), never UTC. No weekday/hour math in the served page's JS.
- Names crossing a module boundary lose their leading underscore; module-private helpers keep it.
- Every task ends with `nix develop -c pytest -q` green and a commit. Baseline to beat: **13 passed**.
- Run everything inside `nix develop -c ...`. There is no pip/venv.

---

### Task 1: Relocate into `src/publicator/` + `tests/`

**Files:**
- Create: `pyproject.toml`, `src/publicator/__init__.py`, `src/publicator/webui/__init__.py`, `src/publicator/apps/__init__.py`, `src/publicator/apps/{publish_next,da_publish,validate,echo_first}.py`
- Move: `validate.py`→`src/publicator/config.py`, `echo_first_unpublished_publication_data.py`→`src/publicator/entries.py`, `llm_meta.py`→`src/publicator/llm_meta.py`, `da_publish.py`→`src/publicator/deviantart.py`, `publish_next.py`→`src/publicator/publish_next.py`, `pw_daemon.py`→`src/publicator/apps/pw_daemon.py`, `publicationsSchema.json`→`src/publicator/publicationsSchema.json`, `test_llm_meta.py`→`tests/test_llm_meta.py`
- Delete after distributing its tests: `test_publish_next.py`
- Create: `tests/{test_config,test_deviantart,test_scheduling,test_store,test_page}.py`
- Modify: `flake.nix`, `CLAUDE.md` (commands block only), `AGENTS.md` (commands block only)

**Interfaces:**
- Consumes: nothing (first task).
- Produces the module paths every later task imports:
  - `publicator.setup_logging(verbose: bool) -> None`
  - `publicator.config.{SCHEMA, load_config(cwd) -> dict, validate_publications(data, config) -> None}`
  - `publicator.entries.{STATE_UNPUBLISHED, STATE_PUBLISHED, set_data_dir, deviantart_apparition, find_art_path, format_schedule, first_unpublished, load_pubs, pubs_file, DATA_DIR}`
  - `publicator.llm_meta.{DEFAULT_MODEL, generate_metadata, run_llm, TITLE_DESC_SCHEMA}`
  - `publicator.deviantart.{configure, publish_batch, load_pending_entries, mark_state, atomic_write_json, check_steps, parse_schedule, STEPS, TAGS_FILE, SKILL_MD}`
  - `publicator.publish_next.*` — everything else, with these renamed (underscore dropped): `schedule_data`, `schedule_profiles`, `profile_slots`, `ts_labels`, `existing_ts`, `zone`, `resolve_ts`, `set_or_pop`, `write_publications`, `apply_update`, `PAGE_TEMPLATE`, `tier_gallery_fields`, `preset_options`, `card_form_html`

- [ ] **Step 1: Create the package skeleton**

```bash
mkdir -p src/publicator/webui src/publicator/apps tests
touch src/publicator/webui/__init__.py src/publicator/apps/__init__.py
```

- [ ] **Step 2: Write `pyproject.toml`**

```toml
[project]
name = "publicator"
version = "0.1.0"
description = "Human-review gallery + DeviantArt publishing pipeline"
requires-python = ">=3.12"
dependencies = ["jsonschema", "playwright", "llm"]

[build-system]
requires = ["setuptools>=68"]
build-backend = "setuptools.build_meta"

[tool.setuptools.packages.find]
where = ["src"]

[tool.setuptools.package-data]
publicator = ["publicationsSchema.json"]

[tool.pytest.ini_options]
pythonpath = ["src"]
testpaths = ["tests"]
```

Nothing installs this package — Nix provides the dependencies and the apps run
modules directly. `pythonpath = ["src"]` is what lets pytest import
`publicator` without an install step.

- [ ] **Step 3: Write `src/publicator/__init__.py`**

Move `setup_logging` here verbatim from `da_publish.py:35-43`:

```python
"""Publicator: human-review gallery + DeviantArt publishing pipeline."""

import logging


def setup_logging(verbose: bool) -> None:
    """App-wide logging. verbose -> DEBUG (llm calls, HTTP, each publish step),
    else INFO. Shared by every entrypoint; basicConfig is a no-op after the first
    call, so whichever main() runs first wins (they use the same settings)."""
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        datefmt="%H:%M:%S",
    )
```

- [ ] **Step 4: `git mv` every module and the schema**

```bash
git mv validate.py                              src/publicator/config.py
git mv echo_first_unpublished_publication_data.py src/publicator/entries.py
git mv llm_meta.py                              src/publicator/llm_meta.py
git mv da_publish.py                            src/publicator/deviantart.py
git mv publish_next.py                          src/publicator/publish_next.py
git mv pw_daemon.py                             src/publicator/apps/pw_daemon.py
git mv publicationsSchema.json                  src/publicator/publicationsSchema.json
git mv test_llm_meta.py                         tests/test_llm_meta.py
```

`git mv` (not `cp`) so history follows the files.

- [ ] **Step 5: Fix `src/publicator/config.py`**

- Delete `_selfcheck()` and `main()` and the `if __name__ == "__main__":` block, plus the now-unused `import argparse`.
- Keep `SCHEMA = Path(__file__).resolve().parent / "publicationsSchema.json"` unchanged — the schema moved next to it, so it still resolves.
- The module must end up with exactly: imports (`json`, `tomllib`, `Path`, `jsonschema.validate as _js_validate`), `SCHEMA`, `_SCHEMA`, `load_config`, `validate_publications`.

- [ ] **Step 6: Fix `src/publicator/entries.py`**

- Delete `main()`, the `if __name__ == "__main__":` block, and `import argparse` / `import sys`.
- Add the state constants (they move here from `da_publish.py:57-58`) right under `DATA_DIR`:

```python
# publications.json apparition states. They live here, with the entry model, so
# scheduling.py can filter on them without importing the Playwright module.
STATE_UNPUBLISHED = "unpublished"
STATE_PUBLISHED = "published_or_scheduled"
```

- In `first_unpublished`, replace the literal `"unpublished"` with `STATE_UNPUBLISHED`.

- [ ] **Step 7: Fix `src/publicator/deviantart.py`**

- Delete `setup_logging` (moved in Step 3), `_selfcheck()`, `main()`, the `if __name__ == "__main__":` block, and `import argparse`.
- Rename `_atomic_write_json` → `atomic_write_json` (definition + both call sites: `mark_state`, and it is imported by `publish_next`).
- Delete the local `STATE_UNPUBLISHED`/`STATE_PUBLISHED` definitions; import them instead.
- Replace the import block and the `PKG`/`SKILL_MD` constants with:

```python
from publicator.config import load_config
from publicator.entries import (
    STATE_PUBLISHED,
    STATE_UNPUBLISHED,
    deviantart_apparition,
    find_art_path,
    format_schedule,
    set_data_dir,
)

# The skill file is a repo asset, not a package one: src/publicator/deviantart.py
# -> parents[2] is the repo root. Absent (e.g. an installed copy) -> check_steps
# reports "not found" and skips, which is the documented dev-only behaviour.
REPO_ROOT = Path(__file__).resolve().parents[2]
SKILL_MD = REPO_ROOT / ".claude/skills/publish-deviantart/SKILL.md"
```

(`PKG` disappears; it had no other use.)

- [ ] **Step 8: Fix `src/publicator/publish_next.py`**

- Replace its import block with:

```python
from publicator.config import load_config, validate_publications
from publicator.deviantart import (
    atomic_write_json,
    configure,
    load_pending_entries,
    publish_batch,
)
from publicator.entries import STATE_UNPUBLISHED, deviantart_apparition
from publicator.llm_meta import DEFAULT_MODEL, generate_metadata
```

- Delete `_selfcheck()` (its assertions become tests in Steps 12–15), `main()` (moves in Step 10), the `if __name__ == "__main__":` block, and the now-unused `import argparse`.
- Rename, definitions and every call site (`rg -n` each old name afterwards to confirm zero hits): `_zone`→`zone`, `_schedule_profiles`→`schedule_profiles`, `_profile_slots`→`profile_slots`, `_schedule_data`→`schedule_data`, `_ts_labels`→`ts_labels`, `_existing_ts`→`existing_ts`, `_resolve_ts`→`resolve_ts`, `_set_or_pop`→`set_or_pop`, `_PAGE_TMPL`→`PAGE_TEMPLATE`, `_tier_gallery_fields`→`tier_gallery_fields`, `_preset_options`→`preset_options`, `_card_form_html`→`card_form_html`, `_DEFAULT_TZ`→`DEFAULT_TZ`, `_SLOT_HORIZON_WEEKS`→`SLOT_HORIZON_WEEKS`, `_WEEKDAY`→`WEEKDAY`.
- `_atomic_write_json` call in the `/update` handler becomes `atomic_write_json`.
- Keep `serve()` here for now (Task 6 moves it).

- [ ] **Step 9: Write the four app entrypoints**

`src/publicator/apps/validate.py`:

```python
#!/usr/bin/env python3
"""CLI: validate publications.json against the bundled schema."""
import argparse
import json
import sys
from pathlib import Path

from publicator.config import load_config, validate_publications


def main() -> int:
    ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
    ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
    args = ap.parse_args()
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    data = json.loads((data_dir / "publications.json").read_text())
    validate_publications(data, load_config(data_dir))
    print("Valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

`src/publicator/apps/echo_first.py` — the body is `entries.main()` verbatim:

```python
#!/usr/bin/env python3
"""CLI: print path/title/schedule for the first state=unpublished publication."""
import argparse
import sys

from publicator.entries import (
    deviantart_apparition,
    find_art_path,
    first_unpublished,
    format_schedule,
    set_data_dir,
)


def main() -> int:
    ap = argparse.ArgumentParser(description="Print the first unpublished publication's data.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir (publications.json + images); default: CWD")
    set_data_dir(ap.parse_args().data_dir)
    pub = first_unpublished()
    app = deviantart_apparition(pub)
    ts = app.get("apparitionTimestampIfDifferentThanSubmission")
    if ts is None:
        raise SystemExit(f"no apparitionTimestampIfDifferentThanSubmission on {pub['uuid']}")
    print(f"path: {find_art_path(pub['files'][0]['basename'])}")
    print(f"title: {app['urlElsePublicationName']}")
    print(f"schedule: {format_schedule(ts)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

`src/publicator/apps/da_publish.py` — `deviantart.main()` minus `--selfcheck`:

```python
#!/usr/bin/env python3
"""CLI: publish ONE publications.json entry to DeviantArt via Playwright."""
import argparse
import sys

from publicator import setup_logging
from publicator import deviantart
from publicator.deviantart import check_steps, configure, load_pending_entries, publish_batch


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Publish ONE publications.json entry to DeviantArt via Playwright.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir (publications.json + .deviantart-session); default: CWD")
    ap.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    ap.add_argument("--uuid", default=None,
                    help="entry to publish; default: first state=unpublished")
    ap.add_argument("--check-steps", action="store_true",
                    help="verify STEPS mirror the skill's steps, then exit")
    ap.add_argument("-v", "--verbose", action="store_true", help="debug logging (steps, session, llm)")
    a = ap.parse_args()

    setup_logging(a.verbose)

    if a.check_steps:
        return 0 if check_steps() else 1

    configure(a.data_dir)
    json_path = a.json or str(deviantart.DATA_DIR / "publications.json")
    entries = load_pending_entries(json_path)
    if a.uuid:
        entries = [e for e in entries if e["uuid"] == a.uuid]
        if not entries:
            print(f"no state=unpublished entry with uuid {a.uuid}", file=sys.stderr)
            return 1
    if not entries:
        print("nothing to publish (no state=unpublished entry)")
        return 0

    entry = entries[0]
    print(f"publishing: {entry['title']}  [{entry['uuid']}]")
    published, failed, err = publish_batch([entry], [entry["uuid"]], json_path)
    print(f"published={published} failed={failed}")
    if err:
        print(f"error: {err}", file=sys.stderr)
    return 0 if published and not failed else 1


if __name__ == "__main__":
    sys.exit(main())
```

`deviantart.DATA_DIR` is read through the module (not `from ... import DATA_DIR`)
because `configure()` rebinds it — importing the name would capture the stale value.

`src/publicator/apps/publish_next.py` — `publish_next.main()` verbatim, minus the `--selfcheck` branch:

```python
#!/usr/bin/env python3
"""CLI: the unified publish workflow (gallery + AI metadata + Firefox batch)."""
import argparse
import logging
import sys
import tempfile
from pathlib import Path

from publicator import setup_logging
from publicator.config import load_config
from publicator.deviantart import configure, load_pending_entries
from publicator.llm_meta import DEFAULT_MODEL
from publicator.publish_next import find_candidates, generate_thumbnails, serve

log = logging.getLogger("publicator.gallery")


def main() -> int:
    parser = argparse.ArgumentParser(description="Unified publish workflow (gallery + AI + Firefox DA).")
    parser.add_argument("-n", type=int, default=10)
    parser.add_argument("--data-dir", default=None,
                        help="publication database dir (publications.json + images + browser session); default: CWD")
    parser.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    parser.add_argument("--ai-model", default=DEFAULT_MODEL)
    parser.add_argument("--openrouter-model",
                        # ponytail: free :free ids churn on OpenRouter; this is the current
                        # free model with both vision and structured_outputs. Override via flag.
                        default="openrouter/google/gemma-4-26b-a4b-it:free",
                        help="free vision model for the OpenRouter option; needs $OPENROUTER_KEY")
    parser.add_argument("--ai-timeout", type=int, default=300,
                        help="seconds to wait for an AI title/description (default: 300)")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="debug logging (HTTP requests, AI/llm calls, publish steps)")
    args = parser.parse_args()

    setup_logging(args.verbose)

    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    config = load_config(data_dir)
    configure(args.data_dir, config)  # points deviantart + entry helpers at the db dir
    args.json = args.json or str(data_dir / "publications.json")
    publicable_dirs = [str(data_dir / d) for d in config["publicable"]]

    print("Finding unpublished images...")
    candidates = find_candidates(publicable_dirs, args.json, args.n)
    pending = load_pending_entries(args.json)
    log.debug("found %d candidate(s), %d pending", len(candidates), len(pending))
    if not candidates and not pending:
        print("Nothing to publish (no new picks, no pending queue).")
        return 0

    msg = f"{len(candidates)} new pick(s)"
    if pending:
        msg += f", {len(pending)} already queued"
    print(f"Found {msg}. Generating thumbnails...")
    with tempfile.TemporaryDirectory(prefix="publish-next-") as thumb_dir:
        thumb_map = generate_thumbnails(candidates + [e["path"] for e in pending], thumb_dir)
        log.debug("generated %d thumbnail(s) in %s", len(thumb_map), thumb_dir)
        result = serve(thumb_dir, thumb_map, candidates, pending, args, config)

    if result is None:
        print("No publish action taken.")
        return 0
    print(f"Done. published={result.get('published',0)} failed={result.get('failed',0)}")
    if result.get("error"):
        print(f"error: {result['error']}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 10: Fix `src/publicator/apps/pw_daemon.py`**

It is a self-contained script (no cross-module imports). Leave the body alone;
only confirm it still ends with `if __name__ == "__main__":` and that
`python -m publicator.apps.pw_daemon --help` would work. It imports only
stdlib + `playwright`.

- [ ] **Step 11: Rewrite the flake apps to run modules**

In `flake.nix`, change the `app` helper's second parameter from a script path to
a module name and export `PYTHONPATH`:

```nix
      app = name: module: extra: pw: {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = name;
          runtimeInputs = [ python ] ++ extra;
          text = (if pw then ''
            export PLAYWRIGHT_BROWSERS_PATH=${browsers}
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
          '' else "") + ''
            export PYTHONPATH=${src}/src
            exec python -m publicator.apps.${module} "$@"
          '';
        }}/bin/${name}";
      };
```

Update the call sites and add `echo-first`:

```nix
        publish-next = app "publish-next" "publish_next" [ pkgs.imagemagick browsers claude ] true;
        da-publish = app "da-publish" "da_publish" [ pkgs.imagemagick browsers ] true;
        pw-daemon = app "pw-daemon" "pw_daemon" [ browsers ] true;
        validate = app "validate" "validate" [ ] false;
        # First state=unpublished entry's path/title/schedule (used by the skill).
        echo-first = app "echo-first" "echo_first" [ ] false;
```

`check-steps` keeps its own `writeShellApplication` but changes its text to:

```nix
            text = ''
              export PYTHONPATH=${src}/src
              exec python -m publicator.apps.da_publish --check-steps "$@"
            '';
```

And the devShell `shellHook` gains (so `python -m publicator.apps.X` works from a checkout):

```nix
          export PYTHONPATH="$PWD/src''${PYTHONPATH:+:$PYTHONPATH}"
```

- [ ] **Step 12: Write `tests/test_config.py`** (converted from `validate._selfcheck`)

```python
import pytest

from publicator.config import load_config, validate_publications


def test_load_config_defaults_when_file_missing(tmp_path):
    assert load_config(tmp_path) == {
        "tiers": [], "galleries": [], "publicable": [], "tags": None, "schedule": {}}


def test_load_config_reads_flat_schedule(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        'publicable = ["picked"]\n'
        'tags = "sub/tags.txt"\n'
        '[deviantart]\ntiers = ["T"]\ngalleries = ["G"]\n'
        '[schedule]\nfrequency = "weekly"\nday = "tuesday"\nhour = 20\nper_slot = 2\n')
    c = load_config(tmp_path)
    assert c["tiers"] == ["T"] and c["galleries"] == ["G"], c
    assert c["publicable"] == ["picked"], c
    assert c["tags"] == "sub/tags.txt", c
    assert c["schedule"] == {
        "frequency": "weekly", "day": "tuesday", "hour": 20, "per_slot": 2}, c


def test_load_config_reads_schedule_profiles(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[[schedule.profiles]]\nname = "free"\nday = "tuesday"\nhour = 20\nper_slot = 2\n'
        '[[schedule.profiles]]\nname = "paid"\nday = "friday"\nhour = 20\nper_slot = 1\n')
    assert load_config(tmp_path)["schedule"] == {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}


def _pubs(**apparition):
    """One schema-valid publication; uuid is exactly 36 chars as the schema demands."""
    return [{
        "uuid": "00000000-0000-4000-8000-000000000001",
        "submissionTimestamp": 1700000000,
        "description": "d",
        "files": [{"basename": "a.png", "sha512sum": "0" * 128}],
        "apparitions": [{
            "platformName": "deviantart",
            "state": "unpublished",
            "urlElsePublicationName": "t",
            "apparitionTimestampIfDifferentThanSubmission": 1800000000,
            **apparition,
        }],
    }]


def test_validate_accepts_configured_tier_and_gallery():
    validate_publications(_pubs(tier="gold", galleries=["Art"]),
                          {"tiers": ["gold"], "galleries": ["Art"]})


def test_validate_rejects_tier_absent_from_config():
    with pytest.raises(ValueError, match="tier"):
        validate_publications(_pubs(tier="gold"), {"tiers": [], "galleries": []})


def test_validate_rejects_gallery_absent_from_config():
    with pytest.raises(ValueError, match="gallery"):
        validate_publications(_pubs(galleries=["Art"]), {"tiers": [], "galleries": []})
```

- [ ] **Step 13: Write `tests/test_deviantart.py`** (converted from `da_publish._selfcheck`)

```python
from pathlib import Path

import pytest

from publicator import deviantart
from publicator.deviantart import STEPS, check_steps, configure, parse_schedule


@pytest.mark.parametrize("s,expected", [
    ("Tue Sep 8 08:00:00 PM CEST 2026", (2026, 9, 8, 20)),
    ("Wed Jan 1 12:00:00 AM UTC 2025", (2025, 1, 1, 0)),
    ("Wed Jan 1 12:00:00 PM UTC 2025", (2025, 1, 1, 12)),
])
def test_parse_schedule(s, expected):
    assert parse_schedule(s) == expected


def test_parse_schedule_rejects_garbage():
    with pytest.raises(ValueError):
        parse_schedule("not a date")


def test_steps_map_to_callables():
    assert STEPS and all(callable(fn) for _, fn in STEPS)


def test_steps_stay_in_sync_with_the_skill():
    assert check_steps()


class _NoPage:
    def __getattr__(self, name):
        raise AssertionError(f"step touched the page ({name}) when it should no-op")


def test_optional_steps_are_no_ops_when_unset():
    deviantart._step_premium(_NoPage(), {"price": None})
    deviantart._step_tier(_NoPage(), {})
    deviantart._step_galleries(_NoPage(), {})


def test_configure_resolves_tags_file_under_data_dir(tmp_path):
    configure(str(tmp_path), {"tags": "sub/tags.txt"})
    assert deviantart.TAGS_FILE == Path(tmp_path) / "sub/tags.txt"


def test_configure_leaves_tags_file_unset_without_config(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart.TAGS_FILE is None


def test_login_cookies_empty_without_profile_db(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart._da_login_cookies() == []
```

- [ ] **Step 14: Write `tests/test_scheduling.py`**

Takes the two schedule tests out of `test_publish_next.py` (verbatim bodies,
plus the `_paris_tuesday_2000_epochs`, `_schedule_core_js` and `_next_slot_for`
helpers) and adds the cadence assertions converted from
`publish_next._selfcheck`. Import from `publicator.publish_next` for now —
Task 3 flips this one line to `publicator.scheduling`.

```python
import json
import os
import shutil
import subprocess
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

import pytest

from publicator import publish_next as scheduling   # Task 3 -> from publicator import scheduling
from publicator.publish_next import PAGE_TEMPLATE

_TZ = "Europe/Paris"
_PARIS = ZoneInfo(_TZ)


def _paris_tuesday_2000_epochs(count):
    """`count` consecutive Tuesday-20:00 Europe/Paris timestamps, first one the
    next Tuesday strictly after now — the real cadence the schedule produces.
    Rebuilt at local 20:00 each week so it stays correct across DST."""
    now = datetime.now(_PARIS)
    ahead = (1 - now.weekday()) % 7  # Python weekday: Tuesday == 1
    first = datetime(now.year, now.month, now.day, 20, tzinfo=_PARIS) + timedelta(days=ahead)
    if first <= now:
        first += timedelta(days=7)
    out = []
    for i in range(count):
        d = (first + timedelta(days=7 * i)).date()
        out.append(int(datetime(d.year, d.month, d.day, 20, tzinfo=_PARIS).timestamp()))
    return out


def _schedule_core_js():
    """The live scheduling functions, sliced verbatim from the served page
    between its marker comments."""
    start = PAGE_TEMPLATE.index("// >>> scheduler core")
    end = PAGE_TEMPLATE.index("// <<< scheduler core")
    return PAGE_TEMPLATE[start:end]


def _next_slot_for(schedules, existing_ts, profile_name, tz):
    """Run the real client scheduler in Node under browser timezone `tz` and
    return the epoch it would pre-fill the date picker with for `profile_name`.
    LABELS is irrelevant to slot choice; an empty map keeps the harness honest."""
    harness = (
        f"const SCHEDULES = {json.dumps(schedules)};\n"
        f"const EXISTING_TS = {json.dumps(existing_ts)};\n"
        "const LABELS = {};\n"
        "const queue = [];\n"
        f"{_schedule_core_js()}\n"
        f"console.log(nextSlotForProfile(profileByName({json.dumps(profile_name)})));\n"
    )
    out = subprocess.run(
        ["node", "-e", harness], check=True, capture_output=True, text=True,
        env={"TZ": tz, "PATH": os.environ["PATH"]},
    )
    return int(out.stdout.strip())


def test_profiles_normalize_day_names_to_python_weekdays():
    assert scheduling.schedule_profiles({"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2}]}) == \
        [{"name": "free", "day": 1, "hour": 20, "per_slot": 2}]


def test_flat_config_becomes_one_default_profile():
    assert scheduling.schedule_profiles({"day": "friday", "hour": 18, "per_slot": 1}) == \
        [{"name": "default", "day": 4, "hour": 18, "per_slot": 1}]


def test_empty_config_falls_back_to_tuesday_2000():
    assert scheduling.schedule_profiles({}) == \
        [{"name": "default", "day": 1, "hour": 20, "per_slot": 2}]


@pytest.mark.parametrize("bad,exc", [
    ({"profiles": [{"frequency": "monthly"}]}, NotImplementedError),
    ({"profiles": [{"day": "tuesday", "per_slot": 0}]}, ValueError),
    ({"profiles": [{"day": "someday"}]}, ValueError),
    ({"profiles": [{"name": "a", "day": "tuesday", "hour": 20},
                   {"name": "b", "day": "tuesday", "hour": 20}]}, ValueError),
    ({"profiles": []}, ValueError),
])
def test_invalid_profiles_are_rejected(bad, exc):
    with pytest.raises(exc):
        scheduling.schedule_profiles(bad)


def test_existing_ts_keeps_only_future_scheduled_entries(tmp_path):
    jp = tmp_path / "publications.json"
    future = int(datetime.now(_PARIS).timestamp()) + 86400
    past = int(datetime.now(_PARIS).timestamp()) - 86400
    jp.write_text(json.dumps([
        {"apparitions": [{"state": "published_or_scheduled",
                          "apparitionTimestampIfDifferentThanSubmission": future}]},
        {"apparitions": [{"state": "published_or_scheduled",
                          "apparitionTimestampIfDifferentThanSubmission": past}]},
        {"apparitions": [{"state": "unpublished",
                          "apparitionTimestampIfDifferentThanSubmission": future + 60}]},
    ]))
    assert scheduling.existing_ts(str(jp)) == [future]


def test_existing_ts_tolerates_a_missing_file(tmp_path):
    assert scheduling.existing_ts(str(tmp_path / "nope.json")) == []


def test_schedule_slots_stay_2000_paris_across_dst():
    """Every generated slot is 20:00 Europe/Paris wall-clock, summer or winter —
    the invariant a fixed UTC offset would violate at the DST boundary."""
    data = scheduling.schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2}]})
    slots = data[0]["slots"]
    assert len(slots) >= 52
    hours = {datetime.fromtimestamp(s, _PARIS).hour for s in slots}
    assert hours == {20}, sorted(hours)
    # crosses at least one DST transition -> two distinct UTC offsets present
    offsets = {datetime.fromtimestamp(s, _PARIS).utcoffset() for s in slots}
    assert len(offsets) == 2, offsets


def test_new_slot_packs_after_taken_slots_regardless_of_browser_tz(tmp_path):
    """Regression: with 6 Tuesdays fully booked (Europe/Paris 20:00), a fresh
    image must default to the 7th Tuesday — and it must do so no matter what
    timezone the *browser* reports. `firefox --private-window` with resist-
    fingerprinting spoofs Date to UTC, and the old wall-clock matching (getHours)
    then failed to recognize the 18:00-UTC slots as taken, re-suggesting the very
    next, already-full Tuesday. Slots are now server-generated absolute instants,
    so occupancy is TZ-independent."""
    if not shutil.which("node"):
        pytest.skip("node required to exercise the client scheduler")

    weeks = _paris_tuesday_2000_epochs(7)
    taken, expected = weeks[:6], weeks[6]

    pubs = []
    for i, ts in enumerate(taken):
        for slot in range(2):
            pubs.append({
                "uuid": f"u{i}-{slot}", "description": "d",
                "files": [{"basename": "img.png"}],
                "apparitions": [{
                    "platformName": "deviantart", "state": "published_or_scheduled",
                    "urlElsePublicationName": "t",
                    "apparitionTimestampIfDifferentThanSubmission": ts,
                }],
            })
    pub_json = tmp_path / "publications.json"
    pub_json.write_text(json.dumps(pubs))

    existing = scheduling.existing_ts(str(pub_json))
    # Anchor slot generation just before the first Tuesday so its slot list starts
    # exactly at weeks[0] — deterministic regardless of when the test runs.
    schedules = scheduling.schedule_data(
        {"timezone": _TZ, "profiles": [
            {"name": "tuesday", "day": "tuesday", "hour": 20, "per_slot": 2},
            {"name": "thursday", "day": "thursday", "hour": 20, "per_slot": 2}]},
        now=weeks[0] - 3600)

    for tz in ("Europe/Paris", "UTC", "America/New_York"):
        got = _next_slot_for(schedules, existing, "tuesday", tz)
        assert got == expected, (
            f"[browser TZ={tz}] picker defaulted to "
            f"{datetime.fromtimestamp(got, _PARIS)}, expected "
            f"{datetime.fromtimestamp(expected, _PARIS)} (first open Tuesday)")
```

- [ ] **Step 15: Write `tests/test_store.py`**

The custom-TZ test from `test_publish_next.py` plus the `write_publications` /
`apply_update` round-trip from `publish_next._selfcheck`. Import from
`publicator.publish_next` for now — Task 4 flips it to `publicator.store`.

```python
import json
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

import pytest

from publicator import publish_next as store   # Task 4 -> from publicator import store
from publicator.entries import STATE_UNPUBLISHED

_TZ = "Europe/Paris"
_PARIS = ZoneInfo(_TZ)
_CFG = {"schedule": {"timezone": _TZ}, "tiers": ["gold"], "galleries": ["Art"]}
_TUE_2000 = int(datetime(2026, 1, 6, 20, tzinfo=timezone.utc).timestamp())  # 2026-01-06 is a Tue


@pytest.fixture
def img(tmp_path):
    p = tmp_path / "pic.png"
    p.write_bytes(b"\x89PNG\r\n\x1a\n")  # bytes are enough for sha512
    return str(p)


def test_write_publications_appends_an_unpublished_entry(tmp_path, img):
    jp = str(tmp_path / "publications.json")
    uuids = store.write_publications(
        [{"path": img, "title": "t", "description": "d", "scheduleTs": _TUE_2000}], jp, _CFG)
    assert len(uuids) == 1
    rows = json.loads(open(jp).read())
    assert len(rows) == 1 and rows[0]["uuid"] == uuids[0]
    app = rows[0]["apparitions"][0]
    assert app["state"] == STATE_UNPUBLISHED
    assert app["apparitionTimestampIfDifferentThanSubmission"] == _TUE_2000
    assert app["urlElsePublicationName"] == "t"


def test_write_publications_requires_a_schedule(tmp_path, img):
    jp = str(tmp_path / "publications.json")
    with pytest.raises(ValueError):
        store.write_publications([{"path": img, "title": "t", "description": "d"}], jp, _CFG)


def test_apply_update_sets_then_clears_optional_fields(tmp_path, img):
    jp = str(tmp_path / "publications.json")
    uuids = store.write_publications(
        [{"path": img, "title": "t", "description": "d", "scheduleTs": _TUE_2000}], jp, _CFG)
    rows = json.loads(open(jp).read())

    store.apply_update(rows, uuids[0],
                       {"title": "T2", "description": "D2", "scheduleTs": _TUE_2000 + 604800,
                        "price": 3, "tier": "gold", "galleries": ["Art"]}, _PARIS)
    app = rows[0]["apparitions"][0]
    assert app["urlElsePublicationName"] == "T2" and app["priceIfNotFree"] == 3.0
    assert app["tier"] == "gold" and app["galleries"] == ["Art"]

    store.apply_update(rows, uuids[0],
                       {"title": "T3", "description": "D3", "scheduleTs": _TUE_2000 + 604800},
                       _PARIS)
    app = rows[0]["apparitions"][0]
    assert "priceIfNotFree" not in app and "tier" not in app and "galleries" not in app


def test_apply_update_raises_on_unknown_uuid():
    with pytest.raises(KeyError):
        store.apply_update([], "nope", {"title": "t", "scheduleTs": _TUE_2000}, _PARIS)


def test_resolve_ts_reads_a_naive_string_in_the_schedule_timezone():
    assert store.resolve_ts({"schedule": "2026-10-15T20:00"}, _PARIS) == \
        int(datetime(2026, 10, 15, 20, tzinfo=_PARIS).timestamp())


def test_custom_schedule_resolves_in_config_timezone(tmp_path, img):
    """A custom-typed wall-clock string persists as the schedule-TZ instant, not
    whatever the browser's timezone makes of it. A UTC-spoofed private window sends
    a scheduleTs 2h early; the naive 'schedule' string must win on both write paths."""
    jp = str(tmp_path / "publications.json")
    intended = int(datetime(2026, 10, 15, 20, tzinfo=_PARIS).timestamp())        # 20:00 Paris
    wrong = int(datetime(2026, 10, 15, 20, tzinfo=ZoneInfo("UTC")).timestamp())  # 20:00 UTC

    uuids = store.write_publications(
        [{"path": img, "title": "t", "description": "d",
          "schedule": "2026-10-15T20:00", "scheduleTs": wrong}], jp, _CFG)
    rows = json.loads(open(jp).read())
    assert rows[0]["apparitions"][0][
        "apparitionTimestampIfDifferentThanSubmission"] == intended

    store.apply_update(rows, uuids[0],
                       {"title": "t2", "description": "d",
                        "schedule": "2026-10-20T20:00", "scheduleTs": wrong}, _PARIS)
    assert rows[0]["apparitions"][0]["apparitionTimestampIfDifferentThanSubmission"] == \
        int(datetime(2026, 10, 20, 20, tzinfo=_PARIS).timestamp())
```

- [ ] **Step 16: Write `tests/test_page.py`**

The three page tests from `test_publish_next.py`, rewritten against the class
attributes as they are today (Task 5 replaces this file's body with
`render_page(...)` kwargs), plus the `/original` query-decoding assertion from
`publish_next._selfcheck`.

```python
import urllib.parse

from publicator import publish_next
from publicator.publish_next import GalleryHandler


def _handler(**overrides):
    """Configure the handler's class attributes for a render-only call.
    ponytail: _build_page reads only class attrs, so it is called with the class
    as `self` — constructing a real BaseHTTPRequestHandler would need a socket."""
    GalleryHandler.thumb_dir = "/t"
    GalleryHandler.thumb_map = {}
    GalleryHandler.candidate_paths = []
    GalleryHandler.pending = []
    GalleryHandler.existing_ts = []
    GalleryHandler.ai_model = "m"
    GalleryHandler.openrouter_model = "o"
    GalleryHandler.config = {}
    GalleryHandler.schedules = publish_next.schedule_data({})
    for k, v in overrides.items():
        setattr(GalleryHandler, k, v)
    return GalleryHandler


def test_page_offers_both_models():
    H = _handler(ai_model="claude-cli-opus",
                 openrouter_model="openrouter/google/gemini-2.0-flash-exp:free")
    page = GalleryHandler._build_page(H)
    assert 'id="ai-model"' in page
    assert "claude-cli-opus" in page
    assert "openrouter/google/gemini-2.0-flash-exp:free" in page
    assert "__AI_MODEL__" not in page and "__OPENROUTER_MODEL__" not in page


def test_page_renders_config_tiers_galleries():
    H = _handler(candidate_paths=["/t/a.png"],
                 config={"tiers": ["gold"], "galleries": ["Art"]})
    page = GalleryHandler._build_page(H)
    assert 'class="f-tier"' in page and ">gold<" in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    config = {"schedule": {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}}
    H = _handler(candidate_paths=["/t/a.png"], config=config,
                 schedules=publish_next.schedule_data(config["schedule"]))
    page = GalleryHandler._build_page(H)
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page   # SCHEDULES injected
    assert '"slots"' in page                                       # canonical instants embedded
    assert "__SCHEDULES__" not in page and "__EXISTING_TS__" not in page
    assert "__LABELS__" not in page                                # label map substituted


def test_original_query_decodes_verbatim_for_the_allow_list():
    """/original decodes the path arg verbatim, so the allow-list (path in
    thumb_map) sees the real path — a non-listed path can't sneak through."""
    q = urllib.parse.parse_qs(
        urllib.parse.urlparse("/original?path=%2Fetc%2Fpasswd").query)
    assert q.get("path", [""])[0] == "/etc/passwd"
```

- [ ] **Step 17: Delete the old test file and point `tests/test_llm_meta.py` at the package**

```bash
git rm test_publish_next.py
```

In `tests/test_llm_meta.py`, change `import llm_meta` to
`from publicator import llm_meta`. Nothing else in that file changes.

- [ ] **Step 18: Run the suite**

Run: `nix develop -c pytest -q`
Expected: PASS, **at least 30 tests** (13 baseline + the converted self-checks), 0 failed.
If collection fails with `ModuleNotFoundError: publicator`, `pythonpath = ["src"]` is missing or misspelled in `pyproject.toml`.

- [ ] **Step 19: Prove the apps still run**

```bash
nix run .#check-steps
tmp=$(mktemp -d); echo '[]' > "$tmp/publications.json"
nix run .#validate     -- --data-dir "$tmp"
nix run .#da-publish   -- --data-dir "$tmp"
nix run .#publish-next -- --data-dir "$tmp"
```

Expected, in order: `check-steps OK: 12 steps in sync with the skill`; `Valid`;
`nothing to publish (no state=unpublished entry)`; `Nothing to publish (no new picks, no pending queue).`

- [ ] **Step 20: Sync the command blocks in `CLAUDE.md` and `AGENTS.md`**

Replace the commands section of both files (they mirror each other) so no
documented command names a deleted path:

```sh
nix develop -c pytest -q                              # full test suite
nix develop -c pytest tests/test_scheduling.py -q     # one file
nix run <this>#check-steps                            # drift guard: STEPS registry vs the publish-deviantart skill
```

Delete the three `--selfcheck` lines and the sentence "plus each module's
`_selfcheck()` (assert-based, offline, no browser) — keep both green after
edits", replacing it with: "Correctness is guarded by pytest (`tests/`)."
Add `nix run <this>#echo-first` to the apps list. Full architecture prose is
rewritten in Task 7 — keep this step to the command lines.

- [ ] **Step 21: Commit**

```bash
git add -A
git commit -m "refactor(layout): move modules into src/publicator, tests into tests/

git mv every module into the package (validate->config,
echo_first_unpublished_publication_data->entries, da_publish->deviantart),
extract each main() into publicator/apps/, and convert the three --selfcheck
blocks into pytest files. Flake apps now run 'python -m publicator.apps.<x>'
with PYTHONPATH=src; app names and flags are unchanged, plus a new echo-first
app for the skill. --check-steps untouched."
```

---

### Task 2: Extract `images.py`

**Files:**
- Create: `src/publicator/images.py`, `tests/test_images.py`
- Modify: `src/publicator/publish_next.py`, `src/publicator/apps/publish_next.py`

**Interfaces:**
- Consumes: `publicator.publish_next` symbols from Task 1.
- Produces: `publicator.images.{IMAGE_EXTENSIONS, MIME, guess_mime(path) -> str, compute_sha512(filepath, chunk_size=1048576) -> str, load_publicated_hashes(json_path) -> set[str], collect_images(directory) -> list[str], find_candidates(directories, json_path, limit) -> list[str], generate_thumbnails(image_paths, thumb_dir) -> dict[str, str]}`

- [ ] **Step 1: Create `src/publicator/images.py`**

Move, verbatim, from `src/publicator/publish_next.py`: the `IMAGE_EXTENSIONS`
and `MIME` constants and the functions `compute_sha512`,
`load_publicated_hashes`, `collect_images`, `find_candidates`,
`generate_thumbnails`. Add the module docstring and, converted from
`GalleryHandler._guess_mime`, a free function:

```python
"""Candidate discovery (hash-dedup against publications.json) and thumbnails."""


def guess_mime(path: str) -> str:
    return MIME.get(os.path.splitext(path)[1].lower(), "application/octet-stream")
```

Imports needed: `hashlib`, `json`, `os`, `random`, `shutil`, `subprocess`, `sys`.

- [ ] **Step 2: Point `publish_next.py` at it**

Delete those symbols from `publish_next.py` and add
`from publicator.images import IMAGE_EXTENSIONS, MIME, find_candidates, generate_thumbnails, guess_mime, compute_sha512, load_publicated_hashes`,
then drop whichever names it no longer references (`compute_sha512` is used by
`write_publications`; `load_publicated_hashes` is used only by
`find_candidates`, so it does not need re-importing). Replace the
`GalleryHandler._guess_mime` method with calls to `guess_mime(...)` at its two
call sites in `do_GET`.

- [ ] **Step 3: Write `tests/test_images.py`**

```python
import json

from publicator.images import compute_sha512, find_candidates, guess_mime


def test_find_candidates_skips_already_published_hashes(tmp_path):
    picked = tmp_path / "picked"
    picked.mkdir()
    new, done = picked / "new.png", picked / "done.png"
    new.write_bytes(b"\x89PNG-new")
    done.write_bytes(b"\x89PNG-done")
    jp = tmp_path / "publications.json"
    jp.write_text(json.dumps([{"files": [{"sha512sum": compute_sha512(str(done)).upper()}]}]))

    got = find_candidates([str(picked)], str(jp), 10)

    assert got == [str(new)]  # case-insensitive hash match drops done.png


def test_find_candidates_honours_the_limit(tmp_path):
    picked = tmp_path / "picked"
    picked.mkdir()
    for i in range(5):
        (picked / f"{i}.png").write_bytes(f"img{i}".encode())
    assert len(find_candidates([str(picked)], str(tmp_path / "none.json"), 3)) == 3


def test_find_candidates_tolerates_a_missing_directory(tmp_path):
    assert find_candidates([str(tmp_path / "nope")], str(tmp_path / "none.json"), 5) == []


def test_guess_mime_falls_back_to_octet_stream():
    assert guess_mime("/a/b.png") == "image/png"
    assert guess_mime("/a/b.unknown") == "application/octet-stream"
```

- [ ] **Step 4: Run the suite**

Run: `nix develop -c pytest -q`
Expected: PASS, 4 more tests than Task 1.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(images): extract candidate discovery + thumbnails from publish_next"
```

---

### Task 3: Extract `scheduling.py`

**Files:**
- Create: `src/publicator/scheduling.py`
- Modify: `src/publicator/publish_next.py`, `tests/test_scheduling.py`

**Interfaces:**
- Consumes: `publicator.entries.STATE_UNPUBLISHED`.
- Produces: `publicator.scheduling.{DEFAULT_TZ, SLOT_HORIZON_WEEKS, WEEKDAY, zone(schedule) -> ZoneInfo, schedule_profiles(schedule) -> list[dict], profile_slots(profile, zone, start) -> list[int], schedule_data(schedule, now=None) -> list[dict], ts_labels(zone, tss) -> dict[int, str], existing_ts(json_path) -> list[int]}`

- [ ] **Step 1: Create `src/publicator/scheduling.py`**

Move verbatim from `publish_next.py`: the section comment, `DEFAULT_TZ`,
`SLOT_HORIZON_WEEKS`, `WEEKDAY`, `zone`, `schedule_profiles`, `profile_slots`,
`schedule_data`, `ts_labels`, `existing_ts`. Imports: `json`, `time`,
`datetime`/`timedelta` from `datetime`, `ZoneInfo` from `zoneinfo`, and
`from publicator.entries import STATE_UNPUBLISHED`.

Keep the module docstring explaining the invariant:

```python
"""Schedule cadence — ALL weekday/hour/timezone math lives here, server-side.

The browser receives absolute slot instants and counts occupancy by exact
timestamp equality; it never reads its own clock, because
`firefox --private-window` with resist-fingerprinting spoofs Date to UTC.
"""
```

- [ ] **Step 2: Point `publish_next.py` at it**

Delete those symbols there and add
`from publicator.scheduling import existing_ts, schedule_data, ts_labels, zone`
(the only four the remaining code calls — verify with
`rg -n "schedule_profiles|profile_slots|WEEKDAY|SLOT_HORIZON" src/publicator/publish_next.py`,
which must print nothing).

- [ ] **Step 3: Flip the test import**

In `tests/test_scheduling.py` replace
`from publicator import publish_next as scheduling   # Task 3 -> ...` with
`from publicator import scheduling`. Leave `from publicator.publish_next import PAGE_TEMPLATE` (Task 5 moves it).

- [ ] **Step 4: Run the suite**

Run: `nix develop -c pytest -q`
Expected: PASS, same count as Task 2.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(scheduling): extract server-side cadence math from publish_next"
```

---

### Task 4: Extract `store.py`

**Files:**
- Create: `src/publicator/store.py`
- Modify: `src/publicator/publish_next.py`, `src/publicator/deviantart.py`, `tests/test_store.py`

**Interfaces:**
- Consumes: `publicator.config.validate_publications`, `publicator.config.load_config`, `publicator.entries.{deviantart_apparition, STATE_UNPUBLISHED}`, `publicator.images.compute_sha512`.
- Produces: `publicator.store.{atomic_write_json(path, data) -> None, resolve_ts(fields, zone) -> int, write_publications(entries, json_path, config=None) -> list[str], apply_update(pubs, uuid_, fields, zone) -> None, set_or_pop(d, k, v) -> None}`

- [ ] **Step 1: Create `src/publicator/store.py`**

Move verbatim: `resolve_ts`, `write_publications`, `set_or_pop`, `apply_update`
from `publish_next.py`, and `atomic_write_json` from `deviantart.py`. Imports:
`json`, `os`, `tempfile`, `time`, `uuid`, `datetime` from `datetime`, `Path`
from `pathlib`, `ZoneInfo` from `zoneinfo`, plus
`from publicator.config import load_config, validate_publications`,
`from publicator.entries import STATE_UNPUBLISHED, deviantart_apparition`,
`from publicator.images import compute_sha512`.

Module docstring:

```python
"""publications.json writes: append, patch, and atomic replace."""
```

- [ ] **Step 2: Point the callers at it**

- `publish_next.py`: delete those four functions; import
  `from publicator.store import apply_update, atomic_write_json, write_publications`.
  It no longer needs `validate_publications`, `deviantart_apparition`, `uuid` or
  `compute_sha512` — remove any import that `rg` shows unused.
- `deviantart.py`: delete `atomic_write_json`, add
  `from publicator.store import atomic_write_json`. This is the one place the
  import direction could invert — `store` must NOT import `deviantart`; confirm
  with `rg -n "import" src/publicator/store.py`.

- [ ] **Step 3: Flip the test import**

In `tests/test_store.py` replace
`from publicator import publish_next as store   # Task 4 -> ...` with
`from publicator import store`.

- [ ] **Step 4: Run the suite**

Run: `nix develop -c pytest -q`
Expected: PASS, same count as Task 3.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor(store): extract publications.json writes from publish_next"
```

---

### Task 5: Extract `webui/page.py` with a pure `render_page`

**Files:**
- Create: `src/publicator/webui/page.py`
- Modify: `src/publicator/publish_next.py`, `tests/test_page.py`, `tests/test_scheduling.py`

**Interfaces:**
- Consumes: `publicator.scheduling.{ts_labels, zone}`.
- Produces: `publicator.webui.page.{PAGE_TEMPLATE, tier_gallery_fields(config) -> str, preset_options(schedules) -> str, card_form_html(cid, js_path, tg, save_label, preset_opts) -> str, render_page(*, thumb_dir, thumb_map, candidates, pending, existing_ts, schedules, config, ai_model, openrouter_model) -> str}`

- [ ] **Step 1: Create `src/publicator/webui/page.py`**

Move verbatim: `PAGE_TEMPLATE`, `tier_gallery_fields`, `preset_options`,
`card_form_html`. Then move the body of `GalleryHandler._build_page` into a
module-level function whose parameters replace every `self.` read:

```python
def render_page(*, thumb_dir, thumb_map, candidates, pending, existing_ts,
                schedules, config, ai_model, openrouter_model) -> str:
    """The whole gallery page as HTML. Pure: no handler, no socket, no I/O
    beyond the paths it is handed — which is what makes it testable."""
```

Inside, rename `self.thumb_dir`→`thumb_dir`, `self.thumb_map`→`thumb_map`,
`self.pending`→`pending`, `self.candidate_paths`→`candidates`,
`self.existing_ts`→`existing_ts`, `self.schedules`→`schedules`,
`self.config`→`config`, `self.ai_model`→`ai_model`,
`self.openrouter_model`→`openrouter_model`. Everything else — the card loops,
the label map, the seven `page.replace(...)` calls — is unchanged.

Imports: `html`, `json`, `os`, `urllib.parse`, `datetime` from `datetime`, and
`from publicator.scheduling import ts_labels, zone`.

- [ ] **Step 2: Shrink the handler's method to an adapter**

In `publish_next.py`, `GalleryHandler._build_page` becomes:

```python
    def _build_page(self) -> str:
        return render_page(
            thumb_dir=self.thumb_dir, thumb_map=self.thumb_map,
            candidates=self.candidate_paths, pending=self.pending,
            existing_ts=self.existing_ts, schedules=self.schedules,
            config=self.config, ai_model=self.ai_model,
            openrouter_model=self.openrouter_model)
```

with `from publicator.webui.page import render_page` at the top. Delete
`PAGE_TEMPLATE`, `tier_gallery_fields`, `preset_options`, `card_form_html` from
`publish_next.py`.

- [ ] **Step 3: Rewrite `tests/test_page.py` against the pure function**

```python
import urllib.parse

from publicator.scheduling import schedule_data
from publicator.webui.page import render_page


def _render(**overrides):
    kwargs = dict(thumb_dir="/t", thumb_map={}, candidates=[], pending=[],
                  existing_ts=[], schedules=schedule_data({}), config={},
                  ai_model="m", openrouter_model="o")
    kwargs.update(overrides)
    return render_page(**kwargs)


def test_page_offers_both_models():
    page = _render(ai_model="claude-cli-opus",
                   openrouter_model="openrouter/google/gemini-2.0-flash-exp:free")
    assert 'id="ai-model"' in page
    assert "claude-cli-opus" in page
    assert "openrouter/google/gemini-2.0-flash-exp:free" in page
    assert "__AI_MODEL__" not in page and "__OPENROUTER_MODEL__" not in page


def test_page_renders_config_tiers_galleries():
    page = _render(candidates=["/t/a.png"],
                   config={"tiers": ["gold"], "galleries": ["Art"]})
    assert 'class="f-tier"' in page and ">gold<" in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page


def test_page_renders_schedule_presets():
    config = {"schedule": {"profiles": [
        {"name": "free", "day": "tuesday", "hour": 20, "per_slot": 2},
        {"name": "paid", "day": "friday", "hour": 20, "per_slot": 1}]}}
    page = _render(candidates=["/t/a.png"], config=config,
                   schedules=schedule_data(config["schedule"]))
    assert 'class="f-preset"' in page
    assert '<option value="free">free</option>' in page
    assert '<option value="paid">paid</option>' in page
    assert '"name": "free"' in page and '"name": "paid"' in page
    assert '"slots"' in page
    assert "__SCHEDULES__" not in page and "__EXISTING_TS__" not in page
    assert "__LABELS__" not in page


def test_pending_card_renders_with_its_schedule_and_uuid():
    pending = [{"uuid": "00000000-0000-4000-8000-000000000001", "path": "/t/a.png",
                "title": "Tickler", "description": "d", "scheduleTs": 1800000000,
                "price": None, "tier": None, "galleries": []}]
    page = _render(pending=pending, thumb_map={"/t/a.png": "/t/thumb_0000.png"})
    assert "Tickler" in page
    assert '00000000-0000-4000-8000-000000000001' in page   # PENDING payload injected
    assert "__PENDING__" not in page and "__CARDS__" not in page


def test_original_query_decodes_verbatim_for_the_allow_list():
    """/original decodes the path arg verbatim, so the allow-list (path in
    thumb_map) sees the real path — a non-listed path can't sneak through."""
    q = urllib.parse.parse_qs(
        urllib.parse.urlparse("/original?path=%2Fetc%2Fpasswd").query)
    assert q.get("path", [""])[0] == "/etc/passwd"
```

- [ ] **Step 4: Flip the remaining `PAGE_TEMPLATE` import**

In `tests/test_scheduling.py`, replace
`from publicator.publish_next import PAGE_TEMPLATE` with
`from publicator.webui.page import PAGE_TEMPLATE`.

- [ ] **Step 5: Run the suite**

Run: `nix develop -c pytest -q`
Expected: PASS, one more test than Task 4 (the pending-card test).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(webui): extract the page template and a pure render_page"
```

---

### Task 6: Extract `webui/server.py` and delete `publish_next.py`

**Files:**
- Create: `src/publicator/webui/server.py`
- Delete: `src/publicator/publish_next.py`
- Modify: `src/publicator/apps/publish_next.py`

**Interfaces:**
- Consumes: everything produced by Tasks 2–5.
- Produces: `publicator.webui.server.{GalleryHandler, serve(thumb_dir, thumb_map, candidate_paths, pending, args, config) -> dict | None}`

- [ ] **Step 1: Create `src/publicator/webui/server.py`**

Move what is left of `publish_next.py` — `GalleryHandler` (with the adapter
`_build_page` from Task 5) and `serve` — plus its module log line
`log = logging.getLogger("publicator.gallery")`. Imports:
`json`, `logging`, `os`, `subprocess`, `urllib.parse`,
`BaseHTTPRequestHandler`/`ThreadingHTTPServer` from `http.server`, plus:

```python
from publicator.config import validate_publications
from publicator.deviantart import publish_batch
from publicator.images import guess_mime
from publicator.llm_meta import DEFAULT_MODEL, generate_metadata
from publicator.scheduling import existing_ts, schedule_data, zone
from publicator.store import apply_update, atomic_write_json, write_publications
from publicator.webui.page import render_page
```

Module docstring:

```python
"""The human-review gallery: a ThreadingHTTPServer on 127.0.0.1 plus its
JSON endpoints (/delete, /ai, /stage, /update, /publish)."""
```

- [ ] **Step 2: Delete the emptied module**

```bash
git rm src/publicator/publish_next.py
```

It must be empty of definitions by now. If `rg -n "^(def|class|[A-Z_]+ =)" src/publicator/publish_next.py`
prints anything before deleting, move that symbol to its module first.

- [ ] **Step 3: Repoint the app**

In `src/publicator/apps/publish_next.py` change the import to:

```python
from publicator.images import find_candidates, generate_thumbnails
from publicator.webui.server import serve
```

- [ ] **Step 4: Run the suite**

Run: `nix develop -c pytest -q`
Expected: PASS, same count as Task 5.

- [ ] **Step 5: Prove the gallery actually serves**

```bash
tmp=$(mktemp -d); mkdir -p "$tmp/picked"
printf '[]' > "$tmp/publications.json"
printf 'publicable = ["picked"]\n[schedule]\ntimezone = "Europe/Paris"\nday = "tuesday"\nhour = 20\nper_slot = 2\n' > "$tmp/publicator.toml"
nix develop -c convert -size 64x64 xc:navy "$tmp/picked/a.png"
(cd "$tmp" && nix develop /home/theta/.vault/repos/publicator.py -c python -m publicator.apps.publish_next --port 8799 &) 
sleep 8
curl -s http://127.0.0.1:8799/ | head -c 400
pkill -f "publicator.apps.publish_next"
```

Expected: HTML containing `<title>Publish next</title>` and a `class="card"`
block. (Firefox may also open; close it.)

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(webui): extract the gallery server; publish_next.py is gone"
```

---

### Task 7: Docs, stale-reference sweep, end-to-end verification

**Files:**
- Modify: `CLAUDE.md`, `AGENTS.md`, `.claude/skills/publish-deviantart/SKILL.md`

- [ ] **Step 1: Update the skill's invocation**

In `.claude/skills/publish-deviantart/SKILL.md` line ~33, replace
`python ./echo_first_unpublished_publication_data.py` with:

```sh
nix run <publicator.py>#echo-first -- --data-dir .
```

and in the sentence at line ~43 replace `publish_next.py` with
`the publish-next app`. Do NOT touch the numbered "### Steps" list — it is the
source of truth for `--check-steps`.

- [ ] **Step 2: Rewrite the architecture prose in `CLAUDE.md`**

Update every module path in the "Architecture" and "State & data" sections:
`publish_next.py` → `publicator.webui.server` (+ `apps/publish_next.py` for the
CLI), `da_publish.py` → `publicator/deviantart.py`, `validate.py::load_config` →
`publicator/config.py::load_config`, `echo_first_unpublished_publication_data.py`
→ `publicator/entries.py`, `pw_daemon.py` → `publicator/apps/pw_daemon.py`.
Add a short "Layout" paragraph naming the four new modules
(`images`, `scheduling`, `store`, `webui/{page,server}`) and stating that code
assets resolve via `__file__` inside the package while data still resolves
against CWD/`--data-dir`.

- [ ] **Step 3: Mirror into `AGENTS.md`**

`AGENTS.md` carries the same content; apply the same edits so the two stay in
sync (its one-line `.webp` fact stays).

- [ ] **Step 4: Sweep for stale references**

Run:

```bash
rg -n --hidden -g '!.git' -g '!docs/superpowers' \
  'publish_next\.py|da_publish\.py|\bvalidate\.py|llm_meta\.py|pw_daemon\.py|echo_first_unpublished_publication_data|--selfcheck'
```

Expected: no hits outside `docs/superpowers/` (historical plans/specs are
allowed to name the old paths — they are a record of what was true then).

- [ ] **Step 5: Full verification sweep**

```bash
nix develop -c pytest -q
nix run .#check-steps
tmp=$(mktemp -d); echo '[]' > "$tmp/publications.json"
nix run .#validate     -- --data-dir "$tmp"
nix run .#da-publish   -- --data-dir "$tmp"
nix run .#publish-next -- --data-dir "$tmp"
nix run .#echo-first   -- --data-dir "$tmp" || true   # SystemExit: no unpublished publication
```

Expected: pytest PASS (≥30); `check-steps OK: 12 steps in sync with the skill`;
`Valid`; `nothing to publish (no state=unpublished entry)`;
`Nothing to publish (no new picks, no pending queue).`; `no unpublished publication`.

- [ ] **Step 6: Browser check of the gallery**

Serve a one-image data dir as in Task 6 Step 5, then drive it with the
`playwright-cli` skill: open `http://127.0.0.1:8799/`, confirm the card renders
with its thumbnail, click **Add**, and confirm the Schedule field is pre-filled
with a Tuesday 20:00 wall clock and the preset dropdown lists the profile.
Screenshot for the record, then stop the server.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "docs: point CLAUDE.md, AGENTS.md and the skill at the new layout"
```

---

## Verification Summary

| Gate | Baseline (before) | Required (after) |
|---|---|---|
| `nix develop -c pytest -q` | 13 passed | ≥30 passed, 0 failed |
| `nix run .#check-steps` | 12 steps in sync | 12 steps in sync |
| `nix run .#validate -- --data-dir <tmp>` | `Valid` | `Valid` |
| `nix run .#da-publish -- --data-dir <tmp>` | `nothing to publish` | `nothing to publish` |
| `nix run .#publish-next -- --data-dir <tmp>` | `Nothing to publish` | `Nothing to publish` |
| gallery renders in a real browser | worked | works, preset pre-fills |
| `--selfcheck` × 3 | OK | removed (assertions live in `tests/`) |
