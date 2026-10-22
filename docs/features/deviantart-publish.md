# DeviantArt publishing

- **Does:** Rust validates and journals each approved pending target, then drives the retained Python Playwright worker through the DeviantArt submit flow.
- **Run:** `nix run .#da-publish -- --data-dir DIR [--uuid ID | --all] [--headless]`; `nix run .#check-steps`; `nix run .#login`; `nix run .#pw-daemon` for explicit DOM probing.
- **Code:** `src/publishing.rs`, `src/publish_journal.rs`, `src/publish_protocol.rs`, `python/publicator_da/`, `.claude/skills/publish-deviantart/SKILL.md`, `src/bin/`.
- **Tests:** `tests/publishing.rs`, `tests/publish_protocol.rs`, `tests/python/test_worker.py`, `tests/python/test_browser.py`.
- **Config:** Data-dir tags and DeviantArt allow-lists are validated by Rust before worker startup.
- **Data:** `.deviantart-login/` is the human-owned original profile; `.deviantart-session/` is a disposable working copy. `.publicator/` holds persistent receipts/intents/staging; `.publicator/publish.lock` excludes concurrent publishers.
- **Decisions:** Rust owns database/config/validation/recovery. Python receives one bounded job at a time and never reads/writes publication data. Keep these data paths ignored in the separate data repository.
- **Verify:** `nix develop -c cargo test --test publishing --test publish_protocol`
- **Verify:** `nix develop -c env PYTHONPATH=python pytest tests/python -q`
- **Verify:** `nix run .#check-steps`

## How it works

Rust validates the complete selected batch and current approved record before dispatch, writes a durable intent, and sends one artwork at a time to the versioned NDJSON worker. A result is saved before the next send. Success receipts prevent duplicate submissions on restart. If the remote outcome is unknown, preserve the receipt and stop; human reconciliation is required before retry.

The Playwright worker uses the existing ordered browser steps and static skill. Rust binds the Store data directory at process startup with `--data-root`; the worker accepts only the `.deviantart-login` and `.deviantart-session` children of that trusted root, while protocol v1 carries no root authority. The login profile is never opened for publishing: Rust/worker use a copy. DeviantArt blocks automated login behind PerimeterX, so `login` remains a human-operated real Firefox action.

## Invariants

- The complete source document, target, options, tags, schedule and artwork identity validate before sending.
- Python receives no publication database/config path and cannot mutate the Store.
- Per-target receipt and Store state survive restart; do not delete `.publicator/` to clear a failure.
- An ambiguous external result is reconciled explicitly, never automatically resubmitted.
- `.webp` is converted before upload; STEPS ordering remains synchronized with the static skill.
- The original browser profile is read-only; publish uses a separate working profile.

## Gotchas

- A browser flow may have created a remote draft before failing. Check DeviantArt before any manual retry.
- `pw-daemon` is a debugging DOM prober, not the production worker or login flow.
- Never run the real publisher for validation or change a real profile/data directory.
