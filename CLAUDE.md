# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

DeviantArt does not accept .webp format for publications.

## What this is

Personal tooling to publish artwork to DeviantArt: a human-review gallery UI that
generates AI metadata, schedules posts, and drives a real browser through the
DeviantArt submit flow. This repo is **code only** — the publication *data*
(images, `publications.json`, `publicator.toml`, browser-session dirs) lives in a
separate **data directory** (in practice `../Art`).

Code assets (JSON schema, `tags/`) resolve via `__file__`; data + runtime state
resolve against **CWD**. So apps must be run *from the data dir*, or given
`--data-dir`. `nix run <this>#<app>` is expected to be invoked from `../Art`.

## Commands

Everything runs inside the Nix dev shell (`nix develop` provides python312 +
pytest + imagemagick + Playwright browsers + the `claude` CLI, and sets
`PLAYWRIGHT_BROWSERS_PATH`). There is no pip/venv.

```sh
nix develop -c pytest -q                              # full test suite
nix develop -c pytest tests/test_scheduling.py -q     # one file
nix run <this>#check-steps                            # drift guard: STEPS registry vs the publish-deviantart skill
```

There is no linter. Correctness is guarded by pytest (`tests/`).
Some scheduler tests shell out to `node` to run the served page's client JS and
`pytest.skip` if `node` is absent.

### Apps (run from the data dir, e.g. `cd ../Art`)

```sh
nix run <this>#login          # one-time DeviantArt sign-in in a REAL Firefox (see PerimeterX below)
nix run <this>#publish-next   # the gallery UI: review picked/, AI metadata, schedule, batch-publish
nix run <this>#da-publish -- --data-dir <dir> [--uuid <id>]   # publish ONE publications.json entry
nix run <this>#validate       # validate publications.json against publicationsSchema.json
nix run <this>#echo-first     # path/title/schedule of the first state=unpublished entry
nix run <this>#check-steps    # cheap CI drift guard (no browser/data deps)
```

## Architecture

**Layout.** `src/publicator/` is a package: `config.py`, `entries.py`,
`images.py`, `scheduling.py`, `store.py`, `deviantart.py`, `llm_meta.py` hold
the logic; `webui/{page,server}.py` is the gallery (a pure `render_page` plus
the `ThreadingHTTPServer` that calls it); `apps/{publish_next,da_publish,
validate,echo_first,pw_daemon}.py` are the thin CLI entry points the flake's
`nix run` apps invoke. Code assets (JSON schema, `tags/`) resolve via
`__file__` inside the package; data + runtime state still resolve against
CWD/`--data-dir`, per the code/data split above.

**The publish pipeline.**
1. `#login` opens a genuine, flake-pinned Firefox for a human sign-in and writes
   the logged-in profile to `.deviantart-login/` in the data dir. This is not
   automatable: DeviantArt sits behind **PerimeterX**, which blocks *every*
   Playwright browser at the login page (Playwright forces `navigator.webdriver`).
2. `publicator.webui.server` (`#publish-next`, launched by `apps/publish_next.py`)
   is the orchestrator — a `ThreadingHTTPServer` gallery on `127.0.0.1` opened in
   Firefox for **human review**, not a headless job. It scans the config's
   `publicable` dirs (e.g. `picked/`), lets you add images to a queue with AI
   title/description and a schedule, writes them to `publications.json`, then
   drives a Firefox persistent context (copied from the `#login` profile)
   through the DA submit flow.
3. `publicator/deviantart.py` is the Playwright submit logic for **one** entry
   (one-way import: `webui.server` / `apps.da_publish` → `deviantart`, no cycle).
   It flips the entry's apparition `state` `unpublished` → `published_or_scheduled`.

**STEPS ↔ skill contract.** `deviantart.py` holds an ordered `STEPS` registry that
mirrors, 1:1, the numbered list in the `publish-deviantart` skill's `SKILL.md`.
`--check-steps` fails loudly if the skill grows a step the code doesn't implement —
run it (or the `check-steps` app) after changing either side.

**AI metadata** (`publicator/llm_meta.py`). Key-free: `llm -m <model>` shells out
to the logged-in `claude` CLI (uses the Claude subscription, no API key). Vision
works by handing the model the image **path** + the Read tool, *not* `llm -a`
(attachments are broken through this plugin). `openrouter/*` model ids are the
exception — they use `-a <image>` and read `$OPENROUTER_KEY` at runtime.

**Scheduling is entirely server-side** (`publicator/scheduling.py`). All
weekday/hour/timezone math lives in Python (`zoneinfo`, `schedule.timezone`,
default `Europe/Paris`). The browser receives absolute slot *instants* and
counts occupancy by exact-timestamp equality — it never reads its own clock,
because `firefox --private-window` with resist-fingerprinting spoofs `Date` to
UTC and would otherwise mis-match local slots. Custom picker times are likewise
resolved server-side on save (`store.resolve_ts`). **Stored timestamps are
LOCAL wall-clock 20:00, never UTC 20:00** — don't reintroduce browser-clock math
in the served page's JS.

**State & data.** `publications.json` is the source of truth (an array of entries,
each with `apparitions[]` carrying `state` + `apparitionTimestampIfDifferentThanSubmission`).
`publicator/entries.py` reports the first `unpublished` entry (by state field,
not git; `apps/echo_first.py` is the CLI wrapper). `publicator/config.py::load_config`
reads `publicator.toml` (`publicable` dirs, `tags` file, `[deviantart]`
tiers/galleries allow-lists, `[schedule]` timezone + weekly `profiles`).
`publicator/apps/pw_daemon.py` is a separate Playwright FIFO daemon (Chromium
persistent context in `.deviantart-session/`) for interactive debugging of the
flow.

## Gotchas

- Run apps from the data dir (or pass `--data-dir`) — see the code/data split above.
- `.webp` is rejected by DeviantArt (top of this file); convert before publishing.
- Login is out-of-band in real Firefox; Playwright cannot pass the PerimeterX wall.
- `AGENTS.md` mirrors the one-line `.webp` fact; this file is the fuller guidance.
