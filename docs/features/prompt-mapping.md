# Prompt mapping (image -> generation prompt)

- **Does:** Resolves an art filename to the prompt that generated it by content-digesting every blob of the data dir's prompt git repo (`Exact`/`Nearest`/`Unknown`), shown per gallery card, searchable, audited.
- **Run:** `nix run .#prompt-audit -- [--data-dir DIR]` (prints parsed/exact/nearest/unknown)
- **Run:** `nix run .#publish-next -- [--data-dir DIR]` (cards show the prompt block; `GET /?prompt=<substring>` searches Exact text, `GET /?lineage=<repo path>` groups images by prompt file)
- **Code:** `src/publicator/prompts.py` (`HEAD_PROBE_TTL`, `PromptArchive`, `from_config`, `rank_paths`, `Nearest`, `resolve_path`), `src/publicator/webui/prompt_view.py` (`render_prompt`, `_stamp`, `_lineage_link`, `_body`), `src/publicator/apps/prompt_audit.py` (`audit`, `format_report`, `main`), `src/publicator/config.py` (`_load_prompts`, `load_config`), `src/publicator/webui/server.py` (`SEARCH_LIMIT`, `GalleryHandler.search`, `GalleryHandler._prompt_html`), `src/publicator/webui/page.py` (`render_page`, `search_enabled`, `__CARDS__`), `flake.nix` (`apps.prompt-audit`, `apps.publish-next`)
- **Tests:** `tests/test_prompts.py` (index, three-state resolve, ranking, HEAD refresh, git failures), `tests/test_prompt_view.py` (Exact/Nearest rendering, escaping, every candidate listed), `tests/test_prompt_audit.py` (counts, feature off), `tests/test_config.py` (`[prompts]` grammar validation and rejections), `tests/test_server.py` (search semantics, feature-off inertness, skipped vs maybe), `tests/test_page.py` (block injection, search box, banners, placeholder survival)
- **Config:** `publicator.toml` `[prompts]`: `repo` (joined to the data dir; dir absent = off silently), `filename` (Python `re` as TOML literal `'...'`; needs `(?P<version>)`, optional `(?P<lineage>)`), `version_hash` (in `hashlib.algorithms_available`; real data `sha1`). Section absent = `config['prompts']` `None`, off. Also `publicable` (dirs walked), `[schedule].timezone` (dates Nearest versions).
- **Data:** `<data-dir>/<prompts.repo>` (the `huggingface_prompts` submodule; read-only via `git cat-file`, `git log --all`, `git rev-parse HEAD`); `<publicable dir>/**` (image filenames parsed with the grammar); `publications.json` (search drops already-published hits via `load_publicated_hashes`; pending entries get prompt blocks); `.thumbs/` (allow-list extended for search results).
- **Decisions:** `docs/adr/0001-archive-prompts-at-identity-mint-time.md`, `docs/adr/0002-resolve-prompts-to-a-three-state-type.md`, `docs/adr/0003-declare-the-filename-grammar-in-publicator-toml.md`, `docs/adr/0004-fail-open-when-prompt-archiving-fails.md`
- **Verify:** `nix develop -c pytest tests/test_prompts.py tests/test_prompt_view.py tests/test_prompt_audit.py tests/test_config.py tests/test_server.py tests/test_page.py -q`
- **Verify:** `rg -n 'sha1|_\[0-9a-f\]\{40\}' src/publicator/` (must print nothing; ADR 0003)
- **Verify:** `nix run .#prompt-audit -- --data-dir <data-dir>` (`parsed` must be non-zero; compare with the baseline and update it after each generation batch: 2026-09-28 parsed 1255 of 4066, exact 455 images / 62 versions, nearest 793 / 355, unknown 7. `nearest` rising means archive writes are failing)

## How it works

1. `config.load_config` -> `_load_prompts` validates `[prompts]` (repo present, filename compiles with a `version` group, `version_hash` in `hashlib.algorithms_available`) and returns `{repo, pattern, version_hash}` or `None`.
2. `prompts.from_config` returns `None` when the section is missing or `<data_dir>/<repo>` is not a dir, else a `PromptArchive`; `webui.server.serve` stores it on `GalleryHandler.archive`, `prompt_audit.audit` builds its own.
3. `PromptArchive.resolve_path(path)`: `parse_identity(basename, pattern)` -> `ImageIdentity(version, lineage)` or `None` (-> `Unknown`, grammar mismatch), then `resolve(identity, near=os.path.dirname(path))`.
4. Every query (`versions`/`paths_for`/`versions_at`) takes `self._lock` and calls `_refresh`: return if probed within `HEAD_PROBE_TTL` (2s); else `git rev-parse HEAD`; if HEAD moved or the index is empty, rebuild and set `_head` last.
5. Rebuild: `_blobs()` walks `cat-file --batch-all-objects --batch` by byte length, digesting each blob with `hashlib.new(version_hash)`; `_history()` walks `log --all --raw --no-abbrev --no-renames --format=%ct` for paths and first commit time.
6. `resolve()`: digest in `versions()` -> `Exact`. No lineage -> `Unknown`. Else `paths_for(lineage)` (all historical paths of that basename), `rank_paths` (dir-name similarity, nothing dropped), `versions_at` newest-first -> `Nearest`, else `Unknown`.
7. Gallery `GET /`: `GalleryHandler.do_GET` -> `_build_page` -> `_prompt_html(candidates + maybe)`: per path, `render_prompt(archive.resolve_path(p), near=dirname(p), tz=schedule tz)` -> `{path: html}`; `Unknown` renders `''` and is dropped.
8. `prompt_view.render_prompt`: `Exact` -> `details.prompt.exact`: `v.paths` as `/?lineage=` links or "path unknown", 7-char digest, escaped `<pre>`; `Nearest` -> `details.prompt.nearest` "prompt NOT ARCHIVED" plus dated `details.pv` per lineage.
9. `GET /?prompt=X` or `/?lineage=P`: `GalleryHandler.search` walks every publicable dir via `collect_images`; `Nearest` -> `skipped` unless P is a candidate path ("maybe"); `Exact` -> kept if X in text (case-insensitive) and P in `version.paths`.
10. `search`, cont.: `truncated` set when a list exceeds `SEARCH_LIMIT` (200) before the cap; survivors sha512-hashed, already-published dropped; `_register_thumbs` extends the `/thumbs` allow-list; `render_page` shows both lists plus banners.
11. prompt-audit: `audit()` iterates `collect_images` over `<data_dir>/<publicable>`, counts files, `archive.parse` -> parsed, `archive.resolve` -> exact/nearest/unknown plus distinct version sets; `format_report` prints `parsed   : N of M files` first.

## Invariants

- **`Nearest` carries no `.text` and no `.version`.** basename@HEAD was wrong on 404 of 426 pairs, so the absent field is enforced by the language; class `Nearest`; `test_prompts.py::test_nearest_has_no_text_attribute`; `test_prompt_view.py::test_nearest_is_labelled_as_not_archived`.
- **`rank_paths` orders candidates and never discards one.** Silently picking a fuzzy winner is the bug class the feature eliminates; `rank_paths` is a `sorted`, no filter; `test_rank_paths_never_discards`, `test_nearest_keeps_every_ambiguous_lineage_ranked_not_filtered`, `test_nearest_lists_every_candidate_lineage`.
- **No filename shape or digest algorithm is hardcoded in `src/`.** The grammar is a data-repo convention; code depends only on the `version`/`lineage` roles and `version_hash`; the `rg` Verify line prints nothing; `_blobs` uses `hashlib.new`; `test_index_uses_the_configured_digest_algorithm`.
- **A superseded prompt version resolves to its own text, never HEAD's.** Prompts evolve after the image is made; `_blobs` uses `--batch-all-objects`; `tests/test_prompts.py::test_resolve_returns_exact_for_a_superseded_version`, `::test_index_keeps_superseded_versions`.
- **Lineage lookup spans all history (`log --all`, `--no-renames`).** A basename gone from HEAD still resolves; `PromptArchive._history`; `tests/test_prompts.py::test_paths_for_finds_a_basename_that_no_longer_exists_at_head`.
- **The digest key is `hashlib.new(version_hash)` over blob content, never the git OID.** Git OIDs hash `blob <len>\0` + content and never equal `sha1sum`; in `_blobs` the oid is only a join key for `_history`; `tests/test_prompts.py::test_index_holds_a_committed_prompt_by_content_digest`.
- **Feature off (`archive=None`): no search form, no prompt blocks, lookups `Unknown`, audit counts files only, nothing raises.** Keeps the gallery usable; `from_config`; `_prompt_html`/`search` early returns; `search_enabled`; `test_search_is_inert_when_the_feature_is_off`; `test_audit_without_a_prompts_section_*`.
- **Any git failure (missing binary, broken repo, non-zero exit) yields `b''` from `PromptArchive._git`, so an empty index.** The review UI must stay usable; `tests/test_prompts.py::test_missing_repo_yields_an_empty_index`.
- Relies on `[prompts]` validated at config load (`ValueError`, never at first lookup), owned by `docs/features/config.md`.
- **Index cached on `git rev-parse HEAD`, rebuilt under a `threading.Lock`, probed at most every `HEAD_PROBE_TTL` (2s).** Each archive write is a commit (ADR 0001) so HEAD is the invalidation key; `_refresh`; `test_index_refreshes_when_head_moves`, `test_index_does_not_refresh_within_the_head_probe_ttl`.
- Relies on `?prompt=` matching Exact text only, Nearest shown as `N candidates skipped`, over-`SEARCH_LIMIT` results announced as truncated (`GalleryHandler.search`), owned by `docs/features/gallery-ui.md`.
- **Prompt text is html-escaped and `__CARDS__` is the last placeholder replaced.** Prompt files are arbitrary text; `prompt_view._body`/`_lineage_link`; `render_page` final `.replace('__CARDS__', ...)`; `test_exact_escapes_html_in_the_prompt`; `test_prompt_text_containing_a_placeholder_survives_as_literal_text`.
- **webui imports prompts; `prompts.py` imports only stdlib.** Same one-way rule as `deviantart.py`, keeps the resolver testable without the server; `prompts.py` import block.
- **Nearest ranking compares the image's directory to prompt dirs, never the full path.** A hex-heavy filename scores spuriously high against hex-like dir names; `resolve_path` (`near=os.path.dirname(path)`), `_similarity`; `tests/test_prompts.py::test_resolve_path_ranks_nearest_by_the_images_own_directory`.

## Gotchas

- `git log --no-renames` is required: with rename detection a line is `R100\tp/a\tp/b` and `_history`'s single `partition()` would store the bogus path `p/a\tp/b`.
- The `cat-file --batch` stream is walked by declared byte length (payloads may contain newlines); a malformed record is skipped, not `break`-ed (`test_blobs_skips_malformed_records_instead_of_truncating_or_raising`).
- A missing `git` is silent: `_git` catches `OSError`, every lookup is `Unknown`, audit shows exact 0 / nearest 0. Both git-shelling apps must list `pkgs.git` in `flake.nix` runtimeInputs.
- `HEAD_PROBE_TTL` = 2s: a prompt committed mid-render appears only on a reload more than 2s later. Tests that commit then query must reset `archive._probed = None` or they flake.
- Grammar drift: `identify_image.sh` produces filenames, `publicator.toml` declares the grammar, two repos. Symptom: prompt-audit `parsed : 0 of M`. Fix the toml, not `src/`.
- `nearest` in prompt-audit is the archive-health metric: flat is healthy, rising means `identify_image.sh` archive writes are failing silently (it fails open, stderr only); it can fall when a failed prompt is committed later.
- Blobs recovered only as unreachable objects have `committed == 0` ("unknown date" in `prompt_view._stamp`) and empty `PromptVersion.paths` ("path unknown" in an Exact summary).
- A grammar without a `lineage` group is legal: unarchived versions resolve to `Unknown`, never `Nearest` (`test_resolve_returns_unknown_when_the_grammar_has_no_lineage`).
- Search walks every publicable image per GET; `SEARCH_LIMIT`=200 is applied before the already-published check, so a broad needle can show far fewer than 200 hits plus the "more matches exist" banner.
- Search results outside the initial sample must go through `_register_thumbs` or their thumbnails 404.
- Write `[prompts].filename` as a TOML literal string (`'...'`); a basic `"..."` string needs every backslash doubled.
- prompt-audit's `unknown` counts only parsed files whose version and lineage both miss; non-prompt-bearing names are in `files` but neither parsed nor unknown.
- The data repo's `huggingface_prompts/.gitignore` contains `prompt`: an image generated from that scratch file is permanently unarchivable (always Nearest/Unknown). Invisible from this repo.

## Open items

- Unbuilt, not rejected: prompts on the calendar tab (`calendar_view.timeline` rows carry `basename`), storing the resolved prompt in `publications.json`, feeding it to `llm_meta`. Rejected on principle: on-disk index, client-side prompt-UI JS.
- Write side: `identify_image.sh` carries the ADR 0001/0004 guards and has `test_identify_image.sh`, but `huggingface_prompts` history holds no `snapshot:` commits yet, so a flat `nearest` is not yet evidence the guards work in production.
- `test_concurrent_queries_never_raise_or_tear` is a smoke/deadlock test; it cannot reliably force the torn-index race red.
- A persistent index keyed on HEAD was considered and never needed; the `HEAD_PROBE_TTL` throttle was enough.
