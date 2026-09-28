# Calendar tab

- **Does:** Read-only second tab of the publish-next gallery: every DeviantArt apparition in `publications.json`, published and queued, as month grids of thumbnail events, day-bucketed server-side in the schedule timezone.
- **Run:** no dedicated app: `nix run .#publish-next -- [--data-dir DIR] [--json PATH] [--port 8765] [-n 10]` from the data dir (or pass `--data-dir`), then click `Calendar` (`#tab-calendar`) in the header.
- **Code:** `src/publicator/webui/calendar_view.py` (`SLUG`, `timeline`, `months_between`, `event_html`, `month_html`, `render_calendar`), `src/publicator/webui/server.py` (`serve`, `thumb_maps`, `GalleryHandler.do_GET`), `src/publicator/webui/page.py` (`PAGE_TEMPLATE`, `render_page`), `src/publicator/images.py` (`thumb_name`, `index_by_basename`, `ensure_thumb`), `src/publicator/scheduling.py` (`zone`, `DEFAULT_TZ`), `src/publicator/apps/publish_next.py` (`main`)
- **Tests:** `tests/test_calendar.py` (timeline rows, titles, sha keys, sorting; rendered day placement, links, classes, months), `tests/test_page.py` (calendar tab present, timeline rendered, queued entry linked to its card), `tests/test_server.py` (thumb_maps pairing and sha-less hashing, `/thumbs` route allow-list)
- **Config:** `publicator.toml` `[schedule]` key `timezone` (string, default `"Europe/Paris"`), read via `scheduling.zone(config.get("schedule", {}))` in `page.render_page`: the tz every row is day-bucketed in and `today` is computed in. `calendar_view.py` reads no config itself.
- **Data:** `publications.json` (or `--json PATH`): `deviantart` apparitions only, read once at `serve()` startup, never written. Data dir non-hidden subdirs: walked once by `images.index_by_basename` to map basenames to art paths. `.thumbs/<sha512[:32]><ext>`: shared thumbnail cache, generated on first `GET /thumbs/<name>`; add `.thumbs/` to the data dir's `.gitignore`.
- **Decisions:** none
- **Verify:** `nix develop -c pytest tests/test_calendar.py -q`
- **Verify:** `nix develop -c pytest tests/test_page.py -q`
- **Verify:** `nix develop -c pytest tests/test_server.py -q`

## How it works

1. `apps.publish_next.main` parses flags, runs `load_config`, defaults `--json` to `<data-dir>/publications.json`, collects `find_candidates` + `load_pending_entries`; both empty → exit 0 ("Nothing to publish") before `serve()`, else `serve(...)`.
2. `serve()` sets `GalleryHandler.timeline = calendar_view.timeline(args.json)`: one row per `deviantart` apparition with `ts` (apparition instant, else entry `submissionTimestamp`, else 0), `state`, `uuid`, `files[0]` basename and sha.
3. `entry_title` takes `title` from the `urlElsePublicationName` slug (minus trailing `-<id>` and `?query`), the first `description` line when the slug has no letters, or a non-http name verbatim; `url` is set only for names starting with `http`.
4. `serve()` calls `thumb_maps(data_dir, candidate + pending paths, timeline)`: `index_by_basename` walks the data dir once; a row found on disk gets `r["thumb"] = images.thumb_name(src, r["sha"])` (hashed only when sha is empty), else `""`.
5. `thumb_maps` also adds found rows to `GalleryHandler.thumb_src` (cache name to art path), the `/thumbs` allow-list; `GalleryHandler.cache_dir = <data-dir>/.thumbs`.
6. `GET /` or `/index.html` (also `?prompt=`/`?lineage=`) goes through `GalleryHandler._build_page` to `page.render_page(timeline=self.timeline, config=self.config, pending=self.pending, ...)`.
7. `render_page` computes `tz = zone(config.get("schedule", {}))`, ids each pending card `pending_{idx}`, builds `anchors = {uuid: cardId}`, replaces `__CALENDAR__` with `render_calendar(timeline, tz=tz, anchors=anchors)` first; `__CARDS__` last.
8. `render_calendar(rows, *, tz, now=None, anchors=None)`: `today = fromtimestamp(now, tz).date()`, rows bucketed by `fromtimestamp(ts, tz).date()`, `days = list(by_day) + [today]`, `months_between(min(days), max(days))` feeds `month_html` per month.
9. `month_html`: `calendar.Calendar().monthdatescalendar` grid; spill cells `<td class="day other">`, own days `<td class="day[ today]" data-date=ISO>` + `event_html`; wrapper `<section class="month" id="m-YYYY-MM">`, current one `month now`.
10. `event_html`: `ev upcoming` if `ts >= now` else `ev past`; a lazy `<img src="/thumbs/<thumb>">` with escaped title as alt/title, or the title text when thumb is falsy; href = row url (new tab), else `#<anchors[uuid]>`, else unlinked `<span>`.
11. `showTab(name)` (page JS) toggles `hidden` on the tab panes and `on` on the buttons; `calendar` scrolls `.month.now` into view; a `#calendar-tab` click on `a[href^="#"]` runs `showTab('gallery')` then `scrollIntoView({block:'center'})`.
12. Each `<img>` hits `GET /thumbs/<name>`: `do_GET` looks the name up in `thumb_src` (unknown gives 404), else `images.ensure_thumb(src, cache_dir, name)` generates on a miss and `_send_file` streams it with `guess_mime`; `OSError` gives 404.

## Invariants

- **Day placement is server-side in the schedule tz; the calendar JS never reads `Date`.** `firefox --private-window` spoofs `Date` to UTC, so a 00:30 Paris post would land the day before; `render_calendar` (`fromtimestamp(ts, tz).date()`), `tests/test_calendar.py::test_event_lands_on_its_local_day_not_the_utc_one`.
- **The calendar is read-only.** The gallery tab owns editing (`/update`, `/stage`) and duplicating it needs forbidden browser-side schedule math; `calendar_view.py` docstring, `server.do_POST` handles only `/delete`, `/ai`, `/publish`, `/stage`, `/update`.
- **Thumb cache names are stamped server-side (`row["thumb"]`) by `server.thumb_maps`; `render_calendar`/`event_html` never open a file.** Keeps the view pure and reuses the stored sha512 instead of re-hashing the back-catalogue; `tests/test_server.py::test_thumb_maps_pairs_gallery_art_with_calendar_rows`.
- Relies on `/thumbs/<name>` serving only `GalleryHandler.thumb_src` names (the path-traversal allow-list; rows whose art is gone get thumb `""` and are never added), owned by `docs/features/gallery-ui.md`.
- **Only `platformName == "deviantart"` apparitions become rows, sorted ascending by `ts`.** Other platforms would mislabel occupancy; `calendar_view.timeline`, `tests/test_calendar.py::test_timeline_ignores_other_platforms_and_sorts_by_date`.
- **Both legacy `fileSha512sum` and current `sha512sum` are read.** Most of the back-catalogue stores the legacy key; `calendar_view.timeline`, `tests/test_calendar.py::test_timeline_reads_the_legacy_sha_key`.
- **The month range always includes the current month, classed `month now`, even for an empty timeline.** `showTab('calendar')` scrolls to `.month.now`; `tests/test_calendar.py::test_empty_timeline_still_renders_the_current_month`, `::test_the_current_month_is_marked_so_the_tab_can_scroll_to_it`.
- **A row links to DA (new tab) only when `urlElsePublicationName` starts with `http`.** Rows published by this tool keep the title in that field, so they stay unlinked; `event_html`, `tests/test_calendar.py`.
- **Queued rows link to `#pending_N` via `anchors`; a queued row with no card is shown unlinked, never dropped.** Only `render_page` mints card ids and dropping rows would hide scheduled work; `event_html`, `tests/test_calendar.py::test_a_queued_event_without_a_card_is_still_shown_unlinked`.
- Relies on `__CALENDAR__` being replaced before `__CARDS__` in `render_page` (a literal `__CALENDAR__` in card prompt text would otherwise receive the calendar HTML), owned by `docs/features/gallery-ui.md`.

## Gotchas

- The calendar cannot be opened alone: `publish_next.main` exits 0 with "Nothing to publish" before `serve()` when `picked/` has no candidates and nothing is queued, so a data dir with only published history never shows it.
- `timeline` is a startup snapshot: entries staged (`/stage`) or rescheduled (`/update`) during a session do not appear or move until publish-next restarts.
- An apparition with no timestamp and no entry `submissionTimestamp` gets `ts=0` (1970-01-01); `months_between` then renders every month from January 1970 to today (~680 grids). Not guarded.
- Thumbnails resolve by basename over one `os.walk` (`images.index_by_basename`): duplicate basenames collide (first walked path wins, `setdefault`), hidden dirs are skipped, and a sha-less row found on disk is sha512-hashed at startup.
- Art missing from disk shows the title text (thumb `""`, decided at startup); a thumb that 404s at request time degrades to alt text via `onerror`. Both look like a missing thumbnail.
- A `#pending_N` href only works via the `#calendar-tab` click listener; plain anchor navigation targets a hidden element and does nothing. No test drives this JS; `tests/test_page.py` only asserts `href="#pending_0"` is rendered.
- Published titles come from the DA URL slug (`SLUG = r"/art/(.+?)(?:-\d+)?(?:\?|$)"`), falling back to the description's first line when the slug has no letters; titles are display-only, anchors key on `uuid`.
- `ensure_thumb` shells out to `convert -resize 300x300>` and silently `shutil.copy2`'s the full-size original on failure; the flake app lists imagemagick, but running the module outside nix can fill `.thumbs/` with originals.
- CSS lives in `page.PAGE_TEMPLATE` (`.month`, `.month.now`, `table.cal`, `.day`, `.day.other`, `.day.today`, `.day .num`, `.ev`, `.ev.past`, `.ev.upcoming`), not `calendar_view.py`; a class rename touches both files and `tests/test_calendar.py`.

## Open items

- Re-read `timeline` on `/stage` and `/update` so the calendar reflects the session's own changes without a restart?
- Skip or clamp rows with `ts=0` instead of rendering month grids back to 1970?
- Keep serving publish-next (calendar only) when there is nothing to publish, instead of exiting before `serve()`?
- The tab-reveal-before-anchor-jump JS has no test; worth one node-driven check like `tests/test_scheduling.py`'s if that handler is touched.
- No test plants a literal `__CALENDAR__` in prompt text; `tests/test_page.py::test_prompt_text_containing_a_placeholder_survives_as_literal_text` guards the ordering only for `__PROMPTSEARCH__`.
