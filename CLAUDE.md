# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

DeviantArt does not accept .webp format for publications.

## What this is

Personal tooling to publish artwork to DeviantArt: a human-review gallery UI that
generates AI metadata, schedules posts, and drives a real browser through the
DeviantArt submit flow. This repo is **code only** — the publication *data*
(images, `publications.json`, `publicator.toml`, browser-session dirs) lives in a
separate **data directory** (in practice `../Art`).

Code assets (the JSON schema) resolve via `__file__`; the tags file, like all
data + runtime state, resolves against the **data dir** (`publicator.toml`'s
`tags` key, joined to `--data-dir`/CWD). So apps must be run *from the data
dir*, or given `--data-dir`. `nix run <this>#<app>` is expected to be invoked
from `../Art`.

## Commands

Everything runs inside the Nix dev shell (`nix develop` provides python312 +
pytest + imagemagick + Playwright browsers + the `claude` CLI, and sets
`PLAYWRIGHT_BROWSERS_PATH`). There is no pip/venv. `pyproject.toml` is metadata plus pytest config: nothing installs the
package (apps run with `PYTHONPATH=src`); the authoritative dependency set is
`flake.nix`'s `pyPkgs`.

```sh
nix develop -c pytest -q                              # full test suite
nix develop -c pytest tests/test_scheduling.py -q     # one file
nix run <this>#check-steps                            # drift guard: STEPS registry vs the publish-deviantart skill
```

There is no linter. Correctness is guarded by pytest (`tests/`).
`tests/test_llm_meta_openrouter.py` calls OpenRouter for real: it **skips with a
warning when offline**, but a reachable network with no `$OPENROUTER_KEY` is a
failure, not a skip.
Some scheduler tests shell out to `node` to run the served page's client JS and
`pytest.skip` if `node` is absent.

### Apps (run from the data dir, e.g. `cd ../Art`)

```sh
nix run <this>#login          # one-time DeviantArt sign-in in a REAL Firefox (see PerimeterX below)
nix run <this>#publish-next   # the gallery UI: review picked/, AI metadata, schedule, batch-publish
nix run <this>#da-publish -- --data-dir <dir> [--uuid <id> | --all] [--headless]   # publish the first pending entry, one by uuid, or all
nix run <this>#generate-meta -- --data-dir <dir> --source openrouter # replace marked metadata
nix run <this>#vision-models  # host-fit verdicts for configured local vision models
nix run <this>#validate       # validate publications.json against publicationsSchema.json
nix run <this>#echo-first     # path/title/schedule of the first metadata-approved pending DA target
nix run <this>#check-steps    # cheap CI drift guard (no browser/data deps)
nix run <this>#prompt-audit   # prompt-mapping coverage: parsed N of M, nearest count
nix run <this>#pw-daemon      # Playwright FIFO daemon for DOM probing (see docs/features/deviantart-publish.md)
```

## Architecture

Per-feature maintenance docs: `docs/README.md` indexes `docs/features/` (one file per
feature, fixed shape: Does/Run/Code/Tests/Config/Data/Decisions/Verify, then flow,
invariants, gotchas). Read the feature file before touching its code.

**Layout.** `src/publicator/` is a package: `config.py`, `entries.py`,
`images.py`, `scheduling.py`, `store.py`, `deviantart.py`, `llm_meta.py`, `prompts.py` hold
the logic; `webui/{page,server,calendar_view,prompt_view}.py` is the UI (a pure `render_page`
plus the `ThreadingHTTPServer` that calls it, and the calendar tab's own pure
renderer); `apps/{publish_next,da_publish,
validate,echo_first,pw_daemon,generate_meta,vision_models,prompt_audit}.py` are the thin CLI entry points the flake's
`nix run` apps invoke. Code assets (the JSON schema) resolve via `__file__`
inside the package; the tags file, like all data + runtime state, still
resolves against CWD/`--data-dir`, per the code/data split above.

**The publish pipeline.**
1. `#login` opens a genuine, flake-pinned Firefox for a human sign-in and writes
   the logged-in profile to `.deviantart-login/` in the data dir. This is not
   automatable: DeviantArt sits behind **PerimeterX**, which blocks *every*
   Playwright browser at the login page (Playwright forces `navigator.webdriver`).
2. `publicator.webui.server` (`#publish-next`, launched by `apps/publish_next.py`)
   is the orchestrator — a `ThreadingHTTPServer` gallery on `127.0.0.1` opened in
   Firefox for **human review**, not a headless job. It scans the config's
   `publicable` dirs (e.g. `picked/`), then persists four distinct phases:
   Pick → Schedule → Describe → Publish. `publications.json` owns completed
   publication states PICKED/SCHEDULED/DESCRIBED/APPEARED, shared title/description,
   and the publication-level apparitionTimestampIfDifferentThanSubmission.
   Apparitions retain delivery states/options and an optional actual `url`.
   Picking never selects a platform or generates metadata. Dates and metadata
   require explicit approval; successful deliveries lock shared content.
   Phase 4 is image rows × registered-platform columns, with persistent selection
   and success-aware retry through `platforms.py`. DeviantArt is the only initial
   executable registration; historical other-platform data remains readable.
   The server rereads saved data on requests and stays open after publishing,
   including history-only startup. No RAM-only queue is authoritative.
   The read-only calendar (`webui/calendar_view.py`) shows every apparition using
   the shared title/date and actual URL, never guessed URL-slug titles.
   Candidate cards and calendar popovers share the prompt renderer; scheduling
   and metadata cards have no prompt/search/delete actions.
   Every mutation validates before atomic replacement under short thread/file
   locks; AI/browser work never holds them. Accepted AI failures save the exact
   [[PUBLICATOR:TODO_GENERATE]] marker in requested fields; blank/marked metadata
   cannot advance or publish. Manual drafts debounce and flush on phase changes.
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
Most free OpenRouter vision models **reject `--schema`**; `run_llm` retries once
with the shape in the prompt and remembers the verdict per model (see
`NO_SCHEMA_SUPPORT`). Don't "fix" a schema failure by chasing a model id that
supports structured outputs — the free ones churn; rationale and the rejected
alternatives are in `docs/adr/0005`. A **429 is a different beast**: the
`:free` tier shares one upstream pool per vendor, so backing off is useless
and `run_llm` walks the other free vision ids after any failed OpenRouter
attempt (same vendor last, using the plugin's cache so ids resolve through
`llm -m`), logging at INFO which model answered. It does not silently switch to
Claude or local Ollama; after the catalogue is exhausted, the last error is
reported (`docs/adr/0008`).
Every AI caller downscales through
`images.thumb_for_ai` first.

**Scheduling is entirely server-side** (`publicator/scheduling.py`). All
weekday/hour/timezone math lives in Python (`zoneinfo`, `schedule.timezone`,
default `Europe/Paris`). Allocation/repacking and occupancy are server-side; the browser never reads its own clock,
because `firefox --private-window` with resist-fingerprinting spoofs `Date` to
UTC and would otherwise mis-match local slots. Custom picker times are likewise
resolved server-side on save (`store.resolve_ts`). **Stored timestamps are
LOCAL wall-clock 20:00, never UTC 20:00** — don't reintroduce browser-clock math
in the served page's JS.

**Thumbnails.** One content-addressed cache, `<data-dir>/.thumbs/<sha512[:32]><ext>`
(`images.thumb_name`/`ensure_thumb`), shared by workflow and calendar and kept across runs —
it is *not* per-run in `/tmp`. Thumbnails are generated on the first request that
needs one, so a cold calendar doesn't stall startup; `/thumbs/<name>` only serves
names the page actually rendered (that allow-list is what keeps paths from being
traversed in). Add `.thumbs/` to the data dir's `.gitignore`.

**Prompt mapping.** A gallery card and a calendar thumbnail alike can show the
generation prompt that produced its image — one block, rendered once by
`webui/prompt_view.py` and handed to both tabs, so they cannot drift apart.
The art repo's `huggingface_prompts` submodule *is* the archive:
`identify_image.sh` commits a prompt before minting the digest that names it,
so a filename references content git holds (it did not, historically — 86% of
prompt versions were never committed and are unrecoverable).
`publicator/prompts.py` digests every blob in that repo, including unreachable
ones, and resolves a filename to one of three types — `Exact`, `Nearest`,
`Unknown`. **`Nearest` deliberately has no `.text`**: a near-miss must not be
renderable as the real prompt. Never resolve a prompt by looking its basename
up at `HEAD`; measured over the real data that is silently wrong on 404 of 426
pairs, because prompts evolve after the image is made.

The **filename grammar is data, not code** — `publicator.toml`'s `[prompts]`
declares a `re` pattern with `version` (required) and `lineage` (optional)
capture groups plus `version_hash`. Nothing in `src/` may hardcode `sha1` or
the filename shape. `nix run <this>#prompt-audit` reports `parsed: N of M`,
which drops to 0 if that grammar drifts from what `identify_image.sh` produces,
and `nearest`, which rising means archive writes are failing. Both apps that
shell out to git (`publish-next`, `prompt-audit`) must list `pkgs.git` in their
flake `runtimeInputs` — `writeShellApplication` pins PATH to those.
Feature doc: `docs/features/prompt-mapping.md`,
decisions in `docs/adr/0001`–`0004`.

**State & data.** `publications.json` is the source of truth. Publication state
records completed phases, while apparitions carry delivery state. Automatic
PICKED dates repack in persisted order; manual pins and approved dates stay
frozen. Shared dates count once regardless of apparition count. `entries.py`
selects metadata-approved pending targets; `config.py` owns data-directory
configuration and option validation. Historical migration is a tested pure
transformation (`migration.py`); bounded title recovery uses a temporary copy
of the named Firefox privacy profile headlessly, **not** a private window.
Never alter the original profile or guess titles from URLs. Specs/plans/trackers
and reports stay in /tmp or managed scratch space, never in Git.
`publicator/apps/pw_daemon.py` is a separate Playwright FIFO daemon (Chromium
persistent context in `.deviantart-session/`) for interactive debugging of the
flow.

## Gotchas

- Run apps from the data dir (or pass `--data-dir`) — see the code/data split above.
- `.webp` is rejected by DeviantArt (top of this file); convert before publishing.
- Login is out-of-band in real Firefox; Playwright cannot pass the PerimeterX wall.
- `AGENTS.md` is a symlink to this file — edit `CLAUDE.md`, never `AGENTS.md`.
- Prompt lookup is content-addressed, never `basename`-at-`HEAD`; and the
  filename grammar lives in `publicator.toml`, not in the code.
