# Application and AI metadata

Contract: `docs/features/ai-metadata.md`. Use injected loopback endpoints, dummy keys and synthetic
images; no real model, installation or provider call. Artwork identity is owned by ASSET-03.

## META-01 — P1 · Defect · Single-generation completion overwrites newer edits

**Evidence:** `src/application.rs:213–230` routes success/failure to unconditional field replacement
in `src/workflow.rs:159–178`. State is reread but not compared to the admitted draft. The reservation
at `src/application.rs:203–211` only protects that Application instance. Confidence: high.

**Trigger/impact:** Another Store/Application writer changes a SCHEDULED row during a provider
request. Success replaces the newer pair and approves it; failure replaces it with TODO markers.
The newer-manual-edit guarantee does not hold across this path.

**Existing coverage:** `tests/application_ai.rs:201` tests same-instance exclusion;
`tests/workflow_metadata.rs:317–386` tests batch marker merging. `tests/ui_describe.rs:965–990`
intentionally allows generation to replace a draft saved **before** admission; preserve that case.

**Regression / close when:** Pause `generate_one` after HTTP request capture, save newer fields
through another writer, then release either a valid pair or malformed completion plus empty fallback
catalogue. Newer fields survive both outcomes without implicit approval; the lease releases.
Condition completion on the admitted fields. Do not blindly use marker-only merging, which would
change intentional replacement of pre-existing non-marker drafts.

## META-02 — P2 · Reproduced · Malformed-response test passes on a fixture failure

**Evidence:** `tests/local_metadata.rs:130–159` scripts tags then chat, but direct `local_generate`
sends chat (`src/metadata/local.rs:150–179`). The fixture panics at `tests/common/http.rs:56`;
Drop ignores join failure (`96–100`), and generic `is_err()` accepts the transport error.

**Observed:** The focused test exited 0 with one pass while two fixture threads panicked:
actual `/api/chat`, expected `/api/tags`. Intended malformed/empty replies never arrived.
Exact command and witness are recorded in [validation](validation.md).

**Existing coverage:** Successful chat (`tests/local_metadata.rs:87`) and shared parsers
(`tests/metadata.rs`) provide partial protection, not proof of this provider-specific test.

**Regression / close when:** Script only chat, assert its request body/path, account for response
delivery, require the specific parse/validation failure, and finish the fixture explicitly.
Separately inject an unexpected request and require a visible test failure. Cleanup must remain
bounded and avoid double panic during unwinding. Coordinate fixture shutdown with VER-05.

## META-03 — P2 · Defect · Image preparation escapes the generation timeout

**Evidence:** `src/application.rs:263–271` prepares synchronously before the provider deadline;
`src/images.rs:266–270,309–313` waits on blocking unbounded image-tool subprocesses. OpenRouter's
budget begins at `src/metadata/openrouter.rs:173`. Confidence: high.

**Trigger/impact:** A hung identify/magick retains generation reservations, prevents publishing or
same-UUID edits and occupies an async runtime thread. An outer Tokio timeout cannot interrupt that
blocking work. Local preflight and generation also start separate budgets (`src/application.rs:238,271`).

**Existing coverage:** `tests/openrouter.rs:283` bounds HTTP bodies; successful image preparation is
covered in `tests/images.rs:215,324`. Neither tests stalled preparation or the composed budget.

**Regression / close when:** In a watchdog-owned child, use a child-only PATH with a synthetic image
tool that signals entry and blocks. Require bounded failure, zero provider calls, released leases,
reaped children and no lingering descendants. Define whether timeout covers the whole operation or
provider work alone, and bound preparation regardless. Do not mutate global test-process PATH.

## META-C01 — P2 · Gap · Real application persistence failures are not exercised

**Evidence:** `src/application.rs:225,229,346–358` separates success-save, failure-marker save and
uncertain batch commits. Helper tests at `372–407` construct errors rather than driving commits.
`tests/application_ai.rs:236,293,539` covers normal ordering/failure persistence.

**Regression / close when:** Reuse the Store's private pre/post-rename hook
(`src/store.rs:114–157`) in current-thread application tests after a paused response is captured:
- Single success + pre-rename failure: original bytes survive, error surfaces, lease releases.
- Provider failure + marker-save failure: persistence failure surfaces; no false marker-save claim.
- Batch success + post-rename uncertainty: new bytes remain, uncertainty escapes, no next request,
  and all reservations release.
- Batch pre-rename failure: assert the intended ordinary-failure continuation and unchanged row.
Use real small PNGs/hashes and two marked rows where needed; no new production fault mode.

## META-C02 — P2 · Gap · Composed provider budgets and local error replies need proof

**Evidence:** `src/metadata/local.rs:94–105,190–222` and
`src/metadata/openrouter.rs:57–95,189–225` implement deadlines. Existing tests cover one OpenRouter
body timeout, schema retry/fallback order and HTTP-500 pull failure, not the following sequences.
This is not a claim that the currently timed network paths exceed their budgets.

**Regression / close when:** Gate requests instead of relying solely on sleeps:
- OpenRouter selected failure → catalogue → multiple failures uses one budget; no post-expiry call.
- Local fit → installed tags → chat headers with withheld body ends boundedly after request capture.
- Consented pull returning HTTP-200 `{"status":"failed"}` never advances to chat.
- Tags returning HTTP-200 without `models` fail as protocol errors and leave publication bytes intact.
- Fake llmfit signals entry, blocks, then is terminated/reaped by the deadline.
Preserve explicit installation consent and loopback-only dependencies.

## META-C03 — P3 · Gap · Ordered admission, cancellation and preflight need direct tests

**Evidence:** Ordered UUID comparison/reservations exist at `src/application.rs:167,175,288–299`.
UI tests cover changed membership (`tests/ui_describe.rs:420,462,503`), but not every application
boundary. Single Local generation preflights before Store validation (`src/application.rs:80–84`),
unlike batch admission; the acceptable side-effect order needs an explicit decision.

**Regression / close when:** Reorder the same eligible UUIDs and reject before llmfit/provider work.
Pause batch preflight: every reserved row rejects edits while unrelated rows remain editable.
Abort a paused generation task and prove a subsequent operation acquires the released reservation.
Probe invalid single-generation source data and document/assert whether any preflight is permitted.
Use two rows, gated endpoints and temporary executables with bounded cleanup.

## META-C04 — P2 · Gap · Wire-level media and catalogue filtering need composition tests

**Evidence:** Provider filtering validates every advertised price component and image capability
(`src/metadata/openrouter.rs:127–225,241–270`), retaining the attempted ID rather than a response
alias. `tests/openrouter.rs:50,69,113,149,228,261` covers individual choices; image tests cover
encoding separately (`tests/metadata.rs:80`, `tests/images.rs:215,324,354`).

**Regression / close when:** Drive the application with a real hashed image, capture its request,
decode the transmitted media and check MIME, dimensions and representative pixels. Traverse a
mixed catalogue containing free, paid, nonvision, duplicate and failing candidates; assert exact
attempt order, no paid/nonvision call, no duplicate retry and correct stored model provenance.
Combine with the shared deadline witness, not another fake all-success provider.
AI acceptance of WebP is distinct from DeviantArt's prohibition on uploading WebP.
