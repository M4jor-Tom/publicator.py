# Worker boundary and operator safety

These items concern the optional DOM prober, human recovery contract and cross-process behavior.
Do not weaken unknown-outcome protection to make a retry convenient. Use private temporary paths
and synthetic sessions; never test through the real prober IPC paths or browser profiles.

## PUB-08 — P1 · Defect · Optional prober executes attacker-precreated temporary files

**Evidence:** `python/publicator_da/pw_daemon.py:22–24,39–41,51–61` uses predictable public temporary
paths, accepts an existing command endpoint without type/ownership checks, reads and execs its
contents. Log/screenshot endpoints also lack ownership/symlink checks. Confidence: high.

**Conditional impact:** On a multi-user machine another user can precreate readable `/tmp/pw.cmd`
containing Python; starting the explicit prober executes it as the operator. This is **not** the
production NDJSON worker, and no exploit was executed during the audit.

**Existing coverage:** Browser profile-root protection is tested, but `tests/python` has no prober
IPC tests. Profile immutability does not validate command-file ownership.

**Regression / close when:** Redirect paths into a private test directory; precreate a regular file
with a harmless sentinel action, a symlink and wrong-owner endpoints where supported. Mock browser
acquisition; reject unsafe endpoints before acquiring the browser or executing content. Establish
private user-owned storage and validate retained endpoints/log destinations. Do not use real
`/tmp/pw.*` or create an attack against another user's process.

## PUB-09 — P1 · Defect · Manual fallback can repeat an unknown submission

**Evidence:** `.claude/skills/publish-deviantart/SKILL.md:22–25,47–72` directs every nonzero automation
exit into manual upload/scheduling without first checking commitment or reconciling its intent.
This conflicts with `docs/features/deviantart-publish.md`. Confidence: high.

**Trigger/impact:** Remote scheduling commits but its response is lost. The CLI exits nonzero;
following the manual fallback creates a duplicate outside Rust's automatic retry block.

**Existing coverage:** `tests/cli.rs:772–800` checks nonzero unknown and blocked automatic retries.
The step checker compares numbered steps only (`python/publicator_da/browser.py:191–209`), not the
recovery prose. Passing check-steps therefore cannot close this finding.

**Regression / close when:** Drive the CLI unknown fixture and inspect operator-facing guidance.
It must distinguish safe failure from unknown, require remote inspection/reconciliation before
any repeat submission, and preserve unresolved receipts. Add a focused consistency assertion if
multiple recovery instructions remain. No real remote operation is needed to validate guidance.

## PUB-R2 — P3 · Gap / decision · No Rust-to-real-Python protocol integration proof

**Evidence:** Rust publisher tests use shell workers (`tests/publishing.rs:259`); Python subprocess
tests call real `serve` with FakeSession (`tests/python/test_worker.py:121`). No inspected test
connects Rust Publisher to that actual Python server.

**Boundary differences to classify:** `python/publicator_da/worker.py:37–59` permits NaN/+infinity
price through comparison, checks image path only for nonempty/non-WebP suffix, and accepts empty
metadata. Session/job timezone equality is not required. Python timezone construction does not
explicitly reject nonexistent local times as Rust does (`src/publish_protocol.rs:143–165`).

**Reachability limits:** These are not proven exploits through normal Rust publishing. Store JSON
cannot supply NaN/infinity; Rust validates prices, resolves non-symlink artwork, validates/prepares
uploads, and generates matching timezones (`src/publishing.rs:424–463,673–707`,
`src/images.rs:339–378,446–465`, `src/store.rs:65–74`). PUB-02 separately owns mutable-source races.
Python already validates trusted profile roots before session acquisition and nonzero minutes
before mutation (`tests/python/test_worker.py:30,49,143`).

**Regression / close when:** First state the worker trust boundary. Drive real Python `serve` from
Rust with a test-only injected session factory; record which requests reach submit. Share a corpus
of valid jobs, nonfinite prices, empty fields, nonexistent/outside/disguised paths, timezone mismatch,
DST gaps/folds, invalid version/attempt/UUID, and malformed/truncated/oversized/UTF-8-boundary frames.
Assert equivalent required guarantees and document intentionally Rust-owned checks. Keep profile
root protection and immutable original profiles. No production fake mode or redundant framework.

## PUB-R3 — P3 · Risk · Abrupt abortion differs from cooperative cleanup

**Evidence:** `src/publishing.rs:205–225,710–719` awaits cleanup after normal batch return; worker
spawn has no drop-cleanup policy. Diagnostic draining after group shutdown lacks its own deadline.
No abrupt-abort or detached-pipe failure was reproduced.

**Conditional impact:** Dropping/aborting the future or terminating the CLI can skip cooperative
ownership. A detached descendant retaining stderr can prevent the drain from completing.

**Existing coverage:** `tests/publishing.rs:461,976` and
`tests/ui_publish_calendar.rs:422,613` cover cooperative cancellation and same-group descendants.
Do not describe those normal paths as absent or broken.

**Regression / close when:** Use gated synthetic processes with PID witnesses. Abort an in-flight
task and separately interrupt its CLI; inspect children, publisher lock and durable pending intent.
Test a deliberately detached stderr-holder in a watchdog-owned process. Require bounded teardown
and explicit fixture cleanup without losing retry-blocking unknown state. Test-only outer watchdogs
must kill/reap their own descendants even when the current implementation fails the assertion.
