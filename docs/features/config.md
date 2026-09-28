# Configuration (publicator.toml)

- **Does:** `load_config(data_dir)` reads `<data_dir>/publicator.toml` with stdlib `tomllib` into one flat dict (DA allow-lists, candidate dirs, tags path, raw `[schedule]`, compiled `[prompts]`), empty defaults when absent.
- **Run:** `nix run .#publish-next -- --data-dir <data-dir>` loads it (library only; `--data-dir` defaults to CWD)
- **Run:** `nix run .#da-publish -- --data-dir <data-dir>` loads it through `deviantart.configure`
- **Run:** `nix run .#validate -- --data-dir <data-dir>` loads it; fastest smoke check that a real file parses
- **Run:** `nix run .#prompt-audit -- --data-dir <data-dir>` loads it
- **Code:** `src/publicator/config.py` (`load_config`, `_load_prompts`, `validate_publications`, `SCHEMA`), `src/publicator/deviantart.py` (`configure`, `DATA_DIR`, `TAGS_FILE`), `src/publicator/scheduling.py` (`zone`, `schedule_profiles`, `WEEKDAY`), `src/publicator/prompts.py` (`from_config`), `src/publicator/store.py` (`write_publications`), `src/publicator/webui/page.py` (`tier_gallery_fields`), `src/publicator/webui/server.py` (`serve`), `src/publicator/apps/publish_next.py` (`main`), `src/publicator/apps/da_publish.py` (`main`), `src/publicator/apps/validate.py` (`main`), `src/publicator/apps/prompt_audit.py` (`main`, `audit`)
- **Tests:** `tests/test_config.py` (defaults, `[schedule]` pass-through, prompt grammar acceptance and rejection, tier/gallery allow-lists), `tests/test_deviantart.py` (`configure` resolves tags file under data dir, or leaves it unset), `tests/test_store.py` (`write_publications` falls back to `load_config` when config is None), `tests/test_scheduling.py` (flat config becomes one profile, defaults, invalid profiles, day names), `tests/test_prompts.py` (`from_config` None without section or repo dir, archive otherwise), `tests/conftest.py` (`_restore_data_dir_globals` autouse fixture resets `configure` globals)
- **Config:** `publicator.toml`, every key, see Keys; absent or empty `[prompts]` gives `prompts = None` (feature off).
- **Data:** `publicator.toml` (read only; missing file is not an error); `<tags>` (read by `_step_add_tags` at STEPS entry 8); `<publicable[i]>/` (scanned by publish-next and prompt-audit); `<prompts.repo>/` (git repo digested by `prompts.PromptArchive` when set and present); `src/publicator/publicationsSchema.json` (code asset via `__file__`, parsed once at import, never read from the data dir)
- **Decisions:** `docs/adr/0003-declare-the-filename-grammar-in-publicator-toml.md`
- **Verify:** `nix develop -c pytest tests/test_config.py -q`
- **Verify:** `nix develop -c pytest tests/test_deviantart.py tests/test_store.py tests/test_scheduling.py tests/test_prompts.py -q`
- **Verify:** `nix develop -c pytest -q`
- **Verify:** `nix run .#validate -- --data-dir <data-dir>` (prints `Valid`, or raises from `_load_prompts` / `validate_publications`; never touches `[schedule]`)

## Keys

| key | type | default | meaning | consumer |
|---|---|---|---|---|
| `publicable` | list[str] | `[]` | dirs relative to data dir scanned for candidates | `images.find_candidates`, `serve` (publish-next, prompt-audit) |
| `tags` | str | None | tag list path, one tag per line | `deviantart.configure` -> `TAGS_FILE`, `_step_add_tags` |
| `[deviantart] tiers` | list[str] | `[]` | allow-list; when `tiers` and `galleries` are both empty the card form omits the tier/gallery fields | `validate_publications`, `page.tier_gallery_fields` |
| `[deviantart] galleries` | list[str] | `[]` | allow-list, same rule as `tiers` | `validate_publications`, `page.tier_gallery_fields` |
| `[schedule] timezone` | str | `"Europe/Paris"` | zoneinfo name; the only flat key still applied when `profiles` is present | `scheduling.zone` |
| `[schedule] frequency` | str | `"weekly"` | anything else raises `NotImplementedError` | `scheduling.schedule_profiles` |
| `[schedule] day` | str | `"tuesday"` | weekday name, must be a key of `scheduling.WEEKDAY` | `scheduling.schedule_profiles` |
| `[schedule] hour` | int | 20 | LOCAL hour in `timezone`, not UTC | `scheduling.schedule_profiles` |
| `[schedule] per_slot` | int | 2 | must be `>= 1` | `scheduling.schedule_profiles` |
| `[[schedule.profiles]]` | array of tables | absent | `{name, frequency, day, hour, per_slot}` with distinct `(day, hour)`; when present the flat keys are ignored except `timezone`; `[]` raises `ValueError` | `scheduling.schedule_profiles` |
| `[prompts] repo` | str | required | git repo path relative to data dir | `_load_prompts`, `prompts.from_config` |
| `[prompts] filename` | regex str | required | needs a `(?P<version>...)` group, optional `(?P<lineage>...)`; compiled into the returned key `pattern` | `_load_prompts`, `prompts.from_config` |
| `[prompts] version_hash` | str | required | must be in `hashlib.algorithms_available` | `_load_prompts`, `prompts.PromptArchive` |

## How it works

1. Each app's `main` (`publish_next.py`, `validate.py`, `prompt_audit.py`) resolves `data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()`; echo-first, generate-meta and login never load the config.
2. `load_config(data_dir)` reads `<data_dir>/publicator.toml` with `tomllib.loads`; a missing file yields `{}`.
3. `load_config` builds the flat dict: `tiers`/`galleries` from `[deviantart]`, `publicable`, `tags`, `schedule` (a raw dict copy, not validated here) and `prompts` via `_load_prompts`.
4. `_load_prompts` is the only load-time validation: requires `repo` and `filename`, compiles the regex, demands a `version` group, checks `version_hash` against `hashlib.algorithms_available`; returns `{repo, pattern, version_hash}` or None.
5. `publish_next.main` passes the dict to `deviantart.configure(args.data_dir, config)`, which sets `DATA_DIR`/`LOGIN_DIR`/`SESSION_DIR`, `TAGS_FILE = DATA_DIR / tags` (or None) and retargets `entries.DATA_DIR` via `set_data_dir`.
6. `da_publish.main` calls `configure(a.data_dir)` with no config, so `configure` runs `load_config(DATA_DIR)` itself.
7. `publish_next.main` resolves `publicable` to `[str(data_dir / d) ...]` and hands it to `images.find_candidates` and to `serve`.
8. `scheduling.zone` reads `timezone` lazily, wherever a timestamp is resolved or labelled (`store.py`, `server.py`, `page.py`); `scheduling.schedule_profiles` is reached only via `schedule_data`, once at startup in `webui.server.serve`.
9. `validate_publications` enforces `tiers`/`galleries` on every write (`store.write_publications`) and in the validate app; `page.tier_gallery_fields` renders them as the card's tier `<select>` and gallery checkboxes.
10. `prompts.from_config(data_dir, config)` turns `prompts` into a `PromptArchive`, returning None when `<data_dir>/<repo>` is not a directory.

## Invariants

- **A missing `publicator.toml` never crashes.** Apps must run on an unconfigured data dir and fail later with a domain error, not `FileNotFoundError`; `load_config` returns `{tiers:[], galleries:[], publicable:[], tags:None, schedule:{}, prompts:None}` (`test_config.py::test_load_config_defaults_when_file_missing`).
- **Config and every path key resolve under the data dir; code assets via `__file__`.** Code/data split, the repo holds no account content; `deviantart.configure` (`DATA_DIR / tags`), `prompts.from_config` (`data_dir/repo`), `config.SCHEMA` (`test_deviantart.py::test_configure_resolves_tags_file_under_data_dir`).
- **`[prompts]` is validated at load; nothing in `src/` hardcodes the filename shape or `sha1`.** ADR 0003, a bad pattern fails at the boundary, not while rendering a card; `_load_prompts`; `prompts.py` uses `hashlib.new(self.version_hash, ...)` (`test_config.py::test_load_config_rejects_*`, `::*_without_lineage_group`).
- **`[schedule]` passes through `load_config` verbatim.** One place owns cadence math so the browser never does weekday/hour work; validation and defaults live in `scheduling.schedule_profiles` / `scheduling.zone` (`test_config.py::test_load_config_reads_*_schedule*`, `test_scheduling.py::test_invalid_profiles_*`).
- **Every written tier / gallery must be in the `[deviantart]` allow-lists.** DA's comboboxes match exact text at STEPS entries 10-11 (`_pick_in_combo`); an unlisted value fails mid-submit after upload and leaves a draft; `validate_publications` via `store.write_publications` (`test_config.py::test_validate_rejects_*`).
- Relies on `deviantart.configure()` mutating process-global module state (`DATA_DIR`/`LOGIN_DIR`/`SESSION_DIR`/`TAGS_FILE`, `entries.DATA_DIR`) that `tests/conftest.py::_restore_data_dir_globals` resets between tests, owned by `docs/features/deviantart-publish.md`.

## Gotchas

- `load_config()["prompts"]["pattern"]` is a compiled `re.Pattern` keyed `pattern`; the TOML key is `filename`. `tests/test_prompts.py`, `tests/test_prompt_audit.py` and `prompts.from_config` all use `pattern`; none go through `filename`.
- `[schedule]` errors surface late: a bad `day`, `frequency = "monthly"`, `per_slot = 0`, colliding profiles or `profiles = []` raise only in `scheduling.schedule_profiles` at gallery startup (`webui.server.serve`), after `find_candidates` has scanned.
- An unknown `timezone` raises from `ZoneInfo(...)` in `scheduling.zone` at its first call instead; da-publish, validate and prompt-audit never reach `[schedule]` validation at all.
- `hour` is a LOCAL hour in `[schedule] timezone`, not UTC; when `profiles` is present, flat `frequency`/`day`/`hour`/`per_slot` are ignored and only `timezone` still applies.
- `tags` unset does not fail at startup: da-publish runs STEPS entries 1-7 against a live DA draft before entry 8 `_step_add_tags` raises `RuntimeError("no 'tags' path configured in publicator.toml")`, leaving a draft on DA.
- `store.write_publications(config=None)` falls back to `load_config(Path.cwd())` for the allow-lists only, timezone to `DEFAULT_TZ`; the webui always passes config, a standalone caller run elsewhere rejects any tier/gallery (`test_store.py`).
- `da_publish.main` calls `configure(a.data_dir)` without a config and re-reads `publicator.toml` itself; `publish_next` passes the already-loaded dict. A monkeypatched config in one path is not seen by the other.
- `[prompts]` set but `<data_dir>/<repo>` missing is NOT a load error: `from_config` returns None and every lookup is `Unknown`; `nix run .#prompt-audit` reporting `parsed: 0` is the signal (`tests/test_prompt_audit.py`).
- An empty `[prompts]` table is treated as absent, whereas `[prompts]` with only `repo` raises for the missing `filename`.

## Open items

- Whether `[schedule]` should be validated in `load_config` (fail-fast like `[prompts]`) rather than at gallery startup in `scheduling.schedule_profiles`; currently inconsistent between the two sections.
- `store.write_publications`' CWD fallback (allow-lists from CWD, timezone from the default) vs the data-dir convention used everywhere else; only `tests/test_store.py` exercises it.
