# Remaining core coverage

These are missing proofs, not additional demonstrated product failures. Delete each block only
when its specific behavior is covered or an explicit decision establishes it is unnecessary.
Store locking, atomic replacement, normal successful-delivery guards, DST resolution and stable
Calendar ordering already have meaningful tests; do not replace those mechanisms wholesale.

## CORE-C01 — P2 · Gap · Existing extension fields need a mutation round trip

**Evidence:** `tests/store.rs:11,179` exercises generic JSON preservation, but the first extension
example only reads; it does not mutate a document containing extensions at every nested level.

**Regression / close when:** Put distinct sentinel extensions on publication, file and apparition
objects. Perform an unrelated successful workflow edit, reread disk and assert all sentinels,
array order and untouched fields survive. Use existing temporary rows and Store, no media.
This protects forward-compatible data, not just parser acceptance.

## CORE-C02 — P2 · Gap · Deterministic precommit failure coverage is incomplete

**Evidence:** `src/store.rs:114–199` has private commit injection and typed post-rename uncertainty;
`tests/store.rs:31–108` exercises rejection and filesystem failures. Write, flush and file-sync
faults lack equivalent deterministic coverage; chmod-based failures depend on user privileges.
Publishing tests already reuse related recovery hooks (`src/publish_journal.rs:1128,1156`).

**Regression / close when:** Extend existing test-only failure points, not a storage framework.
Inject staging-write, flush, file-sync and rename failures; old bytes remain readable and intact,
errors surface, and locks release. Post-rename uncertainty must retain new bytes and its distinct
error rather than masquerade as rollback. Run permission-based cases in an unprivileged child or
state the limitation explicitly. Fake sync failures do not prove actual power-loss durability.
Application-level provider/commit combinations belong to META-C01, not just these Store tests.

## CORE-C03 — P3 · Gap · Process-lock proof depends partly on elapsed time

**Evidence:** `tests/store.rs:113,223` covers thread/process exclusion and persistent lock files,
but child readiness is signaled before attempting mutation; contention includes a 150-ms window.

**Regression / close when:** Add bounded synchronization at the real lock-attempt boundary.
Prove a second writer cannot enter while the first owns the lock, then succeeds after release;
include symlink aliases and restart semantics already supported. Kill/reap failed helper children.
Do not remove the sibling lock file while writers can exist or serialize the entire suite to hide
contention. Preserve executable-fixture inherited-descriptor regressions.

## CORE-C04 — P2 · Gap · Scheduling boundary vectors need direct assertions

**Evidence:** `tests/scheduling.rs:20,80,107,169,208` covers timezone/DST, capacity and reservations;
`src/scheduling.rs:172–326` already accepts explicit `now`. Removed-profile preservation is covered
in `tests/workflow_dates.rs:54–105`; do not list that implemented behavior as missing.

**Regression / close when:** With fixed epochs, assert behavior when `now` is exactly a candidate
slot, weekly slots land in DST gap/fold hours, multiple profiles consume rows in persisted order,
and all 52 slots are exhausted. Preserve manual pins, successful history and removed-profile
reservations. Use in-memory rows and the existing clock parameter; no new global clock service.

## CORE-C05 — P3 · Gap · Calendar extremes and migration idempotence need explicit checks

**Evidence:** `tests/calendar.rs:16–159` covers per-apparition history, tie ordering, fallback dates,
URLs and configured-zone month boundaries. `tests/migration.rs:28–201` covers representative legacy
transforms, not a broad historical idempotence matrix.

**Regression / close when:** Cover leap-February, year rollover, zone-crossing month boundaries,
and explicit rejection of out-of-range timestamps. Run valid historical/mixed migration output
through migration again and compare complete data, including extensions and URLs. Use pure events,
legacy rows and recovery maps, fixed epochs and no artwork. Keep CORE-02's mixed-success invariant
as a distinct integration proof, not merely a Calendar formatting test.
