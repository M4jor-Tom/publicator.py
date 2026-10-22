# Scheduling

- **Does:** Allocates and repacks automatic PICKED dates by persisted order while preserving approved dates and manual pins.
- **Run:** `nix run .#ui -- --data-dir DIR` (Schedule phase).
- **Code:** `src/scheduling.rs`, `src/workflow.rs`, `src/store.rs`, `src/ui/`.
- **Tests:** `tests/scheduling.rs`, `tests/store.rs`, `tests/ui_workflow.rs`.
- **Config:** `[schedule]` timezone and weekly profiles are read from the data directory; default timezone is Europe/Paris.
- **Data:** Publication-level `apparitionTimestampIfDifferentThanSubmission`, `scheduleProfile` and `scheduleIsManual`; no platform is selected by scheduling.
- **Decisions:** Timezone and capacity calculations are in Rust; explicit custom dates are never snapped to a profile.
- **Verify:** `nix develop -c cargo test --test scheduling --test store --test ui_workflow`

## How it works

Approved publications and manual PICKED dates reserve capacity once per publication. Remaining automatic dates are allocated in stored order. A schedule approval freezes the date and advances the state; choosing a preset or suggested date clears the pin, while custom edits are retained.

## Invariants

- Local wall-clock times are interpreted in the configured timezone; do not use host/browser clock math.
- Approved dates and manual pins do not move during repacking.
- Capacity limits apply to automatic allocation only. The horizon does not silently overbook.
- Unknown or removed profiles with an existing saved timestamp are preserved rather than erased.

## Gotchas

- A removed preset is represented as a custom date, not silently reassigned.
- A returned publication without an override is not assigned an invented timestamp.
