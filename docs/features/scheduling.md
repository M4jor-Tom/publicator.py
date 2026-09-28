# Scheduling (slots and weekly profiles)

- **Does:** Turns `[schedule]` weekly profiles into absolute slot instants (52 weeks, wall-clock hour in the schedule TZ) that pre-fill each card's picker, then resolves the saved wall-clock string to an epoch server-side.
- **Run:** `nix run .#publish-next -- [--data-dir DIR] [--json FILE] [--port 8765]` (the only app exercising slot generation and the picker; no CLI flag controls scheduling, all cadence comes from `[schedule]`)
- **Code:** `src/publicator/scheduling.py` (`SLOT_HORIZON_WEEKS`, `schedule_profiles`, `profile_slots`, `schedule_data`, `existing_ts`, `resolve_ts`), `src/publicator/store.py` (`write_publications`, `apply_update`, `atomic_write_json`), `src/publicator/config.py` (`load_config`), `src/publicator/webui/page.py` (`render_page`, `PAGE_TEMPLATE`, JS `nextSlotForProfile`), `src/publicator/webui/server.py` (`serve`, `GalleryHandler.existing_ts`, `GalleryHandler.schedules`), `src/publicator/deviantart.py` (`load_pending_entries`)
- **Tests:** `tests/test_scheduling.py` (profile validation, `existing_ts`, `resolve_ts`, DST, node-driven browser-TZ regression; skips without node), `tests/test_store.py` (write/update persist `schedule` resolved in config TZ over `scheduleTs`; neither field raises), `tests/test_config.py` (flat and profiles `[schedule]` pass through verbatim; missing file gives `{}`), `tests/test_page.py::test_page_renders_schedule_presets` (preset `<select>`; `SCHEDULES`/`EXISTING_TS`/`LABELS` placeholders replaced)
- **Config:** `publicator.toml` `[schedule]`: `timezone` (IANA, default `Europe/Paris`); `[[schedule.profiles]]`: `name` (default `default`), `day` (weekday name, case-insensitive, default `tuesday`), `hour` (int, default 20, wall clock in `timezone`), `per_slot` (int >= 1, default 2), `frequency` (`weekly` only). Flat keys without `profiles` = one `default` profile; absent table = Tuesday 20:00, per_slot 2.
- **Data:** `publicator.toml` (`[schedule]` passed through by `load_config` as a dict, validated later by `schedule_profiles`); `publications.json` (`apparitions[].apparitionTimestampIfDifferentThanSubmission`, epoch seconds; occupancy via `existing_ts`, queue via `load_pending_entries`, written via `atomic_write_json`)
- **Decisions:** no ADR: presets are never derived from price/tier (a new card starts on `SCHEDULES[0]`, the human picks), and an off-cadence custom timestamp counts toward no profile's occupancy by design (do not snap it to a slot)
- **Verify:** `nix develop -c pytest tests/test_scheduling.py -q` (1 test skips if `node` is not on PATH)
- **Verify:** `nix develop -c pytest tests/test_store.py -q`
- **Verify:** `nix develop -c pytest tests/test_config.py -q`
- **Verify:** `nix develop -c pytest tests/test_page.py -q`

## How it works

1. `config.load_config(data_dir)` returns the raw `[schedule]` table as `config['schedule']` (`dict(cfg.get('schedule', {}))`), unvalidated.
2. `webui.server.serve` sets `GalleryHandler.existing_ts = scheduling.existing_ts(args.json)` and `GalleryHandler.schedules = scheduling.schedule_data(config['schedule'])` once at startup; every GET re-renders from those class attributes.
3. `scheduling.schedule_data` validates profiles via `schedule_profiles` (rules: see Invariants), then `profile_slots` builds 52 (`SLOT_HORIZON_WEEKS`) weekly `datetime(date, hour, tzinfo=tz)` epochs per profile, first one strictly after now.
4. `scheduling.existing_ts` keeps apparition timestamps >= now whose state != `unpublished` (background occupancy); unpublished ones arrive via `deviantart.load_pending_entries` as `pending[].scheduleTs`.
5. `webui.page.render_page` injects `SCHEDULES` (`[{name, per_slot, slots}]`), `EXISTING_TS`, `PENDING` and `LABELS` (`scheduling.ts_labels`: epoch to `YYYY-MM-DDTHH:MM` in schedule TZ, slots + pending); `preset_options` adds `(custom)` (`__custom__`).
6. `render_page` calls `scheduling.zone` once and hands the `ZoneInfo` to `calendar_view.render_calendar(tz=...)`.
7. JS `openForm` on a new card calls `nextSlotForProfile(SCHEDULES[0])`: first slot with `EXISTING_TS` + queue count < `per_slot` (last slot when full), filled via `tsToLocalInput` (`LABELS` lookup; browser `Date` only for an unlabeled custom instant).
8. `applyPreset` does the same on `<select>` change; `presetForTs` re-selects the profile whose slot list includes a stored ts, else `__custom__`.
9. `readForm` sends `schedule` (the naive picker string) and `scheduleTs` (`localInputToTs`: `LABEL_TO_TS` lookup, else browser-local `Date` parse; used only for client-side occupancy/display).
10. `/stage` calls `store.write_publications` for every entry, `/publish` only for queue entries without a uuid (staged ones publish as-is), `/update` calls `store.apply_update`; both resolve via `scheduling.resolve_ts` in `zone(config['schedule'])`.
11. `scheduling.resolve_ts` prefers `schedule` (`fromisoformat` then `replace(tzinfo=tz)`), falls back to `scheduleTs` (non-UI callers), and raises `ValueError('schedule required')` if neither.
12. The epoch is stored as `apparitions[].apparitionTimestampIfDifferentThanSubmission`; `validate_publications` then `atomic_write_json` persist it. Next startup: pending (`unpublished`, queue) or background (`published_or_scheduled`, `existing_ts`).

## Invariants

- **All weekday/hour/TZ math is server-side; the browser compares absolute epochs by equality, never its own clock.** private-window RFP spoofs `Date` to UTC and re-suggested full slots; `profile_slots`, `nextSlotForProfile` (no `Date`), `test_new_slot_packs_after_taken_slots_regardless_of_browser_tz` (node, 3 TZs).
- **Every slot is the profile's hour in schedule-TZ wall clock across DST, never a fixed UTC offset.** 20:00 Paris stays 20:00 summer and winter; `profile_slots` rebuilds `datetime(day, hour, tzinfo=tz)` per week, `test_schedule_slots_stay_2000_paris_across_dst` asserts hours == `{20}` and two utcoffsets.
- **On save, the naive `schedule` string resolved in the schedule TZ wins over the client `scheduleTs`.** a UTC-spoofed browser sends `scheduleTs` off; `resolve_ts` checks `schedule` first, `test_custom_schedule_resolves_in_config_timezone` (both writers), `test_resolve_ts_reads_a_naive_string_in_the_schedule_timezone`.
- **Profiles need distinct `(day, hour)`, `per_slot >= 1`, a valid weekday, frequency `weekly`; empty list rejected.** same-instant profiles would share one slot list, and failing in `schedule_data` at `serve` startup beats failing mid-edit; `schedule_profiles`, `test_invalid_profiles_are_rejected` (5 cases).
- **Occupancy background = future, non-`unpublished` apparition timestamps; unpublished entries ride in the client queue.** counting a pending entry twice would double-book its slot, past ts never occupy a future slot and would bloat the payload; `existing_ts`, `test_existing_ts_keeps_only_future_scheduled_entries`.
- **The JS between `// >>> scheduler core` and `// <<< scheduler core` in `PAGE_TEMPLATE` is self-contained (inputs `SCHEDULES`, `EXISTING_TS`, `LABELS`, queue; no DOM).** tests run it in node, a DOM ref or new global breaks them; `_schedule_core_js` slices by marker, `_next_slot_for` sets those four globals, `node -e`.
- **The preset choice is never persisted; a stored ts's profile is re-derived by slot-list membership.** the ts alone identifies the slot; `presetForTs` (`SCHEDULES.find(s => s.slots.includes(ts))`), both writers store only `apparitionTimestampIfDifferentThanSubmission`, `publicationsSchema.json` has no profile field.
- **Flat and profiles forms are both legal input to `schedule_profiles`.** back-compat with the pre-profiles single-cadence toml; `test_flat_config_becomes_one_default_profile`, `test_empty_config_falls_back_to_tuesday_2000`.
- Relies on `load_config` passing `[schedule]` through as a plain dict (`{}` when absent), owned by `docs/features/config.md`.

## Gotchas

- `SLOT_HORIZON_WEEKS = 52`: when the horizon is full, `nextSlotForProfile` reuses the last slot (over-booking it) instead of rolling further; bump the constant to queue more than a year out (ponytail comments in `page.py` and `scheduling.py`).
- A custom off-cadence picker time gets its client `scheduleTs` from the browser clock (`localInputToTs` fallback), so UI occupancy can be off by the UTC-spoof offset until reload; the persisted value is right because the server resolves `schedule`.
- Slots and `existing_ts` are computed once at startup (`serve`, `now=time.time()`); a gallery left open across a slot boundary keeps offering a slot that has passed. Restart the app to refresh.
- Stored timestamps are epoch seconds meaning local 20:00 in the schedule TZ; never compare them to UTC 20:00 or redo TZ math in JS. Tests build expected values with `datetime(..., tzinfo=ZoneInfo('Europe/Paris'))`.
- The timestamp lives on the DeviantArt apparition (`entries.deviantart_apparition`); `store.apply_update` raises `KeyError` on an unknown uuid, which `/update` maps to 404.
- The node test in `tests/test_scheduling.py` skips (not fails) without `node` on PATH: a green run without node has not exercised the client scheduler.
- A bad `[schedule]` only fails at gallery startup (`schedule_profiles` via `serve`); `load_config` validates only `[prompts]`, and `nix run .#validate` (`load_config` + `validate_publications`) never touches `[schedule]`.
- `schedule` must be `YYYY-MM-DDTHH:MM`; `resolve_ts` uses `fromisoformat`, so an offset-bearing string is accepted but `replace(tzinfo=tz)` overrides its offset (`'2026-10-15T20:00+00:00'` and `'2026-10-15T20:00'` both give 20:00 Paris).

## Open items

- The last-slot fallback when the 52-week horizon is full has no test (tests only assert `len(slots) == SLOT_HORIZON_WEEKS`; nothing exercises `slot ?? p.slots[p.slots.length - 1]`).
- `schedule_profiles` does not range-check `hour`; the failure surfaces from `datetime()` inside `profile_slots` (`ValueError 'hour must be in 0..23'`, still at gallery startup) with a message that does not name the profile.
