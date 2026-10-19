# Configuration (publicator.toml)

- **Does:** `load_config(data_dir)` reads `<data_dir>/publicator.toml` with stdlib `tomllib` into one flat dict (DA allow-lists, candidate dirs, tags path, raw `[schedule]`, compiled `[prompts]`), empty defaults when absent.
- **Run:** `nix run .#ui -- --data-dir <data-dir>` loads it (library only; `--data-dir` defaults to CWD)
- **Run:** `nix run .#da-publish -- --data-dir <data-dir>` loads it through `deviantart.configure`
- **Run:** `nix run .#validate -- --data-dir <data-dir>` loads it; fastest smoke check that a real file parses
- **Run:** `nix run .#prompt-audit -- --data-dir <data-dir>` loads it
- **Code:** `src/publicator/config.py` (`load_config`, `_load_prompts`, `validate_publications`, `SCHEMA`), `src/publicator/deviantart.py` (`configure`, `DATA_DIR`, `TAGS_FILE`), `src/publicator/scheduling.py` (`zone`, `schedule_profiles`, `WEEKDAY`), `src/publicator/prompts.py` (`from_config`), `src/publicator/store.py` (validated mutations), `src/publicator/webui/page.py` (`render_page`), `src/publicator/webui/server.py` (`serve`), `src/publicator/apps/ui.py` (`main`), `src/publicator/apps/da_publish.py` (`main`), `src/publicator/apps/validate.py` (`main`), `src/publicator/apps/prompt_audit.py` (`main`, `audit`)
- **Tests:** `tests/test_config.py` (defaults, `[schedule]` pass-through, prompt grammar acceptance and rejection, tier/gallery allow-lists), `tests/test_deviantart.py` (`configure` resolves tags file under data dir, or leaves it unset), `tests/test_store.py` (validated store mutations), `tests/test_scheduling.py` (flat config becomes one profile, defaults, invalid profiles, day names), `tests/test_prompts.py` (`from_config` None without section or repo dir, archive otherwise), `tests/conftest.py` (`_restore_data_dir_globals` autouse fixture resets `configure` globals)
- **Config:** `publicator.toml`, every key, see Keys; the UI's Search path choices are a runtime subset of `publicable` and never modify TOML. Absent or empty `[prompts]` gives `prompts = None` (feature off).
- **Data:** `publicator.toml` (read only; missing file is not an error); `<tags>` (read by `_step_add_tags` at STEPS entry 8); `<publicable[i]>/` (scanned by the UI and prompt-audit; Search can choose a runtime subset only); `<prompts.repo>/` (git repo digested by `prompts.PromptArchive` when set and present); `src/publicator/publicationsSchema.json` (code asset via `__file__`, parsed once at import, never read from the data dir)
- **Decisions:** The data repository owns filename grammar and digest choice; publicator depends only on named roles, expressed with stdlib `re` rather than a custom parser. See [Prompt grammar ownership](#prompt-grammar-ownership).
- **Verify:** `nix develop -c pytest tests/test_config.py -q`
- **Verify:** `nix develop -c pytest tests/test_deviantart.py tests/test_store.py tests/test_scheduling.py tests/test_prompts.py -q`
- **Verify:** `nix develop -c pytest -q`
- **Verify:** `nix run .#validate -- --data-dir <data-dir>` (prints `Valid`, or raises from `_load_prompts` / `validate_publications`; never touches `[schedule]`)

## Keys

| key | type | default | meaning | consumer |
|---|---|---|---|---|
| `publicable` | list[str] | `[]` | dirs relative to data dir scanned for candidates | `images.find_candidates`, `serve` (UI, prompt-audit) |
| `tags` | str | None | tag list path, one tag per line | `deviantart.configure` -> `TAGS_FILE`, `_step_add_tags` |
| `[deviantart] tiers` | list[str] | `[]` | tier allow-list; the UI renders checkboxes and schedule profiles can preselect names; when `tiers` and `galleries` are both empty the card form omits both fields | `validate_publications`, `page.render_page` |
| `[deviantart] galleries` | list[str] | `[]` | allow-list, same rule as `tiers` | `validate_publications`, `page.render_page` |
| `[schedule] timezone` | str | `"Europe/Paris"` | zoneinfo name; the only flat key still applied when `profiles` is present | `scheduling.zone` |
| `[schedule] frequency` | str | `"weekly"` | anything else raises `NotImplementedError` | `scheduling.schedule_profiles` |
| `[schedule] day` | str | `"tuesday"` | weekday name, must be a key of `scheduling.WEEKDAY` | `scheduling.schedule_profiles` |
| `[schedule] hour` | int | 20 | LOCAL hour in `timezone`, not UTC | `scheduling.schedule_profiles` |
| `[schedule] per_slot` | int | 2 | must be `>= 1` | `scheduling.schedule_profiles` |
| `[schedule] default_price` | number | unset | flat-profile price default; finite and `>= 0`, applied like `default_tiers` when first selecting DA in Phase 4 | `scheduling.schedule_profiles`, `store.select_publication_platforms` |
| `[[schedule.profiles]]` | array of tables | absent | `{name, frequency, day, hour, per_slot, default_tiers?, default_price?}` with distinct `(day, hour)`; `default_tiers` names configured tiers and `default_price` is a finite nonnegative number, initializing new Phase 4 DA selections only; when present the flat keys are ignored except `timezone`; `[]` raises `ValueError` | `scheduling.schedule_profiles`, `schedule_data` |
| `[prompts] repo` | str | required | git repo path relative to data dir | `_load_prompts`, `prompts.from_config` |
| `[prompts] filename` | regex str | required | needs a `(?P<version>...)` group, optional `(?P<lineage>...)`; compiled into the returned key `pattern` | `_load_prompts`, `prompts.from_config` |
| `[prompts] version_hash` | str | required | must be in `hashlib.algorithms_available` | `_load_prompts`, `prompts.PromptArchive` |

## How it works

Apps resolve configuration against --data-dir/CWD, not the code checkout.
Filename grammar validation happens at load; schedule validation happens when
slot data is requested. Store validation enforces DeviantArt option allow-lists
on all mutations, including delivery successes. Dispatcher configuration is
passed through to publisher preflight and persistence so --json outside the
data directory does not accidentally use another allow-list.

### Prompt grammar ownership

`identify_image.sh` in the data repository produces image names; that repository's
`publicator.toml` declares how to interpret them. Publicator knows the required
`version` role (the exact content digest) and optional `lineage` role (a weak file
hint), not the separators, field order, UUID format, or digest algorithm. The
instance UUID is deliberately not captured: the resolver has no use for it.
For example, the existing data convention can be declared as:

```toml
[prompts]
repo = "huggingface_prompts"
filename = '^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[^.]+$'
version_hash = "sha1"
```

This is an example, **not a fallback grammar**. Omitting `[prompts]` disables
lookup; publicator must not guess a convention. `_load_prompts` compiles and
validates once at the configuration boundary. Tests use a different filename
shape, `sha256`, and no lineage group to prove that the contract is role-based.

Hardcoding the shape would turn a data-repo naming change into a code release.
A template mini-language would need new ambiguity/greediness rules for basenames
containing separators; fixed separators and field lists assume today's shape.
Parsing the producer's shell script at runtime is worse than declaring the regex.
Named `re` groups already express the needed contract without another parser.

The producer and declaration can still drift across repositories. Prompt-audit's
`parsed: N of M` exposes this: a drop to zero calls for a TOML fix, not a code
fallback. Archive lookup and its separate `nearest` health metric belong to
[prompt mapping](prompt-mapping.md).

## Invariants

- **A missing `publicator.toml` never crashes.** Apps must run on an unconfigured data dir and fail later with a domain error, not `FileNotFoundError`; `load_config` returns `{tiers:[], galleries:[], publicable:[], tags:None, schedule:{}, prompts:None}` (`test_config.py::test_load_config_defaults_when_file_missing`).
- **Config and every path key resolve under the data dir; code assets via `__file__`.** Code/data split, the repo holds no account content; `deviantart.configure` (`DATA_DIR / tags`), `prompts.from_config` (`data_dir/repo`), `config.SCHEMA` (`test_deviantart.py::test_configure_resolves_tags_file_under_data_dir`). UI Search accepts only a runtime subset of configured `publicable` paths; it cannot accept arbitrary filesystem paths or write the TOML.
- **`[prompts]` is validated at load; nothing in `src/` hardcodes the filename shape or `sha1`.** A bad pattern fails at the boundary, not while rendering a card; `_load_prompts`; `prompts.py` uses `hashlib.new(self.version_hash, ...)` (`test_config.py::test_load_config_rejects_*`, `::*_without_lineage_group`).
- **`[schedule]` passes through `load_config` verbatim.** One place owns cadence math so the browser never does weekday/hour work; profile/default validation lives in `scheduling.schedule_profiles` / `scheduling.schedule_data`, timezone defaults in `scheduling.zone` (`test_config.py::test_load_config_reads_*_schedule*`, `test_scheduling.py::test_invalid_profiles_*`).
- **Every written tier / gallery must be in the `[deviantart]` allow-lists.** DA's comboboxes match exact text at STEPS entries 10-11 (`_pick_in_combo`); an unlisted value fails mid-submit after upload and leaves a draft; `validate_publications` guards store mutations and publisher preflights (`test_config.py::test_validate_rejects_*`).
- Relies on `deviantart.configure()` mutating process-global module state (`DATA_DIR`/`LOGIN_DIR`/`SESSION_DIR`/`TAGS_FILE`, `entries.DATA_DIR`) that `tests/conftest.py::_restore_data_dir_globals` resets between tests, owned by `docs/features/deviantart-publish.md`.

## Gotchas

- `load_config()["prompts"]["pattern"]` is a compiled `re.Pattern` keyed `pattern`; the TOML key is `filename`. `tests/test_prompts.py`, `tests/test_prompt_audit.py` and `prompts.from_config` all use `pattern`; none go through `filename`.
- `[schedule]` errors surface late: bad cadence fields are rejected by `scheduling.schedule_profiles`, while malformed or unconfigured `default_tiers` are rejected by `schedule_data` at gallery startup (`webui.server.serve`), after `find_candidates` has scanned.
- An unknown `timezone` raises from `ZoneInfo(...)` in `scheduling.zone` at its first call instead; da-publish, validate and prompt-audit never reach `[schedule]` validation at all.
- `hour` is a LOCAL hour in `[schedule] timezone`, not UTC; when `profiles` is present, flat `frequency`/`day`/`hour`/`per_slot` are ignored and only `timezone` still applies.
- `tags` unset does not fail at startup: da-publish runs STEPS entries 1-7 against a live DA draft before entry 8 `_step_add_tags` raises `RuntimeError("no 'tags' path configured in publicator.toml")`, leaving a draft on DA.
- Store calls without config fall back to CWD; the HTTP layer always passes the loaded data-directory config.
- `da_publish` configures its publisher directly; the UI configures registered publishers at dispatch.
- `[prompts]` set but `<data_dir>/<repo>` missing is NOT a load error: `from_config` returns None and every lookup is `Unknown`; `nix run .#prompt-audit` reporting `parsed: 0` is the signal (`tests/test_prompt_audit.py`).
- An empty `[prompts]` table is treated as absent, whereas `[prompts]` with only `repo` raises for the missing `filename`.

## Open items

- Whether `[schedule]` should be validated in `load_config` (fail-fast like `[prompts]`) rather than at gallery startup in `scheduling.schedule_profiles`/`schedule_data`; currently inconsistent between the two sections.
- Standalone store calls must pass the intended config when CWD differs from the data directory.
