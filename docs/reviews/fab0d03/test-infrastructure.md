# Test gates, packaging and fixture safety

Recorded execution is in [validation](validation.md). Passing suites do not discharge the gaps
below. META-02 owns false-green provider assertions; UI-C01 owns discarded lifecycle actions.

## VER-02 — P2 · Gap · CI omits the production Python browser/worker tests

**Evidence:** `.github/workflows/quality.yml:18–38`, `flake.nix:134–145` and
`scripts/quality.sh:16–20` run gate unittest tests, Rustfmt/Clippy, desktop Rust and metrics.
They do not run pytest's browser/worker functions. Broadening unittest's pattern alone cannot
execute those pytest-style tests. Confidence: high in inspected wiring.

**Impact/coverage:** Profile or submit logic can break while Rust imports/step strings stay green.
`tests/python/test_quality.py:194–208` spies on tools for Git-environment cleanup;
`tests/packaging.rs:15–66` exercises real-worker step checking, not browser behavior.

**Regression / close when:** Assert the gate invokes full `PYTHONPATH=python pytest tests/python`
and propagates nonzero exit even when other checks succeed. In a disposable fixture introduce a
harmless failing pytest test and observe gate failure. Keep discovery limited to tests/python,
existing Git-environment cleanup and the Rust quality checks. No live browser/service is needed.

## VER-03 — P2 · Gap / risk · Headless and real packaged runtime are not gated

Also incorporates **UI-R03**; no separate package issue needs to be maintained.

**Evidence:** `scripts/quality.sh:18–20` always enables desktop; Cargo defaults are empty
(`Cargo.toml:6–9`). `flake.nix:33–85,126–145,178–199` has no checks output and both packages set
doCheck=false. `tests/packaging.rs:6–66` executes Cargo binaries with copied/injected assets and
checkout PYTHONPATH, not the Nix wrappers. A dev shell can mask missing runtime dependencies.
Desktop URL opening calls xdg-open (`src/ui/calendar.rs:426–439`), not explicitly supplied by
rustRuntime (`flake.nix:48,91–95`); this is unverified packaging risk, not a proven missing binary.

**Regression / close when:** Independently gate no-default-features with a fresh target directory;
desktop-only CLI tests cannot rely on stale UI executables. Build CLI/desktop packages, then from
an unrelated temporary CWD with controlled PATH and no checkout PUBLICATOR_WORKER/PUBLICATOR_SKILL/
PYTHONPATH, exercise embedded-schema validation, packaged check-steps and UI help. Capture a harmless
URL-opener request without opening a real browser. A deliberately missing runtime asset must fail.
Keep GPU feature validation separate. Do not count an empty flake checks set or successful Cargo
asset injection as proof of the installed closure. No package build ran during this audit.

## VER-04 — P2 · Defect · Dummy-data generator overwrites supplied destinations

**Evidence:** `scripts/dummy-data.sh:6–19,40–49,59–70` creates/reuses the supplied directory, rewrites
config/art and truncates publications without emptiness/fixture-ownership checking or Store locking.
The optional path is advertised by `docs/features/gallery-ui.md:4`. Confidence: high, source-only.

**Impact/coverage:** A typo, symlink or repeated fixture initialization can destroy saved state.
No test references the generator. The acceptance example's create-new/preservation protection
(`examples/ui_acceptance.rs:526–537,1168–1175`) does not protect this script.

**Regression / close when:** Put sentinel config/database/art bytes in a nonempty temporary root
and a symlink to it. Running the generator refuses before tool work or byte changes. A fresh root
still generates a valid fixture. PATH-spied magick/jq can prove refusal happens first. Never run
the damaging witness against real data; retain safe default mktemp behavior. Preserve reopened
fixture state rather than regenerating the evidence being tested.

## VER-05 — P3 · Defect · Unmet HTTP expectations can hang the test harness

**Evidence:** `tests/common/http.rs:41–46,61–63,80–82,95–101` uses unbounded accept/gate waits.
finish takes the handle and joins, preventing Drop's wakeup from running while accept is blocked.
Socket read/write deadlines do not bound those waits. Confidence: high in source; not executed.

**Trigger/impact:** A regression returns without an expected request; callers such as
`tests/ui_describe.rs:965–989` and `tests/application_ai.rs:201–231` can wait until the outer CI
job timeout instead of reporting unmet expectations. Normal consumed-response and released-gate
cases pass; that does not cover missing requests. This is distinct from META-02's ignored panic.

**Regression / close when:** A watchdog-owned helper starts a one-reply server then finishes without
requesting it; another leaves a gate unreleased. Both terminate boundedly with useful diagnostics.
Ensure shutdown wakes accept, reports thread failure and kills/reaps fixture children without double
panic during unwinding. Preserve ordinary parallel tests and existing executable-fork safety;
serializing the suite is not a fix.

## VER-07 — P2 · Reproduced · Default development worker path cannot execute

**Evidence:** CLI fallback `src/bin/publicator.rs:425–435` and GUI fallback
`src/bin/publicator-ui.rs:39–46` select `python/publicator_da/worker.py`; publishing executes the path
(`src/publishing.rs:117–124,710–719`). It is mode 0644 without a shebang and uses relative imports.
Nix instead injects an executable module launcher (`flake.nix:39–45,59–62`).

**Observed:** Direct Cargo check-steps compiled then exited 1 with
`I/O error: Permission denied (os error 13)`. The worker did not run; this is not a numbered-step
mismatch. Nix-wrapped applications are not proven broken by that result.

**Existing coverage:** `tests/packaging.rs:6–12,29–35` injects a shell launcher and PYTHONPATH;
CLI tests (`tests/cli.rs:302–348,406–459`) likewise override workers, masking the default path.

**Regression / close when:** Bound a no-override CLI check-steps invocation from the checkout,
explicitly removing worker/skill overrides. Require real static-skill comparison without browser
or data/profile acquisition; cover the shared GUI default. Keep external-CWD packaged smoke separate.
Merely chmodding the module or substituting a fake in the test conceals its entrypoint/import issue.

## VER-C01 — P3 · Gap · No measured coverage or complete behavior-to-gate accounting

**Evidence:** The realized shell lacked cargo-llvm-cov, tarpaulin, LLVM coverage tools, grcov and
Python coverage/pytest-cov. Complexity/duplication gates in `flake.nix` measure syntax, not branch
coverage. This audit produced behavioral inspection and test counts only.

**Close when:** Map each retained safety transition to a runnable check: admission, dispatch,
before/after commitment, persistence uncertainty, refresh/close, restart and reconciliation. Record
which checks are automated versus supervised, and measure branch coverage if tooling is authorized.
Keep numbers diagnostic, not a blanket threshold that hides false assertions. Explicitly retain any
unmeasured/excluded paths; no “fully tested” claim based on counts or all-targets compilation.
