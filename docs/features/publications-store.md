# Publications store (`publications.json`)

- **Does:** Validated, locked JSON persistence for publication phases, shared content, schedules, platform choices and delivery outcomes.
- **Run:** `nix run .#validate -- --data-dir DIR`; `nix run .#echo-first -- --data-dir DIR`.
- **Code:** `src/store.rs`, `src/model.rs`, `src/workflow.rs`, `src/application.rs`, `src/migration.rs`, `assets/publicationsSchema.json`, `src/bin/publicator.rs`.
- **Tests:** `tests/store.rs`, `tests/config.rs`, `tests/migration.rs`, `tests/workflow*.rs`, `tests/publishing.rs`.
- **Config:** DeviantArt option allow-lists and schedule profiles are resolved from the data directory.
- **Data:** Publication phase/title/description/date and file identities live in `publications.json`; apparition records own platform delivery state/options and an optional actual URL. Schema is packaged code.
- **Decisions:** One JSON source of truth. Persistent lock files are not disposable; provider/browser I/O never runs under the mutation lock. See [ADR0010](../adr/0010-rust-core-and-native-iced-interface.md).
- **Verify:** `nix develop -c cargo test --test store --test workflow --test migration --test publishing`

## How it works

Store reads validate the complete document. A mutation obtains the per-store thread and advisory file locks, rereads current bytes, applies one operation, validates and atomically replaces the file. Locks are persistent; do not unlink them while another process could be using them. Long provider/browser operations happen outside the Store lock and merge results into fresh state.

Publication state records completed phases PICKED, SCHEDULED, DESCRIBED and APPEARED. Apparitions retain delivery states/options and actual URLs. Successful delivery is saved per target. `.publicator/` contains durable send intents, receipts and staged uploads used to recover or reconcile publishing across restarts.

## Invariants

- Blank or TODO-marked required metadata cannot advance or publish. Manual approval and generated complete-pair transitions remain distinct.
- Mutations validate before atomic replacement; invalid data is an error, never an empty queue.
- Retry selects fresh registered pending targets and skips recorded successes. A remote send and local durable write cannot be atomic together.
- An unknown delivery outcome is not retried automatically. Preserve its receipt and require explicit reconciliation while holding the publisher lock.
- Keep `.publicator/`, `publications.json.lock`, and data outside source control; ignore them in the separate data repository.

## Gotchas

- The persistent lock file must not be deleted as cleanup.
- A receipt is evidence of an attempted side effect, not proof the remote site rejected it. Resolve ambiguous outcomes before retrying.
- Migration is a pure validated transformation; title recovery never guesses URL slugs or mutates the original browser profile.
