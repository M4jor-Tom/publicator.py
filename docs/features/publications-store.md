# Publications store (publications.json + schema)

- **Does:** Validated, locked read-modify-atomic-replace persistence for picking, approvals, drafts, schedule repacks, platform selections/options and delivery successes.
- **Run:** `nix run .#validate -- --data-dir DIR`
- **Run:** `nix run .#echo-first -- --data-dir DIR` (first metadata-approved pending DeviantArt target)
- **Code:** `src/publicator/store.py` (mutations), `src/publicator/entries.py` (states/readers), `src/publicator/publicationsSchema.json` (schema), `src/publicator/config.py` (schema + option allow-lists), `src/publicator/migration.py` (pure legacy transformation), `src/publicator/apps/validate.py`, `src/publicator/apps/echo_first.py`.
- **Tests:** `tests/test_store.py`, `tests/test_config.py`, `tests/test_migration.py`, `tests/test_server.py` (HTTP persistence and reopening), `tests/test_platforms.py` (delivery/retry).
- **Config:** `[deviantart]` tiers/galleries are validated on every write; `[schedule]` timezone/profiles control date resolution/repacking. Callers should pass their data-directory config explicitly.
- **Data:** `publications.json` owns shared title/description/date, UUIDs, file identities and completed phase; `apparitions[]` owns platform delivery states/options and optional HTTP(S) URL. Schema resolves via `__file__`, never the data directory.
- **Decisions:** One JSON database, no runtime migration/title lookup; historical platform names do not require executable registration.
- **Verify:** `nix develop -c pytest tests/test_store.py tests/test_config.py tests/test_migration.py tests/test_server.py tests/test_platforms.py -q`

## How it works

Publication state records **completed** phases:

| State | Completed action | Next action |
|---|---|---|
| PICKED | File identity saved, deduplicated by content hash | Approve a date |
| SCHEDULED | Explicit date approval | Write/generate and approve shared metadata |
| DESCRIBED | Valid title and description explicitly approved | Select platforms, submit/retry |
| APPEARED | Every selected apparition accepted, at least one selected | View history, or add an undelivered platform |

APPEARED includes scheduled submissions, not only publicly visible artwork.
Adding an undelivered platform returns an APPEARED publication to DESCRIBED;
successful targets are never unchecked or resubmitted. Removing the last pending target restores APPEARED when all remaining selections succeeded. Before any delivery
success, explicit backwards actions preserve saved content and pending selections.

## Invariants

- Publication `apparitionTimestampIfDifferentThanSubmission` is the only date override; historical records fall back to `submissionTimestamp`. No apparition-level override is allowed. `title` and `description` are shared across platforms.
- Apparition delivery state remains `unpublished` / `published_or_scheduled`; optional `url` is an actual HTTP(S) URL. `urlElsePublicationName` is rejected, never written as a compatibility alias.
- Blank or marker-bearing required metadata cannot advance or publish; the exact token is `[[PUBLICATOR:TODO_GENERATE]]`, blocked even inside other text. Generation itself does not approve metadata.
- Every store mutation validates before same-directory temporary-file replacement. A standard-library thread lock plus advisory file lock protects short mutations; neither is held during AI/browser calls. Invalid JSON is an error, never an empty queue.
- Delivery successes are persisted individually. Retry resolves fresh stored approved targets and skips successes. External submission and subsequent disk persistence cannot be one atomic transaction.

## Gotchas

- Legacy migration is a pure, tested, idempotent transformation. Conflicting names/dates fail rather than discard data. UUIDs, hashes, filenames, historical options and unrelated fields survive. New-contract empty drafts remain intentionally empty on reruns.
- Empty legacy titles need verified artwork-page titles before replacement; never infer from numeric IDs or URL slugs. Recovery uses a temporary copy of the named Firefox privacy profile headlessly, **not** private-window mode. No original browser profile may be mutated.
- Migration must preflight the source snapshot, validate the complete transformed array, and compare the original again before replacement. Keep backups/audits outside Art; Art changes remain unstaged/uncommitted for human inspection.
- The sibling `publications.json.lock` is persistent runtime state, not publication data. Do not unlink it while writers may be active; keep it out of the data repository's index.
- Specs, plans, trackers and reports stay in `/tmp` or managed scratch space, never committed. Feature docs hold maintained operational contracts, not implementation diaries.
