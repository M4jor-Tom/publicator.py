# Prompt mapping (image -> generation prompt)

- **Does:** Resolves an art filename to the prompt that generated it by content-digesting every blob of the data dir's prompt git repo (`Exact`/`Nearest`/`Unknown`), shown per gallery card and calendar thumbnail, searchable, audited.
- **Run:** `nix run .#prompt-audit -- [--data-dir DIR]` (prints parsed/exact/nearest/unknown)
- **Run:** `nix run .#ui -- [--data-dir DIR]` (cards show the prompt block; Search uses a runtime subset of configured `publicable` paths; `GET /?prompt=<substring>` searches Exact text, `GET /?lineage=<repo path>` groups images by prompt file)
- **Code:** `src/publicator/prompts.py` (`HEAD_PROBE_TTL`, `PromptArchive`, `from_config`, `rank_paths`, `Nearest`, `resolve_path`), `src/publicator/webui/prompt_view.py` (`render_prompt`, `_stamp`, `_lineage_link`, `_body`), `src/publicator/apps/prompt_audit.py` (`audit`, `format_report`, `main`), `src/publicator/config.py` (`_load_prompts`, `load_config`), `src/publicator/webui/server.py` (`SEARCH_LIMIT`, `GalleryHandler.search`, `GalleryHandler._prompt_html`), `src/publicator/webui/page.py` (`render_page`, `search_enabled`, `__CARDS__`), `flake.nix` (`apps.prompt-audit`, `apps.ui`)
- **Tests:** `tests/test_prompts.py` (index, three-state resolve, ranking, HEAD refresh, git failures), `tests/test_prompt_view.py` (Exact/Nearest rendering, escaping, every candidate listed), `tests/test_prompt_audit.py` (counts, feature off), `tests/test_config.py` (`[prompts]` grammar validation and rejections), `tests/test_server.py` (search semantics, feature-off inertness, skipped vs maybe), `tests/test_page.py` (block injection, search box, banners, placeholder survival)
- **Config:** `publicator.toml` `[prompts]`: `repo` (joined to the data dir; dir absent = off silently), `filename` (Python `re` as TOML literal `'...'`; needs `(?P<version>)`, optional `(?P<lineage>)`), `version_hash` (in `hashlib.algorithms_available`; real data `sha1`). Section absent = `config['prompts']` `None`, off. Also `publicable` (dirs walked), `[schedule].timezone` (dates Nearest versions).
- **Data:** `<data-dir>/<prompts.repo>` (the `huggingface_prompts` submodule; read-only via `git cat-file`, `git log --all`, `git rev-parse HEAD`); `<publicable dir>/**` (image filenames parsed with the grammar); `publications.json` (search drops already-published hits via `load_publicated_hashes`; pending entries get prompt blocks); `.thumbs/` (allow-list extended for search results).
- **Decisions:** Git history is the only prompt archive; archive before minting an identity but never block image preservation; distinguish exact content from uncertain lineage in the result type. Filename grammar ownership belongs to [configuration](config.md#prompt-grammar-ownership).
- **Verify:** `nix develop -c pytest tests/test_prompts.py tests/test_prompt_view.py tests/test_prompt_audit.py tests/test_config.py tests/test_server.py tests/test_page.py -q`
- **Verify:** `rg -n 'sha1|_\[0-9a-f\]\{40\}' src/publicator/` (must print nothing; grammar and algorithm belong to configuration)
- **Verify:** `nix run .#prompt-audit -- --data-dir <data-dir>` (`parsed` must be non-zero; compare with the baseline and update it after each generation batch: 2026-09-28 parsed 1255 of 4066, exact 455 images / 62 versions, nearest 793 / 355, unknown 7. `nearest` rising means archive writes are failing)

## How it works

1. `config.load_config` -> `_load_prompts` validates `[prompts]` (repo present, filename compiles with a `version` group, `version_hash` in `hashlib.algorithms_available`) and returns `{repo, pattern, version_hash}` or `None`.
2. `prompts.from_config` returns `None` when the section is missing or `<data_dir>/<repo>` is not a dir, else a `PromptArchive`; `webui.server.serve` stores it on `GalleryHandler.archive`, `prompt_audit.audit` builds its own.
3. `PromptArchive.resolve_path(path)`: `parse_identity(basename, pattern)` -> `ImageIdentity(version, lineage)` or `None` (-> `Unknown`, grammar mismatch), then `resolve(identity, near=os.path.dirname(path))`.
4. Every query (`versions`/`paths_for`/`versions_at`) takes `self._lock` and calls `_refresh`: return if probed within `HEAD_PROBE_TTL` (2s); else `git rev-parse HEAD`; if HEAD moved or the index is empty, rebuild and set `_head` last.
5. Rebuild: `_blobs()` walks `cat-file --batch-all-objects --batch` by byte length, digesting each blob with `hashlib.new(version_hash)`; `_history()` walks `log --all --raw --no-abbrev --no-renames --format=%ct` for paths and first commit time.
6. `resolve()`: digest in `versions()` -> `Exact`. No lineage -> `Unknown`. Else `paths_for(lineage)` (all historical paths of that basename), `rank_paths` (dir-name similarity, nothing dropped), `versions_at` newest-first -> `Nearest`, else `Unknown`.
7. Gallery `GET /`: `GalleryHandler.do_GET` -> `_build_page` -> `_prompt_html(candidates + maybe + pending + timeline rows, deduped)`: per path, `render_prompt(archive.resolve_path(p), near=dirname(p), tz=schedule tz)` -> `{path: html}`, shared by the cards and `render_calendar(prompts=)`; `Unknown` renders `''` and is dropped.
8. `prompt_view.render_prompt`: `Exact` -> `details.prompt.exact`: `v.paths` as `/?lineage=` links or "path unknown", 7-char digest, escaped `<pre>`; `Nearest` -> `details.prompt.nearest` "prompt NOT ARCHIVED" plus dated `details.pv` per lineage.
9. `GET /?prompt=X` or `/?lineage=P`: `GalleryHandler.search` walks every publicable dir via `collect_images`; `Nearest` -> `skipped` unless P is a candidate path ("maybe"); `Exact` -> kept if X in text (case-insensitive) and P in `version.paths`.
10. `search`, cont.: `truncated` set when a list exceeds `SEARCH_LIMIT` (200) before the cap; survivors sha512-hashed, already-published dropped; `_register_thumbs` extends the `/thumbs` allow-list; `render_page` shows both lists plus banners.
11. prompt-audit: `audit()` iterates `collect_images` over `<data_dir>/<publicable>`, counts files, `archive.parse` -> parsed, `archive.resolve` -> exact/nearest/unknown plus distinct version sets; `format_report` prints `parsed   : N of M files` first.

### Archive writes (data repository)

The write-side contract lives in the separate `huggingface_prompts` repository,
not in publicator. `identify_image.sh` attempts to stage and commit the prompt
**before** computing the content digest embedded in the image identity. Unchanged
content makes no new commit; changed content produces `snapshot: <path>`. Resolve
the prompt path absolutely before using `git -C`, whose pathspecs are relative to
the repository rather than the caller's directory.

Archive guards warn on **stderr**, still print exactly one identity on stdout,
and exit zero. `rename_image.sh` captures stdout as the new filename and moves a
download from the fixed `~/Downloads/image.webp` path. Refusing an identity leaves
the image there to be overwritten by the next generation; printing a warning on
stdout corrupts its filename. The priority is **image > prompt > link**. Do not
add fail-closed exits, `set -e`, interactive prompts, or image-management duties
to the identity minter to compensate for an archive failure.

An index lock or interrupted commit can heal retroactively if the same content
is committed before it is edited again. The gitignored scratch file `prompt`
is permanently unarchivable while ignored. Archive-at-mint is an **attempt**, not
a referential-integrity guarantee; `Nearest`/`Unknown` and the audit must remain.

Git is the archive because it already holds content, dates and historical paths.
A digest sidecar directory or JSONL manifest would duplicate that store and need
its own integrity/concurrency rules. No backfill can recover never-committed
content: the 2026-08-21 audit found only 58 of 421 referenced versions recoverable
from all refs, reflogs and dangling objects; 363 (86%) had never been saved.
There was no secondary prompt in the image metadata.

### Exact content versus lineage

A version digest identifies immutable text; a lineage basename is only a hint
because prompts evolve, move, and share basenames. Looking up basename at HEAD
was silently wrong on 404 of 426 measured pairs (14 correct, 8 missing).
`Exact.version.text` is authoritative; `Nearest.candidates` instead exposes all
plausible lineages and their archived versions under an explicit “NOT ARCHIVED”
warning. `Unknown` explains why neither can be found. Historical paths can be
plural even for an Exact hit: identical text in different files is still exact.

A nullable string cannot represent all three outcomes. A text-plus-confidence
flag is too easy for a renderer to ignore, and a single fuzzy winner hides real
ambiguity. Keeping prompt text off `Nearest` prevents accidental conflation;
ranking orders candidates rather than excluding them. Prompt-text search is
therefore Exact-only and announces excluded candidates. Search must resolve and
filter before capping/hashing, not filter `find_candidates`' random sample.

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
- **Index cached on `git rev-parse HEAD`, rebuilt under a `threading.Lock`, probed at most every `HEAD_PROBE_TTL` (2s).** Each successful archive write is a commit, so HEAD is the invalidation key; `_refresh`; `test_index_refreshes_when_head_moves`, `test_index_does_not_refresh_within_the_head_probe_ttl`.
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

### Verify the write-side contract

Use a disposable clone of the prompt repository, never the real archive. Run its
`test_identify_image.sh`, then check that an initial prompt creates a snapshot,
an unchanged repeat creates none, and an edit creates another. Digesting all
stored blob contents with the configured `version_hash` must find the digest
printed in each successful archive's identity; the git object ID is not that key.
Repeat with a locked index and with a gitignored prompt: both must still exit
zero, print exactly one stdout line, and send the warning only to stderr.

## Open items

- Unbuilt, not rejected: storing the resolved prompt in `publications.json`, feeding it to `llm_meta`. Rejected on principle: on-disk index, client-side prompt-UI JS.
- Write-side deployment must be verified in the data repository: the earlier inspection found `identify_image.sh` guards and `test_identify_image.sh` but no `snapshot:` commits. Do not treat a flat `nearest` count alone as proof that production archive writes work.
- `test_concurrent_queries_never_raise_or_tear` is a smoke/deadlock test; it cannot reliably force the torn-index race red.
- A persistent index keyed on HEAD was considered and never needed; the `HEAD_PROBE_TTL` throttle was enough.
