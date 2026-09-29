# Gallery UI (publish-next)

- **Does:** Human-review gallery on 127.0.0.1: unpublished picks + queued entries, AI metadata, per-card edits, queue persisted to `publications.json`, batch publish to DeviantArt; tab 2 is `docs/features/calendar.md`.
- **Run:** `nix run .#publish-next -- [-n 10] [--data-dir DIR] [--json FILE] [--ai-model ID] [--openrouter-model ID] [--ai-timeout 300] [--port 8765] [-v]`
- **Code:** `src/publicator/webui/page.py` (`PAGE_TEMPLATE`, `render_page`, `card_form_html`, `tier_gallery_fields`, `preset_options`), `src/publicator/webui/server.py` (`GalleryHandler`, `serve`, `do_GET`, `do_POST`, `_build_page`, `SEARCH_LIMIT`), `src/publicator/apps/publish_next.py` (`main`), `src/publicator/store.py` (`write_publications`, `apply_update`, `set_or_pop`), `src/publicator/deviantart.py` (`configure`, `load_pending_entries`, `publish_batch`), `src/publicator/images.py` (`find_candidates`, `ensure_thumb`, `thumb_for_ai`), `src/publicator/scheduling.py` (`schedule_data`, `ts_labels`, `resolve_ts`)
- **Tests:** `tests/test_page.py` (render_page: selects, presets, pending payload, thumbs src, placeholders, search box, banners), `tests/test_server.py` (thumb maps, `/thumbs` allow-list, exact-only search, inert when archive off), `tests/test_store.py` (write/update helpers, naive schedule resolved in config TZ), `tests/test_scheduling.py` (scheduler-core JS slice under node; skips without node)
- **Config:** `publicator.toml`: `publicable` (candidate dirs), `tags` (for `deviantart.configure`, unread by the UI); `[deviantart]` `tiers`/`galleries` (card inputs, omitted when both empty, enforced by `validate_publications` on every write); `[schedule]` `timezone` + `profiles` (presets and `SCHEDULES` payload, owned by scheduling); `[prompts]` presence toggles prompt search and per-card prompt blocks.
- **Data:** `publications.json` (read at startup; written by `/stage`, `/update`, `/publish` and `mark_state`, always via `atomic_write_json`), `publicator.toml` (read once by `config.load_config`), `<publicable dirs>/` e.g. `picked/` (walked for candidates; `/delete` `os.remove`s the named path), `.thumbs/<sha512[:32]><ext>` (content-addressed cache, built on first `/thumbs` request, shared with `thumb_for_ai` and the calendar), `.deviantart-login/`, `.deviantart-session/` (used by `publish_batch`)
- **Decisions:** no ADR: `/stage` is its own endpoint, not a `/publish` mode, so only the terminal path sets `publish_done`; prompt search is a plain GET form, no client JS; one thumb cache in the data dir replaced the per-run temp dir; `/publish` blocks for the whole batch (streaming = YAGNI); no UI removes a persisted entry, so staged cards lose Delete.
- **Verify:** `nix develop -c pytest tests/test_page.py -q`
- **Verify:** `nix develop -c pytest tests/test_server.py -q`
- **Verify:** `nix develop -c pytest tests/test_store.py -q`
- **Verify:** `nix develop -c pytest tests/test_scheduling.py -q` (scheduler-core JS slice; skips without node)
- **Verify:** `nix develop -c pytest tests/test_config.py -q` (tier/gallery allow-list enforcement)
- **Verify:** `nix develop -c pytest tests/test_docs.py -q` (every backticked repo path in docs must exist)

## How it works

1. `publish_next.main` parses flags (`--json` defaults to `<data-dir>/publications.json`), calls `load_config`, `deviantart.configure`, `images.find_candidates` (sha512-filtered, shuffled, `-n` cap), `load_pending_entries`; exits 0 if both empty.
2. `server.serve` sets `GalleryHandler` class attributes (candidates, pending, thumb dicts via `thumb_maps`, `existing_ts`, models, timeout, config, `schedule_data`, `prompts.from_config` archive) and starts the server with `publish_done=None`.
3. `serve` prints `Gallery: http://127.0.0.1:<port>`, Popens `firefox --private-window --no-remote` and loops `handle_request()` until `publish_done` is set, then returns it.
4. `GET /` -> `_build_page` (search query: `search()` + `_register_thumbs`) -> `page.render_page`: pending cards (`pending_N`, View + Edit), then candidates (`card_N`, View + Add + Delete), one `card_form_html` each; `__CARDS__` is substituted last.
5. Client boot pushes each PENDING entry into the in-page `queue` with its uuid; `refreshCount()` writes `N queued` / `N on pad`, enables Publish when the queue is non-empty and Add-to-pad when some entry lacks a uuid.
6. Add/Edit -> `openForm(cardId)` pre-fills from the queue entry (`presetForTs`, else `(custom)`), otherwise selects `SCHEDULES[0]` and fills the picker with `tsToLocalInput(nextSlotForProfile(SCHEDULES[0]))`.
7. "Generate with AI" -> `POST /ai {path, model}`: 400 unless model is `ai_model` or `openrouter_model`; `thumb_for_ai(path)` then `llm_meta.generate_metadata(..., timeout=ai_timeout)` fills `.f-title`/`.f-description`.
8. Save -> `readForm` (title, description, schedule required; price >= 0) -> `{title, description, schedule, scheduleTs, price?, tier?, galleries?}`; `saveCard` adds a `queued` entry or updates the existing one, POSTing `/update` if it has a uuid.
9. `POST /update` -> 400 without uuid -> `store.apply_update` (title -> `urlElsePublicationName`, schedule -> `resolve_ts`, price/tier/galleries via `set_or_pop`; 404 on unknown uuid) -> `validate_publications` (400) -> `atomic_write_json`.
10. "Add to publish pad" -> `stageQueue` POSTs uuid-less entries to `/stage` -> `store.write_publications` appends each (`state=unpublished` DA apparition, `resolve_ts`, `set_or_pop`), validates, writes, returns uuids; those cards lose Delete.
11. Delete -> `confirm('Delete <path> from disk?')` -> splice from queue -> `POST /delete {path}` -> `os.remove(path)` (200 `{ok:false, error}` on OSError) -> pop from `thumb_map`/`thumb_src` -> card removed client-side.
12. `publishQueue` POSTs the queue to `/publish` -> `do_POST` writes uuid-less entries first (400 on failure), then `deviantart.publish_batch` -> `publish_done={published, failed, error?}`; `main` prints `Done. published=.. failed=..`, exit 1 on `error`.

## Invariants

- **Slot occupancy never touches the browser clock.** `--private-window` spoofs `Date` to UTC; `nextSlotForProfile` compares `EXISTING_TS` + `scheduleTs` to slot instants by exact equality; `page.py` `// >>> scheduler core`, `tests/test_scheduling.py::test_new_slot_packs_after_taken_slots_regardless_of_browser_tz`.
- Relies on saved schedules being resolved server-side from the naive `schedule` string in the schedule TZ (`scheduleTs` is only a fallback; `localInputToTs` reads browser TZ), owned by `docs/features/scheduling.md`.
- **`__CARDS__` is the last placeholder `render_page` replaces.** Card HTML embeds arbitrary prompt text and `html.escape` leaves `__NAME__` intact; `tests/test_page.py::test_prompt_text_containing_a_placeholder_survives_as_literal_text`.
- **`/thumbs/<name>` serves only `thumb_src` names, `/original?path=` only `thumb_map` keys; anything else 404s.** The only path-traversal guards on the file-serving routes, `_register_thumbs` grows both dicts; `server.do_GET`, `tests/test_server.py::test_thumbs_route_refuses_a_name_the_page_never_rendered`.
- **`/ai` accepts only `ai_model` and `openrouter_model` (missing -> `ai_model`, else 400 `model not allowed`).** This constrains the *requested* id, not what reaches `llm -m`: on a 429 `run_llm` may substitute another free vision id from llm-openrouter's own cache (ADR 0006), always `openrouter/`-prefixed by our code and passed as an argv list element, never a shell string. `server.do_POST` `/ai`, `llm_meta.run_llm`.
- **Every gallery write to `publications.json` runs `validate_publications` before `atomic_write_json`; a failure is 400, the file untouched.** Only `store.mark_state` skips it; `store.write_publications`, `server.do_POST` `/update`, `tests/test_config.py::test_validate_rejects_tier_absent_from_config`.
- **A queue entry with a uuid is already persisted.** `/stage` and `/publish` write only uuid-less entries and a staged card loses Delete, so no double or orphaned rows; `page.py` `stageQueue` (`queue.filter(e => !e.uuid)`), `publishQueue`, `server.py` `/publish` split.
- **`/stage` never sets `publish_done`; only `/publish` does, ending the serve loop.** "Add to publish pad" must keep the session alive; `server.do_POST`, `serve()` `while server.publish_done is None`.
- **Tier/galleries inputs render only when config declares one; a cleared price/tier/galleries is popped, never written null/empty.** The schema forbids `tier`/`galleries` on non-deviantart apparitions; `page.tier_gallery_fields`, `store.set_or_pop`, `tests/test_page.py::test_page_renders_config_tiers_galleries`.
- **Archive off (`archive is None`): no search form, `search()`/`_prompt_html` return empty.** A rendered form could empty the gallery unexplained; `server._build_page`, `tests/test_page.py::test_render_page_with_search_disabled_emits_no_search_form`, `tests/test_server.py::test_search_is_inert_when_the_feature_is_off`.

## Gotchas

- `GalleryHandler` state lives in class attributes (mutated by `/delete` and `_register_thumbs`); tests must monkeypatch them or leak (`tests/test_server.py::test_search_matches_exact_prompt_text_only`); a concurrent tab can race the thumb dicts.
- `/delete` removes whatever `path` the body names, unchecked against `thumb_map`/`candidate_paths`; the only gate is the browser `confirm()`. The thumbnail is deliberately left in `.thumbs/`.
- The page runs in `firefox --private-window`, where `Date` reports UTC: new client date logic must go through `LABELS`/`LABEL_TO_TS`; a custom picker time is read in browser TZ client-side but persisted from the naive string server-side.
- Without `firefox` on PATH `serve` prints `(open the URL yourself; firefox not found)` and keeps serving; exit is 0 when the server stops without a `/publish`.
- `nextSlotForProfile` silently falls back to the last slot when all `SLOT_HORIZON_WEEKS` (52) are full; bump `scheduling.SLOT_HORIZON_WEEKS` if queues reach a year out.
- After `/publish` the serve loop ends and the page body becomes the result summary; rerun the app for the gallery. `do_POST` calls `publish_batch` without `headless`, so `/publish` always drives a headed Firefox.
- `/publish` appends new entries before driving DA; when `submit_entry` raises, `publish_batch` breaks without `mark_state`, so those rows stay `state=unpublished` and reappear as pending cards next run.
- `/ai` runs `generate_metadata` synchronously with `--ai-timeout` (300s default); other requests still get served. `--openrouter-model` needs `$OPENROUTER_KEY` (llm-openrouter plugin); the Claude option shells `llm -m <ai_model>` to the `claude` CLI.
- `load_pending_entries` prints `skip pending <uuid>: art file not found` to stderr and drops queued entries whose art `_pending_art` cannot `rglob(*<stem>*)`; the rows stay in `publications.json` but are invisible in the gallery.
- Title is capped at `maxlength=50` in `card_form_html` only; neither `store` nor the schema enforce it server-side.
- `saveCard` on a re-edit rewrites the card title via `.name.lastChild.textContent`, relying on the title being the last text node after the `queued` badge; changing the card markup breaks that silently.
- `.webp` candidates are accepted into the queue (`images.IMAGE_EXTENSIONS`); conversion happens in `deviantart._step_upload` -> `_resolve_art`, not in the UI.

## Open items

- No automated test covers `do_POST` (`/ai`, `/stage`, `/update`, `/publish`, `/delete`); a regression there is only caught live.
- `-n` (default 10) samples candidates randomly each run (`find_candidates` shuffles); only a prompt search (`?prompt=`/`?lineage=`) shows other picks: it walks the publicable dirs in full but caps each list at `SEARCH_LIMIT=200`.
