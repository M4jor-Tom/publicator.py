# Recorded validation and limits

This is **baseline evidence for the reviewed SHA**, not a fresh result for future fixes.
Source target: `fab0d03a49951d4482c1c997f30fb6939786306f`, branch master.
Audit date: 2026-10-30. Parent checked unchanged HEAD and empty staged/unstaged diffs/status.
The subsequent change creating this review directory is documentation-only.

Five source reviewers inspected core, publishing, metadata, assets and UI. A sixth lane inspected
CI/test safety and ran commands. The parent challenged/deduplicated findings and inspected command
logs. This review exhaustively carries the reported findings/gaps, not every conceivable defect.

## Observed command results

All commands used the project's Nix environment, with normal Rust test parallelism.
`PYTHONDONTWRITEBYTECODE=1` was set for suite/smoke commands; caches/logs stayed outside Git.
The shell timeout wrapper sent TERM, with the listed kill-after bound.

| Command inside `nix develop -c` | Timeout / kill-after | Observed result |
|---|---|---|
| `cargo fmt --check` (first attempt) | 180s / 15s | Exit 124 in Nix dependency retrieval; Cargo never started |
| `cargo fmt --check` (authorized retry) | 600s / 15s | Exit 0 |
| `env PYTHONDONTWRITEBYTECODE=1 cargo test --locked --all-targets --features desktop` | 1200s / 20s | Exit 0; 390 passed, 0 failed/ignored, 30 top-level targets |
| `env PYTHONDONTWRITEBYTECODE=1 cargo test --locked --all-targets --no-default-features` | 900s / 20s | Exit 0; 278 passed, 0 failed/ignored, 26 top-level targets |
| Python command below | 300s / 15s | Exit 0; 36 passed + 11 subtests passed |
| `env PYTHONDONTWRITEBYTECODE=1 cargo run --locked --bin publicator -- check-steps` | 180s / 15s | Exit 1; default worker permission failure |
| Focused metadata command below | 180s / 15s | Exit 0; 1 passed despite 2 fixture thread panics |

Python command:

```sh
nix develop -c env PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=python \
  pytest tests/python -q \
  -o cache_dir=/tmp/publicator-review-validation/pytest-cache
```

The initial timeout is preserved, not hidden by the retry. It was dependency setup, not a hung or
failed Rust test. No host Python, old shell, alternative execution protocol or serial-only fallback
was substituted. After the development check failed, no wrapper injection or Nix-alias retry was
used to force green.

Rust totals exclude two nested one-test subprocess summaries per suite (otherwise a naive sum
would say 392/280). Counts include repeated fixture helper tests and are not unique behaviors.
Desktop/headless overlap and must not be added. Headless reused target/, so this is not pristine
feature-build proof; UI-only targets are absent or run zero tests. No top-level ignored tests were
reported. Expected negative-fixture stderr is distinct from a suite failure.

## Runtime witness: META-02 false-green

Exact focused command, under the 180-second wrapper:

```sh
nix develop -c cargo test --locked --test local_metadata \
  local_chat_rejects_malformed_or_empty_metadata -- --nocapture
```

Both malformed/empty cases produced the same fixture mismatch:

```text
thread '<unnamed>' panicked at tests/common/http.rs:56:17:
assertion `left == right` failed: unexpected test HTTP path
  left: "/api/chat"
 right: "/api/tags"
test local_chat_rejects_malformed_or_empty_metadata ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out
EXIT_CODE=0
```

There were **two** thread panics; one representative excerpt is shown. Direct chat does not first
request tags. Generic is_err accepts the resulting transport error and Drop ignores the join panic,
so the malformed responses never reach the parser. This is an assertion-quality failure despite
a passing harness, not an application malformed-response reproduction.

## Runtime witness: VER-07 default worker launch

The direct Cargo check-steps compiled successfully, then reported:

```text
I/O error: Permission denied (os error 13)
EXIT_CODE=1
```

The fallback executes mode-0644 worker.py directly; it has relative imports and no executable
shebang. Existing tests override it with a shell launcher. Packaged Nix apps inject a module
launcher and were not tested here; do not label them failed based on the development command.
The actual worker never ran, so this is not evidence of a numbered-step mismatch.

## Infrastructure failure and provenance

Workflow `05f19cb5-6596-4003-bcc9-3b9be1f0de0c` completed with five source children successful.
Verification child `556a3a41-df41-41bd-89f1-7f831debaa9e` hit its **1,800,000 ms / 30-minute limit**
after writing its main report and completing the focused command, before finalizing that follow-up.
Its status is **failed**, not a successfully completed verification lane.
The parent recovered and independently checked existing logs/partial report; it did not resume or
change execution protocol. No owned validation process remained at the final audit check.

Optional original scratch evidence: `/tmp/publicator-review/evidence/` contains fmt.log,
fmt-retry.log, rust-desktop.log, rust-headless.log, python.log, check-steps.log,
local-metadata-focused.log, coverage-tools.log and workflow-receipt.json.
Raw lane reports were saved separately under `/tmp/publicator-review/reviews/`.
These paths are disposable provenance only: the findings, commands, results and relevant witnesses
are preserved in this directory so fixes do not depend on access to an old agent session.

## What this audit did not establish

- No native-window interactions or actual keyboard/focus/geometry acceptance.
- No built Nix CLI/desktop/GPU wrapper smoke tests, full quality/Clippy/metrics run or flake check.
- No line/branch coverage percentages. Inspection found no cargo-llvm-cov, cargo-tarpaulin,
  llvm-cov, llvm-profdata, grcov, coverage/coverage3, Python coverage or pytest_cov in the shell.
- No real login/profile use, model installation, provider credential spending or publishing.
- No current live-site selector/authentication compatibility or real remote success/failure.
- No new runtime witness for every source-reported race; their trigger prerequisites remain explicit.

## Verification after authorized fixes

Run affected regressions first, then appropriate gates; do not repeat unchanged expensive checks
merely because the documentation was moved. Keep each result tied to its actual tested commit.

```sh
nix develop -c cargo fmt --check
nix develop -c cargo test --locked --all-targets --no-default-features
nix develop -c cargo test --locked --all-targets --features desktop
nix develop -c env PYTHONPATH=python pytest tests/python -q
nix run .#check-steps
nix run .#quality
```

Also prove pristine headless execution, built wrappers from unrelated CWD, relevant crash/race
transitions, and bounded native replay where changes require them. These additional checks are
outstanding, not claimed results. The current manual smoke waits for a human and is not unattended
acceptance. Step-list comparison does not validate recovery prose, browser effects or selectors.
The flake has no checks output and package doCheck=false; flake check alone is not an application
or package-test result. Retain normal parallel tests and explicit process cleanup.

Durable Store locks/atomic replacement, scheduler clock injection/DST, marker-based batch merges,
consent, recovery receipts, real tiny image resizing and synthetic UI task tests are existing
strengths. Their passing baseline is valuable, but false-green assertions and missing safety edges
mean **full testability and coverage cannot yet be certified**.
