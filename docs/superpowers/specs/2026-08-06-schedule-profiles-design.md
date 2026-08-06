# Schedule profiles — design

**Date:** 2026-08-06
**Scope:** the `Schedule` section of `docs/todos.md`, first bullet only.
Second bullet (create schedules from the UI) is a deliberate drift — **not** in
this spec. `Non Premium DA` and `publish_next.py split-refactor` are out of scope.

## Problem

Today `publicator.toml` holds one `[schedule]` table (`frequency`, `day`, `hour`,
`per_slot`). The gallery client (`publish_next.py`) auto-fills every new card's
`datetime-local` picker with the next slot on that single cadence. There is no
way to run several cadences at once (e.g. *2 free every Tuesday, 1 paid every
Friday*).

## Goal

Several named schedule profiles, each its own weekly cadence. In the gallery UI,
a per-card preset selector lets the user pick which profile's cadence fills the
picker. No free/paid auto-routing — the user picks the profile. The picker stays
hand-editable. No UI for creating profiles (edit `publicator.toml` by hand).

## Config format

Profiles as a TOML array-of-tables under `[schedule]`:

```toml
[[schedule.profiles]]
name = "free"
day = "tuesday"
hour = 20
per_slot = 2

[[schedule.profiles]]
name = "paid"
day = "friday"
hour = 20
per_slot = 1
```

Per profile: `name` (str, shown in the selector), `day` (weekday name),
`hour` (0-23, UTC), `per_slot` (>=1), optional `frequency` (default `weekly`;
only `weekly` implemented, else `NotImplementedError`).

**Backward compatibility.** If `[schedule]` has no `profiles` but carries flat
`day`/`hour`/`per_slot`/`frequency` keys, read them as a single profile named
`"default"`. If `[schedule]` is absent entirely, fall back to one built-in
`"default"` profile (Tuesday 20:00, per_slot 2) — matches today's defaults.

**Validation at load** (in `_schedules_js`, the plural builder):
- every profile validated as today (weekly-only, valid day, per_slot >= 1);
- profiles must have **distinct `(day, hour)`** pairs. Membership is inferred
  from a timestamp's weekday+hour (see below), so a collision would be
  ambiguous — raise `ValueError` on load.

## Slot computation — earliest open slot (occupancy fill)

Each profile is an independent weekly timeline. "Next slot for profile P" =
the **earliest future** P-slot (next occurrence of P's weekday at P's hour,
stepping +7 days) whose occupancy is `< per_slot`.

Occupancy of a slot = count of already-scheduled timestamps that fall on that
exact slot, drawn from:
- **background**: timestamps of `publications.json` apparitions whose state is
  **not** `unpublished` (already scheduled/published — not shown as editable
  cards);
- **queue**: `scheduleTs` of every entry currently in the client queue
  (pending `unpublished` entries + freshly-added cards).

A timestamp counts toward profile P iff its UTC weekday == `P.day` **and** UTC
hour == `P.hour`. Manual off-cadence edits match no profile and count toward
none — acceptable; the user owns manual overrides.

This is computed **client-side** (same as the current `slotForIndex`). The
server passes the background timestamp **list** (currently only the max,
`INITIAL_MAX_TS`, is passed — replace with `EXISTING_TS`, the list of
non-`unpublished` apparition timestamps). The queue already lives on the client.

Example (per_slot=2, one item already on this Tuesday): a new "free" card
pre-fills **this** Tuesday's open 2nd slot; the next one rolls to next Tuesday.

## UI

`_card_form_html`: add a `<select class="f-preset">` immediately above the
existing `<label>Schedule …</label>` picker, one `<option>` per profile
(`value` = profile name, default = first profile).

- **New card** (`openForm`, no entry): default preset = first profile; picker =
  `nextSlotForProfile(firstProfile)`.
- **Preset change** (`onchange`): set picker = `nextSlotForProfile(selected)`.
- **Re-edit** (`openForm`, existing entry): keep the stored `scheduleTs`;
  pre-select the profile whose `(day, hour)` matches it, else a synthetic
  `"(custom)"` option so the stored time is shown without a matching profile.

The picker remains a normal editable `datetime-local`; nothing else about save,
stage, publish, or the read path changes. `readForm` still reads `scheduleTs`
from the picker only — the preset is a convenience, never persisted.

## Data model

**No schema change, no new persisted field.** Profile membership is derived from
the timestamp. `scheduleTs` is stored exactly as today.

## Touched files

- `validate.py` — `load_config`: parse `schedule.profiles` with the flat/absent
  fallback. Update `_selfcheck` (keep a flat-config assertion, add a
  profiles-config assertion).
- `publish_next.py`:
  - `_JS_DAY` unchanged.
  - `_schedule_js(dict)` → `_schedules_js(dict) -> list[dict]` producing
    `[{name, day (JS index), hour, per_slot}, …]` with the validation above.
  - `serve`/`GalleryHandler`: `schedule: dict` → `schedules: list`; replace
    `initial_max_ts`/`_max_existing_ts` (max) with `existing_ts`/`_existing_ts`
    (the non-`unpublished` timestamp **list**).
  - `_build_page`: `__SCHEDULE__` → `__SCHEDULES__`, `__INITIAL_MAX_TS__` →
    `__EXISTING_TS__` (JSON list).
  - Client JS: `SCHEDULES`, `EXISTING_TS`; replace `slotForIndex` with
    `nextSlotForProfile(p)` (occupancy fill); `openForm` default + re-edit
    pre-select; preset `<select>` `onchange`.
  - `_selfcheck`: update the schedule assertions.

`echo_first_unpublished_publication_data.py` and `da_publish.py` are untouched —
they only render/consume a stored `scheduleTs`.

## Out of scope

- Creating/editing profiles from the UI (deliberate drift).
- Free/paid (price/tier) auto-routing to a profile.
- `Non Premium DA` GitHub-Actions execution; `publish_next.py` split-refactor.

## Testing

Extend the existing offline `--selfcheck` self-checks (no framework):
- `load_config`: flat fallback → one `"default"` profile; profiles array →
  parsed list; absent → built-in default.
- `_schedules_js`: profiles list mapping; reject non-weekly, per_slot < 1,
  colliding `(day, hour)`.
- Occupancy JS is exercised via the existing manual gallery run; the Python
  self-checks cover config + profile parsing.
