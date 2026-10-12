# Scheduling (slots and weekly profiles)

- **Does:** Repackages automatic PICKED date suggestions in persisted order; manual pins and approved dates reserve capacity and never move automatically.
- **Run:** `nix run .#publish-next -- --data-dir DIR` (Schedule phase; weekly profiles are read-only)
- **Code:** `src/publicator/scheduling.py` (`schedule_profiles`, `schedule_data`, `profile_slots`, `repack_publication_dates`, `resolve_ts`, `zone`), `src/publicator/store.py` (date edits/approval and repack mutation), `src/publicator/webui/server.py` (HTTP boundary), `src/publicator/webui/page.py` (native picker).
- **Tests:** `tests/test_scheduling.py` (capacity, exhausted horizon, DST and historical occupancy), `tests/test_store.py` (manual pins, unpin/profile edits, frozen dates), `tests/test_server.py` (real HTTP repacks/approval).
- **Config:** `[schedule] timezone` defaults to Europe/Paris; weekly `profiles` specify name, day, local hour, per_slot and optional DA default_tiers. Flat configuration remains one default profile. Profiles need distinct day/hour slots and valid configured tier names.
- **Data:** Publication-level `apparitionTimestampIfDifferentThanSubmission`, `scheduleProfile`, `scheduleIsManual`; no platform is selected in Phase 2 and no second date field exists.
- **Decisions:** All allocation/TZ conversion is Python-only. Explicit over-capacity dates stay chosen; off-cadence custom dates are never snapped to cadence.
- **Verify:** `nix develop -c pytest tests/test_scheduling.py tests/test_store.py tests/test_server.py tests/test_page.py -q`

## How it works

Reserve each approved publication and each manual PICKED date once, regardless
of apparition count; allocate remaining automatic PICKED items in stored order.
Edits and repacking share one validated write. Approval freezes the date and
advances to SCHEDULED. Choosing a preset or “Use suggested date” clears the pin;
a custom picker edit pins the chosen instant.

## Invariants

- A local 20:00 schedule stays 20:00 in the configured timezone across DST, not UTC 20:00. Native picker labels and saves use that timezone; the browser never reads `Date`, allocates slots, or converts epochs.
- Approved SCHEDULED/DESCRIBED/APPEARED dates and manually pinned PICKED dates do not move during repacks. Historical dates without an override reserve their submissionTimestamp fallback.
- Capacity applies to automatic allocation only. Explicit same-slot dates can exceed capacity and the UI reports this; off-cadence instants occupy no configured slot.
- Exhausting the 52-week horizon leaves an automatic date absent, not silently overbooked. Approval is blocked until a date is chosen.
- Explicit dates with no configured matching profile (including a removed preset) return to scheduling pinned as Custom. Manual edits preserve that profile absence/mismatch; only preset choice or “Use suggested date” assigns a valid profile. Returning without an override invents no date.
- DA profile default_tiers initialize a later Phase 4 selection only; they are never instructions to choose a platform or overwrite historical options or profile-less Custom dates.

## Gotchas

- A schedule horizon is regenerated on request/mutation; stored suggestions move only on explicit repacks, not on GET. Increase SLOT_HORIZON_WEEKS if the queue legitimately exceeds a year.
- `resolve_ts` prefers the naive `schedule` wall-clock string over `scheduleTs`; non-UI callers may supply an epoch. Offset-bearing strings are interpreted in the configured zone, not their embedded offset.
- The calendar is read-only and uses the same timezone; it does not duplicate schedule controls.
