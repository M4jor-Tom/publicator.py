# Core data, configuration and history

Open items at `fab0d03`; source paths are repository-relative. See [retirement rules](README.md).
Contracts: `docs/features/config.md`, `docs/features/publications-store.md`,
`docs/features/scheduling.md`, `docs/features/calendar.md`.

## CORE-01 — P1 · Defect · Duplicate UUIDs address the wrong publication

**Evidence:** `assets/publicationsSchema.json:4,8–10` checks UUID length, not document-wide
uniqueness. `src/config.rs:205–231` accepts duplicates; `src/workflow.rs:43–46` selects the first
match. Real UI saves use that lookup (`src/ui/mod.rs:2266–2282`), as do publishing snapshots
(`src/publish_journal.rs:144–147`). Confidence: high; no runtime reproduction claimed.

**Trigger/impact:** Import/manual editing creates otherwise valid rows sharing a UUID. Validation
succeeds, edits affect only the first row, and the second cannot be independently addressed.

**Existing coverage:** `tests/config.rs:262,285` rejects malformed values; `tests/store.rs:49`
checks rollback. Neither establishes uniqueness.

**Regression / close when:** Create two valid temporary rows with the same UUID and different
titles/basenames. Shared validation and Store reads reject them; mutation never invokes its
callback and leaves original bytes untouched. No image files are needed. Reject ambiguity at the
shared boundary; do not silently deduplicate or choose a winner.

## CORE-02 — P1 · Defect · Partial delivered history can move during unrelated repacking

**Evidence:** `src/migration.rs:17–44` marks only all-success history APPEARED. Mixed delivery
without a date override becomes PICKED. `src/scheduling.rs:280–324` moves nonmanual PICKED rows
without checking delivery success. Pick/date actions repack (`src/workflow.rs:108,235,285`);
Calendar uses the shared publication date (`src/calendar.rs:41,75`). Confidence: high.

**Trigger/impact:** A legacy row has complete metadata, an old timestamp, one successful target,
one pending target and no override. **After migration output is adopted**, editing another row's
date moves this historical event. Migration is not automatic. Direct success guards do not protect
against this indirect mutation.

**Existing coverage:** `tests/migration.rs:28,193` covers all-success/all-pending rows;
`tests/scheduling.rs:107` preserves APPEARED reservations; `tests/workflow_dates.rs:144` tests direct
locks. The mixed migration → Store → unrelated edit → Calendar chain is absent.

**Regression / close when:** Migrate that row with `https://example.invalid/history`, persist it
beside a normal PICKED row, and edit only the latter with fixed `now`. Assert identical historical
effective dates, successful apparition, URL and Calendar event. Empty file arrays suffice.
Successful history must remain reserved regardless of editable phase; migration must retain usable
phase semantics and be idempotent. Never repair this by clearing recorded success.

## CORE-03 — P2 · Defect · Public validation accepts invalid schedule semantics

**Evidence:** `Config::load` returns an unvalidated schedule table (`src/config.rs:87–138`). Rules
exist in `src/scheduling.rs:58–99,259–266`, but CLI validation only loads config and reads Store
before printing Valid (`src/bin/publicator.rs:108–113,150–154`). Confidence: high.

**Trigger/impact:** With `publications.json = []`, TOML containing hour 24, numeric timezone, or
empty profiles passes public validation and fails only when scheduling is used.

**Existing coverage:** `tests/scheduling.rs:64,201` calls validators directly. Configuration and
CLI tests cover parsing/publication shape, not these public-boundary semantics.

**Regression / close when:** Parameterize the three invalid configurations through
`publicator validate --data-dir DIR`; require nonzero exit and schedule-specific diagnostics.
Retain missing-config success. Reuse the existing validators at the shared boundary; do not copy
the rules into independent UI/CLI implementations.

## CORE-04 — P2 · Defect · Migration can discard malformed recognized fields

**Evidence:** Legacy extraction ignores non-string `urlElsePublicationName` values
(`src/migration.rs:67–77`). Transformation removes the field before checking its type
(`src/migration.rs:173–186`); final validation only sees the output (`48–58`). Confidence: high.

**Trigger/impact:** A number/object occupies this recognized legacy title/URL field, but another
valid title exists or recovery supplies one. Migration succeeds after dropping the malformed
payload. The input snapshot remains untouched, but its returned migration loses the evidence.

**Existing coverage:** Conflicts, missing/empty recovery and input immutability are covered in
`tests/migration.rs:62,90,165`, not malformed recognized types.

**Regression / close when:** Supply numeric and object legacy values with a valid publication
title. Require contextual errors and unchanged input. Preserve supported missing/empty recovery,
conflict detection and pure-transform behavior. Validate recognized types before removing them.

## CORE-05 — P2 · Defect · Optional apparitions are readable but not selectable

**Evidence:** `assets/publicationsSchema.json:8,22–25` permits omission; Calendar treats it as
empty (`src/calendar.rs:54–57`). Selection instead requires an array
(`src/workflow.rs:377–380`), reached by the UI at `src/ui/mod.rs:1964–1966`. Confidence: high.

**Trigger/impact:** A Store-valid DESCRIBED row with complete metadata omits apparitions.
Selecting DeviantArt fails; in select-all, this one row aborts the whole transaction.

**Existing coverage:** `tests/calendar.rs:137` covers omission. Selection fixtures in
`tests/workflow_platforms.rs:23` and `tests/common/mod.rs:14–16` always include an empty array.

**Regression / close when:** Remove apparitions from a valid row and establish successful Store
read, single selection producing one pending target, select-all beside another valid row, and
safe deselection of absence. Treat absence as empty without accepting malformed non-arrays or
silently tightening historical read compatibility. No artwork/provider is needed.
