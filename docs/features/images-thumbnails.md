# Images and thumbnails

- **Does:** Finds unpublished candidates (sha512-dedup against `publications.json`), keeps one content-addressed `.thumbs/` cache shared by gallery, calendar and `/ai`, and resolves art to a non-webp upload at publish time.
- **Run:** `nix run .#publish-next -- [--data-dir D] [-n 10] [--json F] [--port 8765] [--ai-model M] [--openrouter-model M] [--ai-timeout 300] [-v]` (persistent `.thumbs/` cache)
- **Run:** `nix run .#generate-meta -- [--model M | --openrouter] [--timeout 300] [--json] [-v] IMAGE...` (same `thumb_for_ai`, throwaway tempdir cache)
- **Run:** `nix run .#da-publish -- [--data-dir D] [--json F] [--uuid ID | --all] [--headless] [-v]` (webp to png via `convert`)
- **Run:** `nix run .#prompt-audit -- [--data-dir D]` (library use of `collect_images` only)
- **Code:** `src/publicator/images.py` (`IMAGE_EXTENSIONS`, `find_candidates`, `thumb_name`, `ensure_thumb`, `thumb_for_ai`, `index_by_basename`), `src/publicator/webui/server.py` (`thumb_maps`, `GalleryHandler.do_GET`, `SEARCH_LIMIT`), `src/publicator/webui/calendar_view.py` (`timeline`, `event_html`), `src/publicator/webui/page.py` (`render_page`), `src/publicator/deviantart.py` (`_pending_art`, `_resolve_art`, `_step_upload`), `src/publicator/entries.py` (`find_art_path`), `src/publicator/apps/generate_meta.py` (`main`), `src/publicator/apps/publish_next.py` (`main`)
- **Tests:** `tests/test_images.py` (thumb naming, cache reuse, candidate dedup, dot-dir skip), `tests/test_server.py` (`/thumbs` allow-list and serving, `thumb_maps` row pairing), `tests/test_calendar.py` (legacy sha key, off-disk art renders as title)
- **Config:** `publicator.toml` top-level `publicable` (list[str], default `[]`): dirs joined to the data dir that `find_candidates` walks recursively for `IMAGE_EXTENSIONS` files, and that a filtered page (`?prompt=`/`?lineage=`) re-walks in full via `GalleryHandler.search`.
- **Data:** `.thumbs/<sha512[:32]><lowercased ext>` (`THUMB_CACHE`, `GalleryHandler.cache_dir`; written lazily on first `/thumbs`/`/ai` request, never pruned; gitignore it); `publications.json` (read only: sha keys and `files[0].basename`, fields written by publications-store); `<publicable>/**` (walked for candidates, re-walked in full by filtered pages); data dir (`index_by_basename` walks it once per `serve()` skipping dot-dirs; `rglob('*<stem>*')` includes them); `tempfile.gettempdir()/<stem>.png` (webp converted by `_resolve_art`); `tempfile.TemporaryDirectory` (throwaway thumb cache for `generate-meta`).
- **Decisions:** no ADR: one persistent content-addressed `.thumbs/` cache instead of per-run `/tmp` thumbnails (`ensure_thumb` docstring, `do_GET` comment, commit `ef88b1e`); `/delete` keeps the cached thumbnail (~20KB, reused if the art comes back); `SEARCH_LIMIT = 200` caps sha512 hashing on filtered pages, raise it if a lineage outgrows it.
- **Verify:** `nix develop -c pytest tests/test_images.py tests/test_server.py tests/test_calendar.py -q`

## How it works

1. `apps/publish_next.main` calls `images.find_candidates(dirs, json, n)`; `collect_images` os.walks each dir keeping files whose lowercased ext is in `IMAGE_EXTENSIONS` (a missing dir yields nothing), then the list is shuffled.
2. `find_candidates` sha512-hashes the shuffled list and keeps the first n whose hash is not in `load_publicated_hashes` (both sha keys, lowercased, under `files[]`/`previewFilesIfNotFree[]`); an `OSError` goes to stderr and skips the file.
3. `deviantart.load_pending_entries` → `_pending_art(files[0].basename)`: `DATA_DIR.rglob('*<stem>*')`, `is_file()` only, prefers `suffix.lower() != '.webp'`, else first match; `None`/`SystemExit`/`KeyError`/`IndexError` skips the entry on stderr.
4. `webui.server.serve` calls `calendar_view.timeline(args.json)`: one row per DA apparition carrying `files[0].basename` and the stored sha (`''` when absent), then `thumb_maps(data_dir, candidates + pending, rows)`.
5. `thumb_maps`: `thumb_name` per gallery path into `thumb_map {path: name}`/`thumb_src {name: path}`; `index_by_basename` stamps `row['thumb'] = thumb_name(src, row['sha'])` (stored sha, else hashed) and `row['path'] = src`, both `''` off disk; rows enter `thumb_src` only.
6. `page.render_page`: `<img src="/thumbs/{thumb_map.get(path, '')}">` + `/original?path=` View link per card; `calendar_view.event_html`: `<img src="/thumbs/{row['thumb']}">` with an `onerror` alt fallback, bare title when thumb is `''`.
7. `GET /thumbs/<name>` (`GalleryHandler.do_GET`): `thumb_src.get(name)` verbatim or 404, then `ensure_thumb`: on a miss `convert src -resize 300x300> thumb`, `copy2` of the original on `CalledProcessError`/`FileNotFoundError`; `OSError` → 404.
8. `GET /original?path=P` (`do_GET`) serves full-res only when P is a `thumb_map` key (candidates, pending, registered hits, minus `/delete` pops); `POST /delete`: `os.remove(path)`, pops `thumb_map`/`thumb_src`, thumbnail stays in `.thumbs/`.
9. `POST /ai` (`do_POST`): `thumb_for_ai(path, cache_dir, thumb_map.get(path))` = `ensure_thumb`, original path on `OSError`, then `llm_meta.generate_metadata`; `apps/generate_meta.main` makes the same call with a `TemporaryDirectory` cache, no name.
10. `GalleryHandler.search` (`?prompt=`/`?lineage=`) re-walks `publicable_dirs` with `collect_images`, filters by the prompt archive, caps each list at `SEARCH_LIMIT`, then sha512-hashes the survivors against `load_publicated_hashes`.
11. `GalleryHandler._build_page` calls `_register_thumbs(exact + maybe)`, which hashes each path not yet in `thumb_map` and extends the class-level `thumb_map`/`thumb_src` before `render_page`.
12. `deviantart._step_upload` (publish-next `publish_batch`, da-publish) calls `_resolve_art`: non-webp → as is; webp → `entries.find_art_path` (rglob `*<stem>*`, first `p.suffix != '.webp'`), else `convert` to `<tempdir>/<stem>.png`, `check=True`.

## Invariants

- **Cache name = sha512(content)[:32] + the original's lowercased extension.** One thumbnail per image across runs and renames, and the extension lets `guess_mime` set Content-type; `images.thumb_name`, `test_thumb_name_is_content_addressed`, `test_thumb_name_accepts_a_known_sha_without_reading_the_file`.
- Relies on `/thumbs/<name>` serving only `thumb_src` names (verbatim lookup, no normalisation, else 404), owned by `docs/features/gallery-ui.md`.
- **Thumbnails are made on first request, persist in `.thumbs/`, and a cached file is never regenerated.** A cold calendar must not stall startup; one cache across tabs and runs; `images.ensure_thumb` (`exists` short-circuit), `test_ensure_thumb_reuses_the_cached_file`, `test_thumbs_route_serves_a_rendered_name`.
- Relies on calendar rows being named from the stored sha (hashed from disk only when absent, `''` and title-only when off disk), owned by `docs/features/gallery-ui.md` (`thumb_maps`) and `docs/features/calendar.md` (`event_html`).
- **Every production vision-model call routes through `thumb_for_ai`.** Same visual info for a fraction of tokens; full-res reaches the model only via its fallbacks (unwritable cache, failed/missing `convert`); `do_POST '/ai'` and `apps/generate_meta.main` are the only `src/` callers of `llm_meta.generate_metadata`.
- **`index_by_basename` prunes every dot-directory; the first hit per basename wins (`dict.setdefault`).** Else `.thumbs/` files and browser profiles would shadow real art; `images.index_by_basename`, `test_index_by_basename_finds_art_under_the_data_dir` (dot-dir skip only; first-match-wins has no test).
- **Dedup is by sha512, case-insensitive, via `sha512sum` or legacy `fileSha512sum` under `files[]`/`previewFilesIfNotFree[]`; the calendar reads the same keys.** Back-catalogue is mostly legacy-keyed, some upper-case; `load_publicated_hashes`, `calendar_view.timeline`, `test_timeline_reads_the_legacy_sha_key`.
- Relies on uploads being non-webp (on-disk non-webp match under the data dir, else png conversion at publish), owned by `docs/features/deviantart-publish.md` (`_resolve_art`, `_pending_art`) and `docs/features/publications-store.md` (`find_art_path`).
- Relies on a filtered page extending the `/thumbs` allow-list with its search hits before rendering (`_register_thumbs`), owned by `docs/features/gallery-ui.md`.

## Gotchas

- `ensure_thumb` hides a failing/missing `convert` by `copy2`-ing the full-size original under the thumb name and `exists()` keeps it forever: full-res "thumbnails" served and sent to the AI. Fix: ImageMagick on PATH, delete oversized `.thumbs/` files.
- The suite never proves `convert` resizes: `test_ensure_thumb_*` and `test_thumbs_route_serves_a_rendered_name` write fake bytes (`b'\x89PNG-art'`) on which `convert` exits 1, so they pass via the copy2 fallback (thumb byte-identical to source).
- `generate-meta`'s flake app lists only `claude` in `runtimeInputs` (`flake.nix:103`; `app` adds `python`, no `pkgs.imagemagick` unlike `publish-next`/`da-publish`), so `convert` comes from the host PATH or `thumb_for_ai` silently sends full-res.
- `IMAGE_EXTENSIONS` includes `.mp4`, so a video becomes a candidate and its card is `<img src="/thumbs/<sha>.mp4">`, which browsers cannot render. Whether `convert` resizes it or the copy fallback stores the whole video in `.thumbs/` is untested.
- `/ai` and `/delete` take `path` from the POST body with no `thumb_map` check; only `/thumbs` and `/original` are allow-listed, so any local path posted to `/ai` is thumbnailed into `.thumbs/` and sent to the model. Server binds `127.0.0.1` only.
- `GalleryHandler` state (`thumb_map`, `thumb_src`, `cache_dir`, ...) is class attributes mutated at runtime (`_register_thumbs`, `/delete`); tests must monkeypatch them or leak, and `tests/test_server.py::_serve_once` uses `setattr` and does leak.
- `/original?path=` is allow-listed on `thumb_map` keys only, so calendar rows (in `thumb_src` only) get no full-res link; `event_html` emits none.
- Startup hashes every candidate twice: `find_candidates` sha512s shuffled images until n survivors, then `thumb_maps` re-hashes every candidate and pending path for its cache name. Large `picked/` dirs make "Finding unpublished images..." slow.
- A filtered page hashes at request time: up to `SEARCH_LIMIT` (200) exact plus 200 maybe survivors per load, and `_register_thumbs` hashes each new hit once more the first time it is rendered (`thumb_map` memoises later loads).
- An unwritable `.thumbs/` makes every uncached `/thumbs` request 404 while `/ai` falls back to the original for the same misses (already-cached names still serve: `ensure_thumb` checks `exists()` first): pages look partly broken but AI does not.
- Basename resolvers differ: `index_by_basename` (os.walk, exact name, no dot-dirs, first wins), `_pending_art` (rglob `*stem*` incl. dot-dirs, `is_file`, `suffix.lower()`, `None`), `find_art_path` (rglob incl. dot-dirs, no `is_file`, `SystemExit`).
- `entries.find_art_path` tests `p.suffix != '.webp'` case-sensitively, so a `.WEBP` file passes as uploadable and `_resolve_art` (which lowercases) would then upload it.

## Open items

- Is `.mp4` in `IMAGE_EXTENSIONS` still wanted now that thumbnails go through `convert`? No test covers a video candidate.
- Should the copy2 fallback in `ensure_thumb` be cached at all, or should a failed convert be retried on the next request?
- The three basename resolvers differ in dot-dir handling, case handling and failure mode; unclear if intentional.
- The `.thumbs/` cache is never pruned; no decision recorded on whether that is acceptable long-term.
- Nothing in the suite exercises a real `convert` resize, `_resolve_art`, `_pending_art`, `find_art_path` or `_register_thumbs`.
