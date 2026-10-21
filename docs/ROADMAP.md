# Roadmap

## Rust / Iced migration — IN PROGRESS

**Resumed by owner, 2026-10-21:** Continue from Task20 fix2, complete Tasks21–23, run the required validation/reviews, then parent fast-forwards `master` preserving all commits and pushes normally. After successful integration/push, remove this worktree only when clean. This supersedes the historical pause and worktree-preservation instructions below; no real-data/provider/profile/publishing permissions are added.

**Checkpoint:** 2026-10-21T07:01:52Z. The owner requested: “Pause execution, move plan in docs/ROADMAP for now. We'll resume it later.”

The previous workflow and its active child were stopped. The owner has now explicitly resumed execution; use a fresh continuation at the checkpoint below, not the obsolete workflow script. This file remains the durable plan/checkpoint and the explicit exception to the earlier scratch-only plan rule. Logs, review reports and temporary fixtures still do not belong in Git.

### Current state

- **Accepted:** Tasks **1–19**, with the scoped carry-forward obligations below. Rust core, provider adapters, durable publisher orchestration, CLI, native shell/drafts and Pick/Schedule have passed their task reviews.
- **In progress / NOT accepted:** **Task 20, Describe**, second fix round. Local fit/status and explicit consent are implemented; the remaining review finding is that local batch confirmation must remain bound to the exact UUID set through asynchronous Application admission.
- **Not started:** Tasks **21–23**: native Publish/Calendar/shared prompt views; complete native/browser and packaging acceptance; final Nix cutover, legacy Python/JS/Claude removal and whole-branch review.
- The existing Python apps and old Nix aliases remain intact during staged migration. Rust `rust-*` apps are opt-in; **the full application has not been cut over**. The native shell has an empty-window Xvfb smoke pass, not full workflow acceptance.

### Git and preserved in-progress work

| Item | Checkpoint |
|---|---|
| Feature branch | `feat/rust-iced` |
| Execution worktree | `/tmp/publicator-rust-migration/worktree` |
| Last accepted task HEAD (Task 19) | `1dda85e203dd14b32ec2e441298cbbd9903f386e` |
| Latest implementation commit / Task20 fix2 BASE | `288a46b587ba62a22d2c7d4417e39cb0f57cd1a9` |
| Original Task20 BASE | `1dda85e203dd14b32ec2e441298cbbd9903f386e` |
| Original checkout | `/home/theta/.vault/repos/publicator.py`, `master` |
| Original master, unchanged | `7b5c4d6e4c423e4e23c8897e5fcc13ae5237b41c` |
| Durable WIP backup (Git stash object) | `e691225c2a757af58ffe2437fee66797d1c4961f` |

A documentation-only pause checkpoint may follow the latest implementation commit above. It is **not** Task20 acceptance.

The partial second fix remains **uncommitted and unreviewed** in:

- `src/application.rs`
- `src/ui/mod.rs`
- `tests/ui_describe.rs`

At pause these were 158 added / 26 removed lines. They introduce expected-batch checking in the shared generation path and propagate batch selection through consent. Their completeness still needs review, including the delayed-execution/changed-membership race. No source changes were discarded or folded into a premature “done” commit.

The stash object was created/stored **without removing the working changes**; its patch was compared with the live patch. Do not apply it on top of already-present changes. If the temporary worktree disappears, the committed branch and stash object remain in the original repository; inspect Git/worktree state first, then recover only missing work from that exact object. A second patch copy is at `/tmp/publicator-rust-migration/sdd/pause-checkpoint/task-20-fix-2-uncommitted.patch`, but `/tmp` is not a durability guarantee.

### Checks and known limits

- Last committed Task20 fix1: 18 Describe tests plus desktop/core/format checks passed, but independent re-review rejected the **batch target identity** gap. Latest review: `03aab2b4-791a-4d8a-9c78-54ed43e0f237`.
- Captured partial fix2 logs report **19 Describe** and **8 Application AI** tests passing; desktop/core runs also reported passes. These are interrupted-worker evidence, **not** a completed fix delivery, new commit or independent approval. Recheck the exact resumed state.
- Shared lazy preview preparation and initial/later-range request tests are implemented. Task19's original eager-preview minor was explicitly assigned to Task20 and cannot disappear merely because a narrow re-review omitted it. Close the carried requirement at Task20's final gate and prove actual visible-card scrolling at Task22.
- A normal parallel desktop run previously failed `shutdown_terminates_worker_descendants`; isolated/serial passes do not establish cause or a fix. Task21 owns investigation and normal parallel regression proof. Do not hide it with `--test-threads=1`, sleeps, skips or removed assertions.
- Full real native-window/browser acceptance remains **mandatory at Task22**, then again after final wrapper switching. Reducer tests, manually injected messages, cache counts or an empty-window screenshot are not substitutes.
- No live DeviantArt submission/selector, original-profile or real provider validation is claimed. Only fake workers, loopback providers and disposable fixtures are authorized for implementation checks.

### Resume sequence — only after a new owner instruction

1. Read this checkpoint, actual `git status`, the partial patch and latest Task20 report/review. Keep the three source changes; do not reset, blindly stash-apply or redo accepted Tasks1–19. Check what survived `/tmp` cleanup.
2. Use the governed serial implementer → fresh read-only reviewer workflow with `openai-codex/gpt-6-luna:high` (`:max` is available for escalation). Preserve one source writer. The old workflow is **stopped**, not a resumable live continuation: do not blindly run or resume its script, which is hardcoded to an obsolete Task6 checkpoint.
3. Continue **Task20 fix2 from reviewed BASE `288a46b...`**, retaining all follow-up commits in its review range. Capture/freeze the batch UUIDs or reject changed membership at actual Application admission. A UI-only comparison followed by unscoped `generate_marked` can still race. Preserve marker-only merge, reverse persisted OpenRouter order, forward local order, first-answer-model preference, dirty flush, target-only busy-through-persistence and unrelated edits. Do not replace batch semantics with a loop of `generate_one`.
4. Verify local Good/installed and Good/missing/confirmation paths; refusal/cancel/bad/unknown/unavailable state; stale source/model/target/batch; saved rows and fake call logs. Finish/report/commit the fix and obtain independent scoped approval before Task21.
5. Execute Tasks21–23 and their carried obligations below. Resolve review findings; perform simplification then ponytail review and apply valid changes. Require full final whole-branch review, a fix wave if needed, and fresh final verification.
6. Only when **everything is implemented and checked**, the owner has already authorized preserving all feature commits by fast-forwarding `master` and pushing the intended remote. Parent performs this, with clean worktrees/ancestry/remote checks; no force push, protection bypass or early integration. Keep the feature branch; the latest owner request authorizes removing the worktree when clean after final integration/push. No merge or push happened at the pause.

### Open obligations that must not be lost

- **Task21:** Preserve typed `Error::DurabilityUncertain` when worker stderr is present; currently diagnostic decoration can turn it into `Error::Protocol`. Retain receipts, stop before the next send, reread/report; no rollback or implicit retry. Add actual production-boundary evidence.
- **Task21:** Diagnose the captured concurrent descendant-cleanup failure with bounded process-state/signal evidence; distinguish a real leak from asynchronous termination assertion timing and fix the proper boundary. Show normal parallel tests passing.
- **Task21:** Native selection/remaining/retry derives from persisted **registered pending cells**. A successful DA cell must not resend or inflate executable remaining merely because its publication has an unregistered historical pending apparition. Calendar still shows every apparition. Core production evidence was closed in Task16; native behavior is still open.
- **Task22:** Real noVNC-canvas keyboard/paste, focus, resize and scrolling through all five native tabs, empty/history/mixed fixtures, actual later-visible image loading, multiline descriptions with exact internal newlines/Unicode/literal markup, draft races/failure, consent refusal, tier/gallery multi-selection, fake publish retry/unknown reconciliation, refresh/reopen and dirty close. Fix the smallest real widget seam if these reveal issues.
- **Task22:** Packaged schema/skill/worker and runtime libraries must work from an external CWD without the source tree. Use existing `PUBLICATOR_WORKER` / `PUBLICATOR_SKILL` consistently; the old draft's `PUBLICATOR_DA_*` spelling was corrected. Software and normal-GPU compile paths remain required. Test tools/bridge/fake providers stay out of shipped closures.
- **Task23:** Exact old-test → retained/ported/HTTP-only disposition before deleting the old application; remove Claude/Python llm dependencies and obsolete app code only after prior gates. Update maintained feature docs/AGENTS/ADR. Keep the static publishing skill and Python browser boundary. Document the approved CLI-only `PUBLICATOR_OLLAMA_URL` dependency override without creating a production fake mode.
- **Final minor checks:** Add direct preservation coverage for an automatic PICKED row with unknown `scheduleProfile` and an existing timestamp (not only a manual Custom row). Consider a small shared pure delivery transition for journal/workflow duplication without nesting Store mutations or weakening atomicity.

### Evidence and operating rules

- Scratch root: `/tmp/publicator-rust-migration/`; task reports, results, diffs, logs, parent rulings and restored ledger: `sdd/`. These paths are useful provenance, **not required to survive a reboot**; the essential plan/checkpoint is copied into this file.
- Stopped workflow: `c2243b3a-ab30-4294-8d9f-7bdb3f9c00f1`; stopped fix2 child: `4b34fdc3-d713-4501-b1d9-e780d3588294`; mission: `f559104b-45c4-4eef-b769-cb7f235ed315`.
- `execution-from-task6.js`, `execution-continuation.js` and older workflow scripts are historical, **not resume commands**. Read durable mission/task evidence and construct a checkpointed continuation only when resumed by the owner.
- Shared `progress.md` is **parent-write-only**. A child accidentally replaced it; exact recorded-mutation recovery restored it and verified intact prefix/tail witnesses. Recovery provenance is in `sdd/ledger-recovery/`. Child agents put proposed progress in their own report/native result; never overwrite a large report from a truncated read.
- No real Art changes/migration, original Firefox-profile changes, model installation, provider spending, real publishing, tags/releases or force push. Git push authorization does not grant those effects.
- The implementation procedures below preserve the approved task detail. Their unchecked substeps are reference checklists, **not a contradiction of the accepted-state checkpoint above**. Completed code and `Cargo.lock` supersede draft API/dependency suggestions (e.g. actual process-group code uses libc). Do not rerun completed work just to tick historical boxes.

## Task status

| Task | Work | State at pause |
|---|---|---|
| 1 | Boot a working Rust validator with lossless configuration | Accepted |
| 2 | Atomic locked Store with fresh reads | Accepted |
| 3 | Candidate discovery, authorization and shared thumbnails | Accepted |
| 4 | Timezone scheduling and capacity | Accepted |
| 5 | Pick and date approval operations | Accepted |
| 6 | Metadata drafts, approval and generated merges | Accepted |
| 7 | Platform selection, options and completion | Accepted |
| 8 | Pure migration and calendar data projection | Accepted |
| 9 | Content-addressed prompt archive, search and audit | Accepted |
| 10 | Shared metadata schema/parser and HTTP test seam | Accepted |
| 11 | OpenRouter schema recovery and bounded free-model fallback | Accepted |
| 12 | Local Ollama, llmfit and installation consent | Accepted |
| 13 | Application admission, single generation and exact-marker batches | Accepted |
| 14 | Extract Python browser worker with explicit v1 protocol | Accepted |
| 15 | Durable publishing intents, receipts and reconciliation | Accepted |
| 16 | Rust publisher preflight and sequential worker lifecycle | Accepted |
| 17 | Port CLI operations and preserve exit/consent semantics | Accepted |
| 18 | Native application shell and revision-safe drafts | Accepted |
| 19 | Native Pick and Schedule stories | Accepted scoped; preview follow-through in Task20, native proof in Task22 |
| 20 | Native Describe, local consent and AI batches | NOT accepted; fix2 partial/uncommitted |
| 21 | Native Publish, recovery, Calendar and shared prompt panel | Not started |
| 22 | Real native-window acceptance and packaged runtime assets | Not started |
| 23 | Switch Nix apps, delete old application and remove Claude toolchain | Not started |

## Approved design reference

The design is included here so resumption does not depend on a temporary spec file. The pause/checkpoint and explicit ownership rulings above override historical “execute now” language.

<details>
<summary>Approved design (full reference)</summary>

Date: 2026-10-20
Source baseline: `/home/theta/.vault/repos/publicator.py`, `master`, `7b5c4d6`.
Status: reviewed and auto-approved under delegated owner authority. The owner subsequently instructed: "Execute the plan subagent-driven"; execution is now authorized, using luna-6 agents.

### 1. Goal and authority

Replace Publicator's maintained Python application and JavaScript UI with Rust application logic, Rust CLIs, and a native Iced UI. Keep DeviantArt's Playwright browser automation in Python. AI generation supports only OpenRouter and local Ollama; remove Claude Code and the Python `llm` toolchain entirely.

The initial delivery was design/plan only. That stop boundary was superseded by the owner's subsequent explicit instruction to execute the plan subagent-driven. Implement and commit logical units in an isolated worktree. The owner's later instruction conditionally authorizes the parent, after all implementation/checks and final independent review/fixes pass, to fast-forward master to the reviewed feature HEAD (preserving every commit) and push the intended remote without force; see `/tmp/publicator-rust-migration/sdd/final-integration-authorization.md`. No early integration/push, real-data migration, original-profile modification, model installation, remote provider spending, tag/release creation or real publishing is authorized. Specs, plans, checklists and review reports belong in `/tmp/publicator-rust-migration/` or managed scratch artifacts, never `docs/superpowers/` or any other tracked report directory.

Architecture, spec and plan are approved under delegated review authority. The original checkout remains protected during implementation; only the parent may update it for final authorized master integration after all gates. Implementation uses fresh luna-6 subagents and task-scoped independent review.

### 2. Chosen approach and alternatives

Use staged replacement on one isolated execution branch: core/data operations; providers and publishing boundary; native UI and CLI integration; final Nix cutover and removal. Until the cutover gate, the old Python app remains runnable. Never run old and new publishers concurrently. There is no permanent Python business-logic bridge or dual UI after cutover.

A one-shot replacement has less temporary coexistence but makes invariant regressions and packaging failures harder to isolate. A native wrapper around the existing Python core would speed an initial screen but would not meet the Rust ownership requirement. Neither alternative is selected. Leptos, Tauri, a webview, an HTTP application server, a new database, a broker and a plugin framework are unnecessary.

### 3. Minimal final structure

Use one Cargo package, not a multi-crate framework:

- A framework-independent Rust library: `src/lib.rs` and modules corresponding to existing config, entries/model, Store, workflow, scheduling, images, prompts, migration, calendar, metadata and publishing responsibilities.
- `src/bin/publicator.rs`: CLI subcommands for existing non-GUI apps.
- `src/bin/publicator-ui.rs` and `src/ui/`: native Iced presentation and message/update orchestration. Optional Cargo `desktop` feature keeps the CLI/core test build independent of Iced.
- `assets/publicationsSchema.json`: move the existing schema unchanged and embed it with `include_str!`; never load a schema from the data directory.
- `python/publicator_da/`: browser steps/session preparation, a stdin/stdout worker, and the existing Playwright DOM-probing daemon. No Python workflow, TOML, publication-schema, scheduling-allocation or database code remains at cutover.
- Keep `.claude/skills/publish-deviantart/SKILL.md` as the static manual-publishing/STEPS contract; its directory name is not a runtime Claude dependency. Update its code-path instructions when the worker moves. Do not remove a browser recovery contract merely to remove the CLI toolchain.

The implementation plan must settle exact filenames and public interfaces before its task list. Split only by actual responsibilities; do not create traits/factories for hypothetical providers or platforms.

### 4. Stack and packaging

- Iced **0.14.0**, enabled only for desktop; Tokio for async HTTP/subprocess work and blocking-task dispatch.
- Serde/serde_json, TOML parsing, reqwest with Rustls, chrono/chrono-tz, UUID generation, a JSON Schema Draft 2020-12 validator, filesystem locking and same-directory temporary replacement. Pin released compatible dependencies in Cargo.lock; do not track framework Git branches.
- Reuse installed Git for prompt history, ImageMagick for thumbnail/upload conversion, and llmfit as the only local model host-fit evaluator. Do not reimplement these tools.
- Use a Rust digest implementation supporting runtime-selected digest names. Preserve the currently configured and tested digest algorithms and Python-style named regex captures. Unsupported regex syntax/algorithms must produce a precise configuration error, not a fallback grammar, a different digest, or an empty archive. The plan must name the compatibility strategy rather than silently assume Rust regex equals Python re.
- Nix remains the build/distribution authority, initially x86_64-linux as today. Package Iced windowing/graphics dependencies for Linux and its software rendering path; no WebKitGTK or Leptos toolchain.
- Remove the `claude-code` flake input and its lock nodes, the vendored `llm-claude-cli` derivation, `ps.llm`, `ps.llm-openrouter`, Python jsonschema runtime, Claude environment/session access, Claude defaults/options/buttons/tests and maintained AI documentation. Keep Python + Playwright only for browser tasks, and pytest only as a development test dependency for them.
- Preserve Nix app names: `ui`, `da-publish`, `generate-meta`, `vision-models`, `validate`, `echo-first`, `prompt-audit`, `check-steps`, `login`, `pw-daemon`. Rust owns argument parsing/business orchestration for all but the deliberately Python Playwright prober; login may remain the existing minimal shell wrapper around real Firefox. Remove obsolete HTTP `--port` and Claude `--ai-model` UI flags with explicit unsupported-option errors. Existing `--data-dir`, `--json` where supported, DA `--uuid`/`--all`/`--headless` and AI timeout/consent semantics remain.

### 5. Data boundary and validation

`publications.json` remains authoritative; preserve its array order, unknown fields, UUID strings, historical platforms, legacy/new hash keys, historical options and intentionally blank new drafts. JSON whitespace may change; semantic fields must not disappear through typed deserialization. A raw JSON document with validated views or lossless flattened typed records is acceptable; a lossy new schema is not.

Paths resolve from `--data-dir` or CWD, never from the code checkout. `--json` may point outside that directory; always carry the effective data-dir config into validation and result persistence. Config/TOML is read-only. Missing config keeps current defaults; missing publication file starts empty; malformed JSON is an error, never an empty queue. Loading legacy data does not auto-migrate it.

Every write takes a short in-process mutex plus an advisory file lock compatible with the existing persistent `<json-path>.lock`, rereads fresh data, applies one operation, validates the complete result, then atomically replaces through a same-directory temporary file. Keep the lock file, clean failed temp files, and test that validation or pre-rename replacement failure preserves the original bytes. Atomic rename is the visibility commit boundary: a subsequent durability/cleanup failure must be a distinct typed committed-but-uncertain outcome, never an ordinary error promising no write occurred or a speculative rollback. Retain the underlying cause/path, report uncertainty and reread before retry; publishing preserves its durable receipt and stops. AI, browser work, full image hashing and prompt archive indexing must not hold the publication mutation lock.

Preserve JSON Schema 2020-12 validation plus DeviantArt tier/gallery allow-lists, HTTP(S) URL-with-host checks, obsolete-field rejection, finite nonnegative prices (including explicit zero), and the DA title limit of **50 Unicode scalar characters**, not UTF-8 bytes. Do not infer actual URLs or titles from slugs.

The pure legacy migration remains a Rust library operation, not an automatic startup rewrite. It requires supplied verified title recoveries, fails conflicts without changing its input, preserves unrelated fields, and is idempotent. The rewrite does not reopen or edit the real Art migration.

### 6. Workflow and scheduling

Publication states record completed phases: `PICKED`, `SCHEDULED`, `DESCRIBED`, `APPEARED`. Apparitions separately retain `unpublished` / `published_or_scheduled`. Selection alone is never submission. A successful delivery locks shared content and successful cells; an APPEARED publication can gain a new pending registered platform without resubmitting old successes. All historical apparitions remain visible and retain completion significance, but only registered executable platforms participate in publish preflight/dispatch/counts; initially only DeviantArt is executable. Historical-only pending data is a no-op.

Pick records file identity/hash, deduplicates, and creates no platform or metadata. Schedule editing/repacking and approval are explicit persisted operations. Manual metadata drafts need approval; a valid generated title/description pair may advance SCHEDULED to DESCRIBED. The marker is exactly `[[PUBLICATOR:TODO_GENERATE]]`; blank fields or any occurrence of that marker block approval/publication. Backwards actions preserve drafts/selections and are forbidden after delivery success.

Scheduling is Rust-core-only and uses the configured IANA zone, default `Europe/Paris`. Generate **52** upcoming weekly slots per profile from an injected clock, maintaining local wall-clock hours across DST. Count a publication once, not once per apparition. Repack only automatic PICKED dates in persisted order; manual PICKED pins and approved/historical dates reserve capacity and remain fixed. Explicit dates may exceed capacity and stay chosen; exhausted automatic horizons leave no suggestion and block approval. Off-cadence or removed-profile dates stay Custom until an explicit preset/suggested-date action. Profile default tiers/prices apply only on first DA selection; never overwrite existing options or apply them to an unmatched Custom date.

Explicit DST conversion behavior must be tested: ambiguous local times select the earlier occurrence (Python fold=0 behavior); a nonexistent spring-forward wall time uses the pre-transition offset, normalizing forward through the gap, matching current zoneinfo conversion. Offset-bearing custom input continues to be interpreted as a wall-clock in the configured schedule zone, not as an instruction to switch zones. Never use host/browser locale, `date` output or UTC-hour recurrence arithmetic.

### 7. Images, prompts and calendar

Preserve configured-path Search (limits **0, 25, 50, 75, 100**, default 25/all configured paths), content-hash dedup including preview-file hashes, runtime path subsets without TOML edits, exact-prompt text and lineage search, and the **200**-result pre-hash cap for each filtered group. Candidate deletion is only for currently authorized, unreferenced artwork; recheck fresh publication references so changed bytes cannot make a referenced file deletable. No arbitrary filesystem-path action becomes reachable through the native UI.

Reuse lazy `<data-dir>/.thumbs/<sha512[:32]><lowercase-original-ext>` across sessions and UI/calendar/AI. Do not hash every history image on startup or decode all full-resolution originals on the UI thread. Preserve the current conversion/copy-original fallback with an explicit diagnostic; test real resizing separately so fake test bytes cannot masquerade as proof. Unsupported native previews (including legacy video candidates) show a named placeholder, not a crash or an invented video player. AI input must be an actual image MIME; a failed/unsupported media request remains retryable. DA uploads must be non-WebP: reuse a proven matching non-WebP image or convert into per-attempt runtime scratch, never a colliding shared `/tmp/<stem>.png` path.

Prompt filename grammar and `version_hash` remain data-owned. Index every Git blob including unreachable objects, digest raw content rather than Git OIDs, associate historical paths/times, and refresh on HEAD changes with the current **2-second** probe throttle. Byte-length framing must survive multiline/binary blobs and malformed headers without corrupting the index. Use distinct Rust `Exact`, `Nearest`, `Unknown` variants; Nearest has no authoritative prompt-text field. Candidate ranking orders all alternatives and never silently chooses one; test historical/superseded exact resolution and ambiguous lineages. A missing/broken optional archive degrades to Unknown; a malformed configured grammar fails at configuration load.

Calendar is read-only, one event per apparition across every historical platform, using shared publication title/date and actual optional apparition URL, bucketed in the configured zone. Include the current month; missing art remains a text event. Candidate and calendar prompt panels reuse the same native component; schedule/description cards do not gain prompt/search/delete controls. Calendar navigation to an internal publication reveals its correct phase.

### 8. OpenRouter and local metadata

Both providers use one prompt, one title/description response schema and shared image preparation. Keep the explicit model catalogue with independent `ollama_name`, `openrouter_name`, `llmfit_name`; do not derive one provider's IDs from another's. Keep existing published model IDs until normal catalogue changes are justified. Preserve the `openrouter/` user-facing prefix where existing CLI options accept it; strip it only for OpenRouter wire IDs.

#### OpenRouter

Rust sends HTTPS requests to `https://openrouter.ai/api/v1/chat/completions`, reading `$OPENROUTER_KEY` only at runtime. Vision messages carry an image data URL with the actual MIME. Do not log credentials or image payloads. Support schema-capable models with `response_format: {type: json_schema, json_schema: {name, strict, schema}}`; unsupported schema use/rejection falls back once to a schema-in-prompt request, memoized by model. Parse the outermost JSON object when prompt-shaped replies have fences/prose; require string, nonblank, marker-free title and description and enforce the title limit before approval.

Without llm-openrouter, the authoritative fallback catalogue changes deliberately to `GET https://openrouter.ai/api/v1/models`: image input modality plus zero prompt/completion price is required for fallback candidates, and any advertised nonzero image/request/other charge excludes a candidate from free fallback. Preserve dynamic IDs, other vendors before siblings of the failed selected vendor, bounded overall attempt budget (default **300 seconds**, including catalogue/schema retries), last-error preservation and logging the answering model. Query the catalogue lazily after a selected-model failure. Use the returned supported-parameter metadata when available, while still recovering a real unsupported-schema response; no schema-capability model hunt. A catalogue error must not replace the original generation error. Any failed candidate, not just 429, can advance within OpenRouter; never fall back to local or another hosted source. No new paid fallback is allowed. A small in-memory catalogue cache is sufficient; the plugin's former disk cache is not a data contract to recreate.

#### Local Ollama

Use `http://localhost:11434`: `GET /api/tags`, `POST /api/pull` with `stream:false`, and `POST /api/chat` with base64 `images`, shared schema in `format`, `stream:false`. Validate status/body errors and missing/invalid response content. Do not start/manage the Ollama service. Continue to call `llmfit info <llmfit_name> --json`; only `Good` is eligible, failed/unassessable fits are unavailable with an explanation. UI installation needs explicit consent. CLI asks `[Y/n]` (Enter accepts); `-y` accepts; a non-TTY without `-y` aborts without changing publications.

#### Admission, merges and batches

Single generation accepts only a fresh SCHEDULED UUID, a canonical model and a real stored image. Reserve it in the shared application-session coordinator until result/markers are saved; disable its draft edits/approval/phase changes and reject duplicate AI work while unrelated records remain usable. Pre-admission validation failures change no fields. After accepted single-generation failure, save the exact marker in both fields, leave SCHEDULED, surface the original error; persistence failure is a separate unsaved error.

Batch admission selects only SCHEDULED records with an exact top-level title or description marker, and reserves the still-eligible set together. OpenRouter uses reverse persisted order for the entire eligible batch (last, penultimate, ..., first), preferring a successful answering model for later records. Local preserves persisted order. Validate the incoming generated pair, then save each result independently, replacing only fields still equal to the exact marker, never a newer manual edit. Persist valid marked-field replacements even if a preserved manual counterpart remains blank/incomplete; keep SCHEDULED in that case. Advance only when at least one generated field was applied and both saved fields form a valid pair. A stale result replacing zero markers never implicitly approves an entirely manual draft. Continue after record failures. A batch failure retains existing markers and does not overwrite an unrelated manually edited field. UI and CLI call the same orchestration/operations; do not duplicate HTTP-era policy in widgets.

### 9. Python publishing protocol and recovery

#### Ownership and request content

Rust resolves fresh approved records, validates the whole selected batch/options/tags/art before browser startup, prepares non-WebP uploads and timezone-derived schedule components, and records all delivery outcomes. Python never receives a publications.json path, never loads publicator.toml, and never reads/writes the publication database. It retains the existing selectors, ordered STEPS, mature/AI flags, optional premium/tiers/galleries, genuine-Firefox login-copy/cookie/session-verification behavior, and browser teardown.

Use version **1** newline-delimited JSON over a child process's stdin/stdout, no Python embedding or HTTP service. One process keeps one authenticated browser session for the batch; Rust sends **one artwork at a time**, waiting for its result to be persisted before sending the next. Logs are stderr only; stdout is exclusively bounded protocol records. Read stderr concurrently to avoid pipe deadlock. Validate versions, event types, attempt IDs, UUIDs, ordering and one terminal event per sent job; malformed/duplicate/mismatched records are errors, not successes.

Session-open command includes login/work profile paths, headless=false by default, and timezone. A submit command includes attempt ID, UUID, absolute prepared image path, title, description, tags array, local year/month/day/hour/minute, timezone/epoch, price if present, plural tiers and galleries. Validate at the Python boundary too, before touching browser state. Jobs cannot request another platform or arbitrary code execution. Existing DA hour-only picker cannot silently discard minutes: fail preflight for an unrepresentable time rather than schedule a different instant. The DOM prober remains an explicit local debugging tool, never the worker protocol.

#### Events and sequential durability

Session events: ready or fatal; per-artwork terminal events: scheduled, failed, unknown. Include protocol version, attempt ID and UUID on job events. A scheduled result means the existing ordered submission flow completed successfully; it is not proof that a future publication is already public. Preserve current flow/confirmation semantics during the port; do not invent a URL. Exceptions after the final scheduling click may have taken effect and must be unknown, not safely failed. Instrument the final-effect boundary so pre-submit failures can be distinguished in tests. Stop the batch after its first failed/unknown result, leaving unsent records unattempted.

Acquire a separate nonblocking per-data-dir publisher file lock for the batch so Rust UI and CLI cannot start competing browser sessions. Explicit human reconciliation takes the same lock and refuses Busy while another process's batch is active; it cannot release or rewrite a still-active intent. This is NOT the publication mutation lock. Immediately before each send, reread/validate that record and write a durable intent (unique attempt ID, resolved target database path, UUID/platform and approved-content/options fingerprint) beneath `<data-dir>/.publicator/publish/`. Persist the terminal worker receipt before applying its short Store mutation, and send the next job only after both succeed. Publication success uses the same effective config. A remote-success/local-save failure must retain the receipt and stop, not leave a silently retryable pending target.

On restart, a confirmed scheduled receipt can be replayed into Store idempotently only if its identity/fingerprint still matches; the current-row approval/fingerprint check and delivery transition occur inside the same short Store mutation, not a separate check-then-save. Full image hashing stays outside the lock, with relevant fresh row fields compared again inside it. Replay never launches a browser. A pending/unknown intent or changed approved content requires explicit human reconciliation and blocks automatic retry of that target. Other targets remain readable. Both GUI and CLI expose reconciliation: mark a verified remote success (with optional actual URL), or confirm not-submitted and permit retry. Do not claim exactly-once remote delivery; the durable journal prevents blind duplicate retries after a crash, not every possible external edit or human submission.

Cancellation is stop-after-current. Closing during publishing offers wait/cancel-after-current; explicit force termination warns that the current result may be unknown. Worker timeout/crash/EOF never marks unsent artwork successful or safely retries the sent one. Use a bounded per-artwork timeout and graceful close followed by bounded process-group cleanup on Linux; keep login source profile read-only. Runtime journal/lock/conversion paths belong under the data directory and must be documented for its gitignore. No secrets/cookies are placed in receipts.

The STEPS guard must locate its packaged static skill reliably and fail on missing/drifting assets instead of passing vacuously. `check-steps` and `da-publish --check-steps` remain browser/data independent.

### 10. Native UI behavior

Retain Pick / Schedule / Describe / Publish / Calendar navigation, image cards, read-only schedule profiles, Custom date/time entry, approvals/backwards actions, per-card and batch AI, multi-checkbox tiers/galleries, registered-platform selection grid, partial select-all, saved result/error visibility, success-aware retries, and missing-image/history-only startup. This is functional equivalence, not pixel-identical HTML/CSS.

Use Iced state/update/view, Tasks and Subscriptions directly. Image/file/provider/browser work leaves the UI thread; Store snapshots refresh after successful operations and explicit Refresh rereads disk, not a startup queue. No polling filesystem watcher is required. UI caches are disposable and never own publication truth.

Preserve **350 ms** metadata debounce, field-level dirty state and draft revision numbers. An older save completion cannot clear/replace newer input. Flush before phase switches, approvals, generation, refresh and close; save failure keeps fields dirty, reports an error and prevents destructive navigation. Closing with unsaved work offers retry/wait or explicit discard; never silently exit after a failed save. Generation admission must finish the relevant draft flush before it reserves a UUID. Busy/disabled state follows persisted completion, not just network return.

Use built-in widgets first, no iced_aw unless a concrete missing control justifies it. Label controls, retain keyboard focus/tab navigation, accessible error/busy information and a visible dirty/saved state. Exact/Nearest distinctions must not rely only on color. Use plain text for untrusted titles/prompts/errors, no HTML renderer.

### 11. Test and cutover gates

The plan must map current behavioral tests to Rust equivalents before deleting Python coverage. HTTP-origin/body/HTML-escaping details cease to be relevant when their HTTP boundary is removed; path/model/phase validation, draft-race and publication invariants move to core/UI tests rather than being dropped. Keep Python tests for the extracted browser/worker and static STEPS contract. The existing live OpenRouter test behavior is not a reason to require network for the default suite; use local deterministic HTTP fixtures, with a separately selected opt-in live check when keyed and explicitly authorized.

Required gates:

1. Headless Rust core/CLI tests: lossless round trips, schema/allow-lists, failed writes, thread/process lock/reread behavior, all phase transitions, DST/capacity/Custom/defaults, legacy migration, prompt archive/search, thumbnail reuse and a real resize.
2. Provider fixtures: actual wire payload/MIME, no Claude path, schema rejection then prompt fallback, catalogue filtering/order, answer-model preference, overall deadline, last-error preservation, local fit/pull consent, complete-pair validation, batch ordering/admission and stale-edit merge.
3. Worker/process fixtures: browser test doubles; real stdin/stdout framing; stderr pressure, bad version/ID/order/duplicate results, failed/unknown/EOF/timeout/cancel, restart replay, success-save failure, publisher lock exclusion, no publication/config reads by Python, no work after first failed/unknown.
4. Native UI message/draft tests plus a real Linux Iced window: launch with empty, history-only and mixed disposable data; navigate all phases/calendar; exercise focus, typing while an earlier save completes, flush/close failure, checkbox multiselect, resize/scroll, lazy thumbnails, AI and fake publish/reconciliation. Headless/message tests alone are not native acceptance. To satisfy browser verification of UI stories without rebuilding a web UI, a test-only Xvfb/VNC/noVNC bridge may expose that actual native window to Playwright; all bridge tooling is development-only. Never count the old HTML UI as proof that Iced works.
5. Nix checks/build and smoke each preserved app from a disposable external data dir, including packaged schema/skill/worker assets and headless `check-steps`. Verify GUI/CLI closure ownership and the absence of Claude/llm application dependencies.
6. Before deletion/cutover: explicit feature/test mapping has no unexplained gaps; compare JSON/TOML compatibility fixtures; all safety tests and native acceptance pass; run simplification and ponytail review and apply valid findings. Then switch Nix apps, remove replaced Python/JS and obsolete tests, update maintained feature docs and AGENTS to real new paths/commands; no specs/reports get committed.

Use no real Art mutations, provider spending, model installation, original Firefox profile changes or remote submissions for demonstrations. All native acceptance uses disposable fixtures, fake provider endpoints and fake publishers. A future live DA test requires separate explicit owner authorization and cannot be implied by approving this plan.

### 12. Source evidence and documentation

Authoritative current behavior: `docs/features/*.md`; `docs/adr/0009-separate-interfaces-application-operations-and-store.md`; `src/publicator/{config,entries,store,workflow,scheduling,images,prompts,migration,llm_meta,platforms,deviantart}.py`; `src/publicator/webui/{page,server,calendar_view,prompt_view}.py`; `src/publicator/apps/`; `tests/`; `flake.nix`; the schema and static publishing skill.

Current Context7 references consulted for the design, not build/runtime verification:

- Iced 0.14.0: https://github.com/iced-rs/iced/tree/0.14.0 — State/Message/View/Update, Tasks, Subscriptions and native widgets.
- OpenRouter API: https://openrouter.ai/docs/api-reference/overview and https://openrouter.ai/docs/features/structured-outputs — chat completions, image content and response_format.
- Ollama: https://docs.ollama.com/api/tags, https://docs.ollama.com/api/pull, https://docs.ollama.com/api/chat — local inventory/install/vision protocol.

No Rust implementation, native-window compatibility, real provider result or current DeviantArt selector correctness is claimed by writing this spec. Those require the execution gates above.

</details>

## Detailed implementation plan

> **Paused reference:** Resume only after a new owner instruction. On resume use subagent-driven-development or executing-plans; the current-state table above, not historical unchecked substeps, controls the accepted frontier.

**Goal:** Replace Publicator's Python/JavaScript application with Rust and a native Iced UI, preserving its publication workflow and retaining Python only for Playwright browser work.

**Architecture:** One Cargo package provides a GUI-independent application library, a CLI binary and an optional Iced desktop binary. UI and CLI share application operations over the existing validated JSON Store. A Python browser worker receives explicit NDJSON jobs; Rust owns provider HTTP, publishing recovery and all publication writes.

**Tech Stack:** Rust; Iced 0.14.0; Tokio; Serde/serde_json; TOML; reqwest/Rustls; chrono/chrono-tz; JSON Schema 2020-12; PCRE2; OpenSSL digests; Nix; existing Git/ImageMagick/llmfit; Python 3.12/Playwright for browser code only.

**Spec:** the approved design copied above in this roadmap.

## Global Constraints

- The owner explicitly authorized implementation: "Execute the plan subagent-driven". Execute in an isolated worktree with fresh luna-6 subagents and independent task review.
- The owner authorized this durable plan/checkpoint in `docs/ROADMAP.md` when pausing. Separate review reports, logs, checklists and temporary fixtures remain in scratch, not Git.
- Iced **0.14.0**, enabled only for desktop; Tokio for async HTTP/subprocess work and blocking-task dispatch.
- Pin released compatible dependencies in Cargo.lock; do not track framework Git branches.
- AI generation supports only OpenRouter and local Ollama; remove Claude Code and the Python `llm` toolchain entirely.
- `publications.json` remains authoritative; preserve its array order, unknown fields, UUID strings, historical platforms, legacy/new hash keys, historical options and intentionally blank new drafts.
- Config/TOML is read-only. Missing config keeps current defaults; missing publication file starts empty; malformed JSON is an error, never an empty queue.
- AI, browser work, full image hashing and prompt archive indexing must not hold the publication mutation lock.
- The marker is exactly `[[PUBLICATOR:TODO_GENERATE]]`; blank fields or any occurrence of that marker block approval/publication.
- Generate **52** upcoming weekly slots per profile from an injected clock, maintaining local wall-clock hours across DST.
- Preserve configured-path Search (limits **0, 25, 50, 75, 100**, default 25/all configured paths).
- Preserve **350 ms** metadata debounce, field-level dirty state and draft revision numbers.
- OpenRouter uses reverse persisted order for the entire eligible batch (last, penultimate, ..., first). Local preserves persisted order.
- Python never receives a publications.json path, never loads publicator.toml, and never reads/writes the publication database.
- No new paid fallback is allowed. Never fall back between OpenRouter and local Ollama.
- Use no real Art mutations, provider spending, model installation, original Firefox profile changes or remote submissions for demonstrations.
- Preserve Nix app names: `ui`, `da-publish`, `generate-meta`, `vision-models`, `validate`, `echo-first`, `prompt-audit`, `check-steps`, `login`, `pw-daemon`.
- The existing `.claude/skills/publish-deviantart/SKILL.md` is a static browser recovery contract, not a dependency on the removed Claude CLI.
- For later delegation, use the owner's **luna-6** preference; resolve its exact registered provider/model ID first. This plan was drafted/reviewed directly after explicit owner permission because the attempted subagent models could not launch.

**Approval boundary:** The owner has explicitly resumed execution from Task20 fix2. After all implementation, native/package acceptance, final review/fixes and fresh verification pass, the parent is authorized to fast-forward master preserving every feature commit and push without force. No early integration/push or real data/profile/model/provider/publishing effects.

---

## Execution setup and dependency decisions

Source baseline: original master `7b5c4d6`. The isolated `feat/rust-iced` worktree already exists; inspect the checkpoint above instead of creating another branch or restarting accepted tasks. Do not rewrite history to invent a develop branch. Keep master unchanged until the final authorized integration.

All paths below are relative to that execution worktree unless absolute. Each task includes a red test, minimum implementation, green checks and a conventional commit. The code blocks identify load-bearing implementation seams, not permission to omit the source/test inventory named by the task. Expand large port steps into individual functions/tests while executing; never commit an empty stub as a completed task.

Use one package, edition 2024, with `default = []` and `desktop = ["dep:iced", "dep:image"]`. Start with compatible released dependency lines: serde 1 (derive), serde_json 1 (preserve_order), toml 0.9, tokio 1 (rt-multi-thread/macros/process/io-util/sync/time), reqwest 0.12 (json/rustls-tls, default features off), chrono 0.4, chrono-tz 0.10, clap 4 (derive), uuid 1 (v4), thiserror 2, tempfile 3, fs2 0.4, pcre2 0.2, openssl 0.10, base64 0.22, url 2, rand 0.9 and jsonschema 0.33. Iced is pinned `=0.14.0`, with image/Tokio/software-renderer support; optional image 0.25 enables BMP/TIFF decoding alongside the usual PNG/JPEG/GIF/WebP codecs. Use nix 0.30 (signal/process) for Linux worker process-group termination. Cargo.lock pins exact resolved releases. Add Tokio's test-util feature as a dev-dependency for paused-clock tests, not a shipped runtime requirement. Task 1 verifies MSRV/API compatibility inside the pinned Nix shell; these dependency choices are a proposed lock resolution, not an already-built artifact.

**Regex/digest compatibility:** Use PCRE2 UTF/UCP with Python-style `(?P<version>...)` / optional `lineage`, require a match starting at byte zero (Python `re.match`, even for unanchored patterns), and test Unicode classes, lookbehind and a named backreference. Do not pretend PCRE2 is identical to every Python `re` extension: invalid/unsupported syntax is a clear configuration error. OpenSSL `MessageDigest::from_name` provides runtime selection; normalize Python SHA3 underscore spelling and aliases `blake2b`/`blake2s` to the corresponding fixed-size OpenSSL names. Fixed-output digest names used by current data/tests must keep identical bytes. Reject unsupported/XOF-without-length algorithms at load, rather than changing a digest silently. No filename grammar or selected prompt digest is hardcoded in application source.

**Native testing:** Use the real Iced view/update code in a development-only acceptance example with injected fake endpoints/worker and disposable data. A shell harness starts Xvfb + x11vnc + noVNC bound to loopback; Playwright drives that actual native window through the remote-desktop canvas. This is not a second application UI. Do not add the bridge or test fake providers to the shipped runtime closure.

## File/responsibility map

| File(s) | Responsibility |
|---|---|
| `Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `src/error.rs` | One package, exports, concrete error/result types |
| `assets/publicationsSchema.json` | Existing unchanged schema, compiled into Rust |
| `src/config.rs`, `src/model.rs` | Data-dir config, grammar/digest boundary; lossless JSON and state readers |
| `src/store.rs` | Fresh reads, short mutex/flock mutations, validation, atomic replacement |
| `src/images.rs` | Candidate authorization/discovery, hashes, thumbnail cache, upload preparation |
| `src/scheduling.rs` | Profiles, configured-zone instants, occupancy/repack and Custom conversion |
| `src/workflow.rs` | Persisted pick/date/metadata/selection/options/delivery operations |
| `src/migration.rs`, `src/calendar.rs` | Pure legacy conversion; read-only per-apparition event projection |
| `src/prompts.rs` | Git blob/history index, distinct match variants, search/audit |
| `src/metadata/{mod,openrouter,local}.rs` | Shared schema/parser/catalogue; two concrete HTTP providers |
| `src/application.rs` | Operation admission, busy UUIDs, AI batch orchestration and publish coordination |
| `src/publish_protocol.rs` | Versioned explicit worker commands/events; no persistence |
| `src/publish_journal.rs` | Durable attempt/receipt states, fingerprints and idempotent reconciliation |
| `src/publishing.rs` | Preflight, worker lifecycle, sequential sends and Store result recording |
| `src/bin/publicator.rs` | Thin CLI adapters, consent, output/exit statuses |
| `src/bin/publicator-ui.rs`, `src/ui/{mod,drafts,pick,schedule,describe,publish,calendar,prompt}.rs` | Native startup, interaction state and feature views |
| `python/publicator_da/{__init__,browser,worker,pw_daemon}.py` | Retained browser/session/STEPS/prober code only |
| `tests/common/{mod,http}.rs`, `tests/*.rs`, `tests/support/fake-worker.sh` | Rust fixtures, behavioral/HTTP/process tests |
| `tests/python/test_{browser,worker}.py` | Browser doubles and Python protocol tests |
| `examples/ui_acceptance.rs`, `scripts/native-ui-smoke.sh` | Real native-window disposable acceptance harness |
| `flake.nix`, `flake.lock`, `pyproject.toml`, `.gitignore` | Build/runtime/dev tooling and cutover |
| `docs/features/*.md`, `docs/README.md`, `AGENTS.md` | Maintained operational docs updated with real paths |

Do not add a platform trait/registry loader: one executable DA identifier plus historical data readers is sufficient. Do not port HTTP/HTML implementation details to Rust.

## Shared interfaces and conventions

These names are the task-to-task contract. Types not needed until a later task are introduced by that task, not empty forward modules.

```rust
// error.rs
pub type Result<T> = std::result::Result<T, Error>;
// Error is a concrete thiserror enum: Io, Json, Config, Validation,
// NotFound, InvalidState, Busy, Provider, Protocol, ReconciliationRequired.
// Each variant carries its cause/context; never collapse an unknown outcome into Provider.

// model.rs; preserve documents, do not deserialize/re-serialize away extension fields.
pub type Publications = Vec<serde_json::Value>;
pub const GENERATE_MARKER: &str = "[[PUBLICATOR:TODO_GENERATE]]";
pub enum Phase { Picked, Scheduled, Described, Appeared }
pub enum Delivery { Unpublished, PublishedOrScheduled }
pub fn phase(row: &serde_json::Value) -> Result<Phase>;
pub fn effective_schedule(row: &serde_json::Value) -> Option<i64>;
pub fn has_delivery_success(row: &serde_json::Value) -> bool;
pub fn validate_metadata(title: &str, description: &str) -> Result<()>;

// config.rs
pub struct Paths { pub data_dir: std::path::PathBuf, pub json: std::path::PathBuf }
impl Paths {
    pub fn resolve(data_dir: Option<&std::path::Path>, json: Option<&std::path::Path>) -> Result<Self>;
}
pub struct Config {
    pub tiers: Vec<String>, pub galleries: Vec<String>, pub publicable: Vec<String>,
    pub tags: Option<String>, pub schedule: toml::Table, pub prompts: Option<PromptConfig>,
}
// Config implements Clone and Default (current defaults); schedule stays raw until scheduling reads it.
pub struct PromptConfig { pub repo: String, pub filename: String, pub version_hash: String }
pub struct PromptGrammar {
    pattern: pcre2::bytes::Regex,
    digest: openssl::hash::MessageDigest,
}
impl Config { pub fn load(data_dir: &std::path::Path) -> Result<Self>; }
impl PromptGrammar {
    pub fn compile(config: &PromptConfig) -> Result<Self>;
    pub fn identity(&self, basename: &str) -> Result<Option<ImageIdentity>>;
    pub fn digest(&self, bytes: &[u8]) -> Result<String>;
}
pub struct ImageIdentity { pub version: String, pub lineage: Option<String> }
pub fn validate_publications(rows: &Publications, config: &Config) -> Result<()>;

// store.rs
pub struct Store {
    paths: Paths,
    config: Config,
    // The schema validator is compiled once in new(); its concrete crate type is private.
    validator: jsonschema::Validator,
}
impl Store {
    pub fn new(paths: Paths, config: Config) -> Result<Self>;
    pub fn paths(&self) -> &Paths;
    pub fn config(&self) -> &Config;
    pub fn read(&self) -> Result<Publications>;
    pub fn mutate<R>(&self, f: impl FnOnce(&mut Publications) -> Result<R>) -> Result<R>;
}
```

`read` parses missing as empty, rejects malformed data and validates normal live documents. The pure migration parser separately accepts raw legacy documents. Store mutation revalidates the latest input and complete output. Use `Value` accessors returning contextual errors, not unchecked indexing of untrusted input.

`tests/common/mod.rs` owns these actual fixture helpers from Task 2 onward:

```rust
pub fn id(n: u64) -> String { format!("00000000-0000-4000-8000-{n:012}") }
pub fn row(n: u64, state: &str) -> serde_json::Value {
    serde_json::json!({"uuid": id(n), "state": state, "title": "Title",
        "description": "Description", "submissionTimestamp": 1_700_000_000,
        "files": [{"basename": format!("{n}.png"), "sha512sum": "0".repeat(128)}],
        "apparitions": []})
}
pub fn epoch(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
}
pub struct Fixture { pub root: tempfile::TempDir, pub store: publicator::store::Store }
// Fixture::new(), Fixture::with_config(Config), path(&str) -> PathBuf,
// write(Value) -> (), read() -> Value. write intentionally bypasses live validation for red tests.
// image(name, width, height) -> PathBuf runs pinned ImageMagick to create a real PNG.
```

Each test file imports `mod common; use common::{Fixture, id, row, epoch};` only for helpers it uses. Never add tests that succeed because a missing tool silently skips them inside Nix.

## Milestone A — Rust data/core/CLI validation

### Task 1: Boot a working Rust validator with lossless configuration

**Files:** Create `Cargo.toml`, `Cargo.lock`, `src/{lib,error,config,model}.rs`, `src/bin/publicator.rs`, `assets/publicationsSchema.json`, `tests/config.rs`; modify `flake.nix` dev shell only.

**Interfaces:** Produce all shared model/config interfaces above and a working `publicator validate --data-dir DIR [--json FILE]`. No Store or desktop references yet. Validation uses the embedded unchanged schema plus existing URL/obsolete-field/DA allow-list checks.

- [ ] **Red — configuration/schema:** Write tests using `tempfile` directly: absent TOML defaults; invalid TOML; named captures and byte-zero matching; configured SHA-256/SHA-1 content identities; unknown fields retained; bad URL/obsolete `tier` rejected; explicit zero price retained. Representative assertion:
  ```rust
  #[test]
  fn grammar_is_configured_not_assumed() {
      let g = PromptGrammar::compile(&PromptConfig { repo: "archive".into(),
          filename: r"img-(?P<version>[a-f0-9]+)\.png".into(), version_hash: "sha256".into() }).unwrap();
      assert_eq!(g.digest(b"abc").unwrap(),
          "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
      assert!(g.identity("prefix-img-a.png").unwrap().is_none());
  }
  ```
- [ ] **Run red:** Add Rust/cargo/rustfmt/clippy/pkg-config/OpenSSL/PCRE2 development dependencies, retaining the old app tooling until cutover. Run `nix develop -c cargo test --no-default-features --test config`; first failure must be missing new Rust functionality, not an unavailable compiler/library.
- [ ] **Implement:** Resolve Paths against data-dir/CWD and optional JSON path. Compile the bundled schema once per validator/Store, enable its correct draft and explicit URL host checks. Use raw JSON values. Reject invalid configuration with the field name. CLI prints `Valid` only after validation; errors go stderr/nonzero.
  ```rust
  pub fn validate_metadata(title: &str, description: &str) -> Result<()> {
      if [title, description].iter().any(|s| s.trim().is_empty() || s.contains(GENERATE_MARKER)) {
          return Err(Error::Validation("complete unmarked metadata required".into()));
      }
      if title.chars().count() > 50 {
          return Err(Error::Validation("title exceeds 50 characters".into()));
      }
      Ok(())
  }
  ```
  Preserve schema UUID-string length behavior rather than requiring historical UUID strings to parse as the uuid crate's structured type. Use uuid only to create new identities.
- [ ] **Green:** Run the config test, CLI `--help`, a temporary valid/invalid JSON CLI check, and `nix develop -c pytest tests/test_config.py -q`. During coexistence COPY the schema into `assets/`; do not remove the Python schema yet. Compare both schema files byte-for-byte.
- [ ] **Commit:** `git add Cargo.toml Cargo.lock src/lib.rs src/error.rs src/config.rs src/model.rs src/bin/publicator.rs assets/publicationsSchema.json tests/config.rs flake.nix && git commit -m "feat(core): add Rust configuration and validator"`.

### Task 2: Atomic locked Store with fresh reads

**Files:** Create `src/store.rs`, `tests/store.rs`, `tests/common/mod.rs`; modify `src/lib.rs`.

**Interfaces:** Consume Paths/Config/validation; produce `Store` and Fixture methods above. Expose no unchecked alternate write path.

- [ ] **Red:** Add unknown-field round-trip, missing/malformed JSON, rollback-on-validation-failure and concurrent process/thread tests. A child-process locking test can re-execute the integration test binary with an environment-selected helper case; it must wait on the same persistent `<json>.lock` as a parent holding fs2/flock.
  ```rust
  #[test]
  fn invalid_edit_does_not_replace_original() {
      let f = Fixture::new(); f.write(serde_json::json!([row(1, "PICKED")]));
      let before = std::fs::read(&f.store.paths().json).unwrap();
      let result = f.store.mutate(|rows| { rows[0]["state"] = "wrong".into(); Ok(()) });
      assert!(result.is_err());
      assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test store`.
- [ ] **Implement:** Short process mutex, persistent lock file, fresh validated read, closure operation, output validation, `NamedTempFile::new_in(parent)`, JSON write/flush/sync and atomic persist/rename. Keep the lock until replacement finishes. Any pre-rename failure preserves original bytes and removes staging files. After rename, durability/cleanup errors use a distinct typed committed-but-uncertain outcome retaining cause/path: do not promise unchanged data or attempt speculative rollback. Keep directory sync; preserve the operation's error rather than masking it with unlock/cleanup failure. Config always belongs to the data directory even when JSON is elsewhere.
  ```rust
  let result = f(&mut rows)?;
  validate_publications(&rows, &self.config)?;
  let mut temp = tempfile::NamedTempFile::new_in(parent)?;
  serde_json::to_writer_pretty(temp.as_file_mut(), &rows)?;
  temp.as_file_mut().sync_all()?;
  temp.persist(&self.paths.json).map_err(|e| Error::Io(e.error))?;
  ```
  Add lock acquisition/release and input reads around this seam; test replacement failure separately from validation failure. Directory fsync is also required for the publishing journal's durability later.
- [ ] **Green:** Run Store/config tests; prove two Store instances merge separate edits rather than replace stale snapshots; lock file remains. The subprocess must invoke real Store with a readiness handshake while an external interoperable flock is held, not merely test fs2 against itself. Prove staging failure occurs after lock creation and actual rename failure preserves old bytes; a narrow private commit-helper test deterministically injects post-rename sync failure and asserts changed bytes plus typed uncertainty. Verify a JSON path outside data-dir still uses its configured option allow-lists. Record the exact uncertainty Error variant and later Application/Publisher/UI handling obligation in the ledger.
- [ ] **Commit:** `git add src/store.rs src/lib.rs tests/store.rs tests/common/mod.rs && git commit -m "feat(store): add fresh validated atomic mutations"`.

### Task 3: Candidate discovery, authorization and shared thumbnails

**Files:** Create `src/images.rs`, `tests/images.rs`; modify exports and Fixture::image.

**Interfaces:** `Search {limit: usize, paths: Vec<String>}`; `Candidate {path: PathBuf, sha512: String}`; `file_sha512(path: &Path) -> Result<String>`; `find_candidates(store: &Store, search: &Search) -> Result<Vec<Candidate>>`; `thumb_name(path: &Path, sha: &str) -> Result<String>`; `ensure_thumb(path: &Path, cache: &Path, name: &str) -> Result<PathBuf>`; `prepare_ai_image(path: &Path, cache: &Path, known_sha: Option<&str>) -> Result<PreparedImage>` where `PreparedImage {path: PathBuf, mime: String}`; `prepare_upload(source: &Path, attempt_dir: &Path) -> Result<PathBuf>`; `delete_candidate(store: &Store, authorized: &[Candidate], path: &Path) -> Result<()>`.

- [ ] **Red:** Test all accepted limits, unconfigured/duplicate paths, both hash keys and previewFiles hashes, referenced filenames with changed bytes, hidden runtime directories in basename lookup, real resize, lazy cache reuse and mixed-case WebP conversion. Use ImageMagick-created images, not fake PNG signatures, for the resize proof.
  ```rust
  #[test]
  fn cache_reuses_content_identity_and_really_resizes() {
      let f = Fixture::new(); let p = f.image("large.png", 900, 600);
      let sha = publicator::images::file_sha512(&p).unwrap();
      let name = thumb_name(&p, &sha).unwrap();
      assert_eq!(name, format!("{}.png", &sha[..32]));
      let t = ensure_thumb(&p, &f.path(".thumbs"), &name).unwrap();
      let out = std::process::Command::new("identify").args(["-format", "%wx%h"]).arg(t).output().unwrap();
      assert_eq!(String::from_utf8(out.stdout).unwrap(), "300x200");
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test images`.
- [ ] **Implement:** Reuse directory traversal and streamed hashing; shuffle only eligible search input; stop at requested candidate count. Distinguish a current search authorization set from historical basename resolution. Before deletion, reread publication references (files and previews), not just current hashes. No delete of picked/referenced files. Cache misses call ImageMagick, falling back to original with a visible diagnostic; detect actual image MIME before provider upload. Preserve video candidates with preview-unavailable state, never send video bytes as `image/png`.
  ```rust
  let status = std::process::Command::new("convert")
      .arg(source).args(["-resize", "300x300>"]).arg(&thumb).status()?;
  if !status.success() { std::fs::copy(source, &thumb)?; }
  ```
  Use a temporary output and atomic cache publication for concurrent requests; leave no partial cached image after failure. Upload conversion lives in a unique attempt directory. Always convert a selected WebP rather than adding a sibling-search/visual-identity heuristic; an already selected non-WebP is used unchanged. This takes the spec's conversion alternative and removes the old substring-glob ambiguity.
- [ ] **Green:** Run image/Store tests and existing `tests/test_images.py`. Confirm cached names survive renames and no full history thumbnail generation occurs during discovery.
- [ ] **Commit:** `git add src/images.rs src/lib.rs tests/images.rs tests/common/mod.rs && git commit -m "feat(images): port discovery and persistent thumbnail cache"`.

### Task 4: Timezone scheduling and capacity

**Files:** Create `src/scheduling.rs`, `tests/scheduling.rs`.

**Interfaces:** `Profile {name, weekday: chrono::Weekday, hour: u32, per_slot: usize, default_tiers: Vec<String>, default_price: Option<f64>}`; `profiles(config: &Config) -> Result<Vec<Profile>>`; `slots(profile: &Profile, zone: chrono_tz::Tz, now: i64) -> Result<Vec<i64>>`; `resolve_wall_clock(input: &str, zone: Tz) -> Result<i64>`; `repack(rows: &mut Publications, config: &Config, now: i64) -> Result<()>`.

- [ ] **Red:** Port the table of profile validation, DST, historical occupancy, horizon exhaustion, manual pins and explicit overcapacity checks. Include spring gap and autumn fold:
  ```rust
  #[test]
  fn custom_times_match_zoneinfo_fold_zero() {
      let tz = chrono_tz::Europe::Paris;
      assert_eq!(resolve_wall_clock("2026-10-25T02:30", tz).unwrap(), epoch("2026-10-25T00:30:00Z"));
      assert_eq!(resolve_wall_clock("2026-03-29T02:30", tz).unwrap(), epoch("2026-03-29T01:30:00Z"));
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test scheduling`.
- [ ] **Implement:** Normalize flat/default and named profiles, reject empty/colliding profiles, invalid hours/capacity/prices/default tiers. Construct each weekly local date independently; do not add fixed UTC weeks. `LocalResult::Ambiguous` chooses earlier UTC; for a gap find the pre-transition offset and convert the original naive wall time with it. Bound the search and error instead of looping indefinitely. Ignore embedded offsets on custom wall-clock input per current contract.
  ```rust
  for row in rows.iter().filter(|r| phase(r).ok() != Some(Phase::Picked)
      || r.get("scheduleIsManual").and_then(serde_json::Value::as_bool) == Some(true)) {
      if let Some(ts) = effective_schedule(row) { *occupancy.entry(ts).or_insert(0usize) += 1; }
  }
  ```
  Then clear/reallocate only automatic PICKED suggestions in stored order; preserve Custom profile absence/mismatch and leave date absent after the 52-slot horizon is full.
- [ ] **Green:** Run scheduling/config/Store tests and `nix develop -c pytest tests/test_scheduling.py -q`; check occupancy is per publication regardless of apparition count.
- [ ] **Commit:** `git add src/scheduling.rs src/lib.rs tests/scheduling.rs && git commit -m "feat(schedule): preserve timezone slots and custom pins"`.

### Task 5: Pick and date approval operations

**Files:** Create `src/workflow.rs`, `tests/workflow_dates.rs`.

**Interfaces:** `DateEdit::Preset(String) | Custom(String) | Epoch(i64) | Suggested`; `pick(store: &Store, candidate: &Candidate, now: i64) -> Result<String>`; `edit_date(store, uuid: &str, edit: DateEdit, now: i64) -> Result<()>`; `approve_date(store, uuid, now) -> Result<()>`; `return_to_scheduling(store, uuid) -> Result<()>`.

- [ ] **Red:** Pick idempotency by content; no apparition/metadata on pick; one atomic pick+repack; date approval requires a visible date; Custom/removed-profile preservation; no backwards/date edit after a delivery success.
  ```rust
  #[test]
  fn pick_does_not_select_a_platform() {
      let f = Fixture::new(); let p = f.image("new.png", 10, 10);
      let c = Candidate { sha512: file_sha512(&p).unwrap(), path: p };
      let u = pick(&f.store, &c, 1_800_000_000).unwrap();
      assert_eq!(pick(&f.store, &c, 1_800_000_000).unwrap(), u);
      let rows = f.store.read().unwrap();
      assert_eq!(rows[0]["state"], "PICKED"); assert_eq!(rows[0]["apparitions"], serde_json::json!([]));
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test workflow_dates`.
- [ ] **Implement:** Hash before entering Store; inside the fresh mutation deduplicate legacy/new hashes case-insensitively, append a UUID-v4 PICKED row, and repack in that same write. A custom edit sets the chosen epoch/manual pin but does not invent a matching profile. A preset or Suggested explicitly clears pin and reallocates. Recheck phase/delivery locks inside every mutation.
  ```rust
  if has_delivery_success(pub_) || phase(pub_)? != Phase::Picked {
      return Err(Error::InvalidState("date editing requires undelivered PICKED".into()));
  }
  pub_["apparitionTimestampIfDifferentThanSubmission"] = ts.into();
  pub_["scheduleIsManual"] = true.into();
  ```
- [ ] **Green:** Run workflow_dates/scheduling/Store tests; verify no metadata/provider/platform operation occurs during pick or date approval.
- [ ] **Commit:** `git add src/workflow.rs src/lib.rs tests/workflow_dates.rs && git commit -m "feat(workflow): port picking and schedule approval"`.

### Task 6: Metadata drafts, approval and generated merges

**Files:** Modify `src/workflow.rs`; create `tests/workflow_metadata.rs`.

**Interfaces:** `Metadata {title: String, description: String}`; `MetadataPatch {title: Option<String>, description: Option<String>}`; `save_metadata(store, uuid, patch: &MetadataPatch)`; `approve_metadata(store, uuid)`; `save_generated_pair(store, uuid, pair: &Metadata)`; `fail_generation(store, uuid)`; `merge_generated_markers(store, uuid, pair)`; `return_to_metadata(store, uuid)` — all return `Result<()>`.

- [ ] **Red:** Draft != approval; complete generated pair advances; accepted single failure writes both markers; any marker occurrence blocks approval; marker-only batch merge preserves newer manual fields; successful delivery freezes metadata/backwards edits. Test 50 non-ASCII characters versus 51, not byte length.
  ```rust
  #[test]
  fn merge_does_not_overwrite_a_manual_field() {
      let f = Fixture::new(); let mut p = row(1, "SCHEDULED");
      p["title"] = "Manual".into(); p["description"] = GENERATE_MARKER.into(); f.write(serde_json::json!([p]));
      merge_generated_markers(&f.store, &id(1), &Metadata {title:"AI".into(), description:"Story".into()}).unwrap();
      assert_eq!(f.read()[0]["title"], "Manual"); assert_eq!(f.read()[0]["state"], "DESCRIBED");
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test workflow_metadata`.
- [ ] **Implement:** Each operation uses Store fresh mutation and SCHEDULED/delivery checks. Single accepted failure sets both exact markers without advancing. Validate an incoming generated pair before applying it. Marker-only merge persists valid generated replacements even when a preserved unmarked manual field is blank/incomplete, leaving SCHEDULED; it must not roll back that useful replacement. Advance only if at least one generated field was applied and the saved pair is complete/valid. Zero-marker stale results never approve an entirely manual draft. A batch error does not call `fail_generation` over a manually edited counterpart.
  ```rust
  for (key, value) in [("title", &pair.title), ("description", &pair.description)] {
      if pub_.get(key).and_then(serde_json::Value::as_str) == Some(GENERATE_MARKER) {
          pub_[key] = value.clone().into();
      }
  }
  ```
- [ ] **Green:** Run metadata/date/Store tests; include an edit arriving while a provider test is blocked, then release result and prove that only the still-marked field changes. Cover a blank/unapproved preserved counterpart: save the valid marked-field result but remain SCHEDULED. Cover zero markers after a concurrent manual edit: do not advance or overwrite either manual field. Retain invalid-incoming-pair rejection with no partial write.
- [ ] **Commit:** `git add src/workflow.rs tests/workflow_metadata.rs && git commit -m "feat(workflow): preserve metadata approval and merge rules"`.

### Task 7: Platform selection, options and completion

**Files:** Modify `src/workflow.rs`, `src/model.rs`; create `tests/workflow_platforms.rs`.

**Interfaces:** `REGISTERED_PLATFORMS: &[&str] = &["deviantart"]`; `select(store, uuid: Option<&str>, platform: Option<&str>, selected: bool)`; `PlatformOptions {price: Option<Option<f64>>, tiers: Option<Vec<String>>, galleries: Option<Vec<String>>}`; `save_options(store, uuid, options)`; `record_delivery(store, uuid, platform, actual_url: Option<&str>)`. Absence of price edit differs from explicitly clearing it; explicit zero is retained.

- [ ] **Red:** Select-all is one mutation; successful cells are immutable; new pending target reopens APPEARED; historical pending targets remain visible and prevent total completion without entering executable dispatch; defaults apply once, never to unmatched Custom. Options target DA, not the first apparition.
  ```rust
  #[test]
  fn historical_pending_prevents_appeared() {
      let f = Fixture::new(); let mut p = row(1,"DESCRIBED");
      p["apparitions"] = serde_json::json!([
          {"platformName":"pixiv","state":"unpublished"},
          {"platformName":"deviantart","state":"unpublished"}]); f.write(serde_json::json!([p]));
      record_delivery(&f.store,&id(1),"deviantart",None).unwrap();
      assert_eq!(f.read()[0]["state"],"DESCRIBED");
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test workflow_platforms`.
- [ ] **Implement:** Only registered identifiers can be selected/executed. Preserve historical records. Apply profile defaults only while creating a new DA apparition; compute APPEARED iff at least one remaining apparition and all succeeded. Do not resubmit or remove a successful selection.
  ```rust
  let complete = apps.iter().all(|a| a.get("state").and_then(serde_json::Value::as_str)
      == Some("published_or_scheduled"));
  pub_["state"] = if complete { "APPEARED" } else { "DESCRIBED" }.into();
  ```
- [ ] **Green:** Run all workflow tests and port option-validation, historical-only selection no-op and completion expectations from `tests/test_platforms.py`/`tests/test_server.py`. The registered-cell remaining/retry integration in `test_selections_all_options_success_and_retry_restore` belongs to Task 16, which owns real pending-target selection and publishing reports; it is an explicit Task 16 acceptance obligation, not an assumed Task 7 result.
- [ ] **Commit:** `git add src/workflow.rs src/model.rs tests/workflow_platforms.rs && git commit -m "feat(workflow): port platform selection and delivery completion"`.

### Task 8: Pure migration and calendar data projection

**Files:** Create `src/migration.rs`, `src/calendar.rs`, `tests/migration.rs`, `tests/calendar.rs`.

**Interfaces:** `migrate(rows: &Publications, recovered: &std::collections::HashMap<String,String>, config: &Config) -> Result<Publications>`; `Event {uuid: String, apparition_index: usize, platform: String, title: String, timestamp: i64, url: Option<String>, basename: Option<String>, delivery: Delivery}`; `timeline(rows: &Publications) -> Result<Vec<Event>>`; `months(events: &[Event], zone: Tz, now: i64) -> Result<Vec<chrono::NaiveDate>>`.

- [ ] **Red:** Port all `tests/test_migration.py` conflict/title/idempotency/draft cases; calendar all-platform/shared-title/date/URL, independent event identities and current-month inclusion.
  ```rust
  #[test]
  fn calendar_keeps_two_apparitions_with_one_shared_title() {
      let mut p=row(1,"DESCRIBED"); p["apparitions"]=serde_json::json!([
          {"platformName":"deviantart","state":"unpublished"},
          {"platformName":"historical","state":"published_or_scheduled"}]);
      let ev=timeline(&vec![p]).unwrap(); assert_eq!(ev.len(),2);
      assert_ne!(ev[0].apparition_index,ev[1].apparition_index);
      assert_eq!(ev[0].title,ev[1].title);
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test migration --test calendar`.
- [ ] **Implement:** Clone legacy input, move old URLs/shared dates, reject conflicting names/dates, require supplied verified recovery titles, infer phase and validate output. Keep this pure and disconnected from startup. Calendar projects every apparition; no URL-title inference or network title lookup.
  ```rust
  let mut out = rows.clone();
  // Apply the existing migration.py transformation to out, not rows.
  validate_publications(&out, config)?;
  Ok(out)
  ```
  Implement the actual field-by-field transformation before calling this task complete; include the complete current migration test table, not only the clone/validation seam.
- [ ] **Green:** Run migration/calendar plus all headless tests; unchanged input on every failed migration; no filesystem writes anywhere in `migrate`.
- [ ] **Commit:** `git add src/migration.rs src/calendar.rs src/lib.rs tests/migration.rs tests/calendar.rs && git commit -m "feat(core): port migration and calendar projection"`.

### Task 9: Content-addressed prompt archive, search and audit

**Files:** Create `src/prompts.rs`, `tests/prompts.rs`, `tests/prompt_search.rs`; modify exports.

**Interfaces:** `PromptVersion {digest: String, text: String, paths: Vec<String>, committed: i64}`; `Lineage {path: String, versions: Vec<PromptVersion>}`; `PromptMatch::Exact(PromptVersion) | Nearest(Vec<Lineage>) | Unknown(String)`; `PromptArchive::new(data_dir: &Path, config: &PromptConfig) -> Result<Self>`; `resolve_path(&self,path:&Path)->Result<PromptMatch>`; `search(store: &Store, search: &Search, archive: Option<&PromptArchive>, needle: Option<&str>, lineage: Option<&str>) -> Result<SearchMatches>` where `SearchMatches {exact: Vec<Candidate>, maybe: Vec<Candidate>, skipped: usize, truncated: bool}`; `audit(data_dir: &Path, config: &Config, archive: Option<&PromptArchive>) -> Result<AuditCounts>` with usize fields files/parsed/exact/nearest/unknown and exact_versions/nearest_versions.

- [ ] **Red:** Build disposable Git repos with superseded commits, removed paths, duplicate basenames and unreachable blobs; verify raw-content digest versus object ID, all ranked candidates retained, 2-second HEAD throttle, malformed length framing, exact-only text search, 200-group cap before hashing, feature-off counts.
  ```rust
  #[test]
  fn superseded_filename_resolves_old_content_not_head() {
      let tmp = tempfile::tempdir().unwrap();
      let git = |args: &[&str]| {
          assert!(std::process::Command::new("git").arg("-C").arg(tmp.path())
              .args(args).status().unwrap().success());
      };
      git(&["init", "-q"]);
      for text in ["old prompt", "new prompt"] {
          std::fs::write(tmp.path().join("scene.txt"), text).unwrap();
          git(&["add", "scene.txt"]);
          git(&["-c", "user.name=Test", "-c", "user.email=test@example.invalid",
              "commit", "-qm", "snapshot"]);
      }
      let cfg = PromptConfig { repo: ".".into(),
          filename: r"image-(?P<version>[a-f0-9]+)\.png".into(), version_hash: "sha256".into() };
      let digest = PromptGrammar::compile(&cfg).unwrap().digest(b"old prompt").unwrap();
      let archive = PromptArchive::new(tmp.path(), &cfg).unwrap();
      match archive.resolve_path(&tmp.path().join(format!("image-{digest}.png"))).unwrap() {
          PromptMatch::Exact(v) => assert_eq!(v.text, "old prompt"),
          other => panic!("expected archived exact version, got {other:?}"),
      }
  }
  ```
  Match variants derive Debug; render text only after matching Exact. Nearest archive/ambiguous-lineage tests must resolve real fixtures, not assert that a locally constructed enum matches itself.
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test prompts --test prompt_search`.
- [ ] **Implement:** Use Git `cat-file --batch-all-objects --batch` and length-framed bytes plus `log --all --raw --no-abbrev --no-renames`; build a complete replacement index before swapping it under a short archive lock. Git/index work never holds Store mutation locks. Match config roles, not filename assumptions. Keep directory-similarity ranking stable; use the current SequenceMatcher-equivalent matching-block score or verify an exact port against ranking fixtures, never replace with a first-match heuristic.
  ```rust
  let body_start = newline + 1;
  let body_end = body_start.checked_add(size).ok_or_else(|| Error::Protocol("blob size overflow".into()))?;
  let body = bytes.get(body_start..body_end).ok_or_else(|| Error::Protocol("truncated blob".into()))?;
  ```
  Invalid header lines skip to the next header; truncated declared payload must not be interpreted as a fresh prompt. Broken optional Git archives produce Unknown plus a diagnostic, not a fabricated exact result.
- [ ] **Green:** Run prompt/search/config/image tests; compare parse/exact/nearest/unknown output on committed disposable compatibility fixtures. Do not inspect/mutate the real archive as test setup.
- [ ] **Commit:** `git add src/prompts.rs src/lib.rs tests/prompts.rs tests/prompt_search.rs && git commit -m "feat(prompts): port content-addressed archive and search"`.

## Milestone B — direct AI and Python browser boundary

### Task 10: Shared metadata schema/parser and HTTP test seam

**Files:** Create `src/metadata/mod.rs`, `tests/metadata.rs`, `tests/common/http.rs`; modify exports.

**Interfaces:** `Source::OpenRouter | Local` derives Copy/Clone/Debug/Eq/PartialEq/serde with lowercase wire names; `available_sources() -> &'static [Source]` returns `[OpenRouter, Local]` (preserves the current UI's initial OpenRouter choice); `ModelOption {ollama_name: String, openrouter_name: String, llmfit_name: String}`; `VISION_MODELS` retains the existing explicit catalogue records; `GenerationRequest {source: Source, model: ModelOption, timeout: Duration, install_missing: bool}`; `Generated {metadata: Metadata, model_used: String}`; `MetadataClient::new(http: reqwest::Client, endpoints: Endpoints, llmfit_program: PathBuf) -> Self`; `Endpoints {openrouter: String, ollama: String}` with production defaults `https://openrouter.ai/api/v1` and `http://localhost:11434`; `parse_metadata(content:&str)->Result<Metadata>`; `title_description_schema() -> serde_json::Value`; `metadata_prompt() -> &'static str`. Introduce the shared async `generate(&self,image:&PreparedImage,request:&GenerationRequest,preferred:Option<&str>)->Result<Generated>` dispatcher in Task12 after both concrete provider methods exist; do not commit a failing/stub provider dispatcher in this task.

- [ ] **Red:** Pair validation rejects nonstrings/blank/marker/overlong title; fenced/prose-wrapped JSON accepted; only two source enum values deserialize; no Claude CLI path. Define `tests/common/http.rs::Server` using a loopback `TcpListener`, a finite scripted `Reply {path,status,body}` queue, captured request bodies and bounded socket deadlines. `Server::start(Vec<Reply>)`, `url()`, `requests()` are its only API; tests fail unexpected paths and join the server thread on cleanup.
  ```rust
  #[test]
  fn parses_fenced_pair_but_not_partial_metadata() {
      assert_eq!(parse_metadata("```json\n{\"title\":\"T\",\"description\":\"D\"}\n```").unwrap().title,"T");
      assert!(parse_metadata("{\"title\":\"T\"}").is_err());
      assert!(serde_json::from_str::<Source>("\"claude\"").is_err());
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test metadata`.
- [ ] **Implement:** Shared instructions/schema; JSON outermost-object parsing followed by strict pair validation; actual image MIME/base64 encoding; errors distinguish transport/unavailable from invalid response without swallowing original causes. Production endpoints are constants in construction, not arbitrary UI URL fields. Client/request injection supplies local test endpoints without altering shipped defaults.
  ```rust
  let begin = content.find('{').ok_or_else(|| Error::Provider("no JSON object".into()))?;
  let end = content.rfind('}').ok_or_else(|| Error::Provider("no JSON object".into()))?;
  let pair: Metadata = serde_json::from_str(&content[begin..=end])?;
  validate_metadata(&pair.title,&pair.description)?;
  ```
- [ ] **Green:** Parser tests plus a self-test of the loopback HTTP fixture using reqwest. The real provider payload/MIME/model-mapping checks run in Tasks11–12, after those clients exist. Redact auth/body image data from diagnostic output.
- [ ] **Commit:** `git add src/metadata src/lib.rs tests/metadata.rs tests/common/http.rs && git commit -m "feat(ai): add Rust metadata schema and HTTP boundary"`.

### Task 11: OpenRouter schema recovery and bounded free-model fallback

**Files:** Create `src/metadata/openrouter.rs`, `tests/openrouter.rs`; modify `src/metadata/mod.rs`.

**Interfaces:** Concrete MetadataClient methods `catalogue(&self, deadline: tokio::time::Instant) -> Result<Vec<CatalogModel>>`, `openrouter_attempt(&self, image: &PreparedImage, model: &str, schema: bool, deadline: tokio::time::Instant) -> Result<Generated>`, and `openrouter_generate(&self, image: &PreparedImage, request: &GenerationRequest, preferred: Option<&str>) -> Result<Generated>`; `CatalogModel {id: String, image_input: bool, free: bool, schema_supported: Option<bool>}` and `CatalogModel::from_api(value: &serde_json::Value) -> Result<Self>`. The concrete OpenRouter method implements selected/preferred then dynamic fallback, no provider trait.

- [ ] **Red:** Script HTTP fixtures for 429→other vendor→success; unsupported response_format→one prompt-shaped retry; lazy catalogue only after a failure; malformed catalogue preserves original model error; nonfree/nonvision entries excluded; memoized schema rejection; fallback result reports answering model; a total deadline spans catalogue and attempts. Use paused Tokio time where only timers are involved and bounded real loopback delays for HTTP.
  ```rust
  #[test]
  fn catalogue_requires_vision_and_zero_pricing() {
      let v=serde_json::json!({"id":"vendor/free","architecture":{"input_modalities":["text","image"]},
          "pricing":{"prompt":"0","completion":"0"},"supported_parameters":[]});
      let m=CatalogModel::from_api(&v).unwrap(); assert!(m.image_input && m.free);
  }
  ```
  Add a nonzero-image/request-fee fixture so no advertised per-image/per-request cost is accepted as a free fallback.
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test openrouter`.
- [ ] **Implement:** Read runtime OPENROUTER_KEY; strip only the application `openrouter/` prefix; POST text plus base64 image URL to `/chat/completions`. Schema support false omits response_format and embeds shape; an actual unsupported-schema rejection retries once without it. Unsupported response_format is not every HTTP 400. Maintain a model capability set and small in-memory catalogue cache. Order candidate IDs without duplicates, sorting other vendors ahead of selected-vendor siblings.
  ```rust
  let deadline=tokio::time::Instant::now()+request.timeout;
  let remaining=deadline.saturating_duration_since(tokio::time::Instant::now());
  let response=tokio::time::timeout(remaining, self.http.post(url).json(&body).send()).await;
  ```
  Bound body reads and catalogue requests with the same deadline, not only TCP connect/send. A failed rescue lookup returns the last model error. `Generated.model_used` retains the validated attempted candidate ID for later batch preference; an untrusted response.model label must never become a request ID (e.g. a canonical paid alias returned for a free request). If useful, log the reported label separately with sanitized diagnostics. No backoff loop, paid fallback or silent move to local.
- [ ] **Green:** All scripted failures pass offline with self-contained dummy credentials; do not require ambient real keys or unsafe global env mutation. Retain temporary image directories through each test. Bound server accept/read/cleanup and surface unexpected paths. The body-deadline test proves an actual request arrived and the intended deadline expired, not merely any early auth/file error. Test a response.model alias differing from the attempted free/zero-priced ID and prove subsequent preference stays on the safe attempted ID. Verify logs and returned/displayed errors name useful failures without credentials/image data: add a synthetic upstream-error echo containing a dummy key/data URL and ensure diagnostics redact it instead of returning the entire raw body. Keep failure/status/schema-rejection meaning after sanitization. Default test suite makes no Internet calls. Keep a separate ignored/explicit live test, never required to approve this port.
- [ ] **Commit:** `git add src/metadata/openrouter.rs src/metadata/mod.rs tests/openrouter.rs && git commit -m "feat(ai): port OpenRouter vision and model recovery"`.

### Task 12: Local Ollama, llmfit and installation consent

**Files:** Create `src/metadata/local.rs`, `tests/local_metadata.rs`; modify `src/metadata/mod.rs`. Narrowly extend `src/metadata/openrouter.rs` and `tests/openrouter.rs` for the parent-noted clipped-data-URI diagnostic regression; do not otherwise reopen Task11.

**Interfaces:** `Fit {fit: bool, level: String, reason: String}`; `LocalStatus {fit: Fit, installed: bool, available: bool}`; async MetadataClient methods `local_fit(&self, model:&ModelOption)->Result<Fit>`, `local_status(&self, model: &ModelOption, deadline: tokio::time::Instant) -> Result<LocalStatus>`, `prepare_local(&self, model: &ModelOption, install: bool, deadline: tokio::time::Instant) -> Result<()>`, `local_generate(&self, image: &PreparedImage, request: &GenerationRequest) -> Result<Generated>`; `consent_required(status:&LocalStatus)->bool`. Complete MetadataClient::generate with a two-variant match dispatching to openrouter_generate/local_generate. CLI/UI provide consent; provider code never reads stdin.

- [ ] **Red:** Fake llmfit executable reports Good/Too Tight/unavailable/malformed; vision-models needs no Ollama service; tags/pull/chat wire bodies; missing-model consent false causes no pull; pull errors do not claim installed; chat rejects malformed/empty metadata. Deadline includes local status/pull/generation so no unbounded accepted job.
  ```rust
  #[tokio::test]
  async fn model_is_not_pulled_without_consent() {
      let server=Server::start(vec![Reply::json("/api/tags",200,serde_json::json!({"models":[]}))]);
      let client=local_client_for_test(&server); // file-local helper constructs MetadataClient with fake llmfit Good.
      assert!(client.prepare_local(&model_option(),false,tokio::time::Instant::now()+Duration::from_secs(2)).await.is_err());
      assert!(server.requests().iter().all(|r| r.path != "/api/pull"));
  }
  ```
  Define the two file-local helpers in this test file using the shared constructor and existing catalogue row; they are not production APIs.
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test local_metadata`.
- [ ] **Implement:** Spawn llmfit with timeout; only Good fits. GET tags; POST pull `{model,stream:false}` only after consent; require success status; POST chat with base64 images, shared schema `format`, `stream:false`; parse `message.content`. Do not spawn Ollama or implement hardware heuristics.
  ```rust
  let body=serde_json::json!({"model":model.ollama_name,"stream":false,
      "format":title_description_schema(),
      "messages":[{"role":"user","content":metadata_prompt(),"images":[encoded]}]});
  ```
- [ ] **Green:** Local/metadata/OpenRouter tests; confirm source and model names remain explicit, no OpenRouter call on local failures. Close `task-12-parent-note.md`: a clipped upstream data:image/... echo must not bypass complete-string masking and expose an image fragment; add synthetic loopback coverage and a minimal formatter fix while retaining bounded, useful HTTP/schema diagnostics. Local errors must not echo known image content. Use only disposable art/dummy credentials/fake llmfit; no new public diagnostic framework or live provider calls.
- [ ] **Commit:** `git add src/metadata/local.rs src/metadata/mod.rs src/metadata/openrouter.rs tests/local_metadata.rs tests/openrouter.rs && git commit -m "feat(ai): port Ollama generation and host-fit consent"`.

### Task 13: Application admission, single generation and exact-marker batches

**Files:** Create `src/application.rs`, `tests/application_ai.rs`; modify exports.

**Interfaces:** `Application::new(store: Arc<Store>, metadata: MetadataClient, clock: fn()->i64) -> Self`; `mutate<R>(&self, uuid:Option<&str>, f:impl FnOnce(&Store)->Result<R>)->Result<R>` guards edits during publishing/UUID AI; `generate_one(&self,uuid:&str,request:GenerationRequest)->Result<Generated>`; `generate_marked(&self,request)->Result<BatchReport>`; `BatchReport {succeeded,failed:usize, results:Vec<RecordResult>}`; `RecordResult {uuid:String,saved:bool,error:Option<String>}`. Private RAII leases release busy UUIDs even after persistence failure. No long-held operation/Store locks across awaits.

- [ ] **Red:** Reproduce admission-vs-manual-edit race with barriers, duplicate generation rejection until save completes, unrelated draft allowed, illegal phase/model/path produces no markers, accepted single failure writes both markers, batch `[3,2,1]` OpenRouter vs `[1,2,3]` local, continue failure, marker-only stale-field merge and learned model preference.
  ```rust
  // Unit test inside application.rs, where the private batch_ids helper is visible.
  #[cfg(test)]
  mod ordering_tests {
      use super::{batch_ids, Source};
      #[test]
      fn batch_order_is_source_specific() {
          let ids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
          assert_eq!(batch_ids(ids.clone(), Source::OpenRouter), vec!["3", "2", "1"]);
          assert_eq!(batch_ids(ids, Source::Local), vec!["1", "2", "3"]);
      }
  }
  ```
  Define private `batch_ids(ids: Vec<String>, source: Source) -> Vec<String>` in application.rs. Integration tests additionally assert actual saved result ordering with real valid fixture UUIDs, not just this helper.
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test application_ai`.
- [ ] **Implement:** Under operation mutex reread eligibility then reserve the exact still-eligible UUID set; release before thumbnails/provider/persistence. Single requests call save-generated-pair or failure-marker operation; batches call merge-only-markers and retain existing markers on errors. Save independently, continue ordinary known failures, and report persistence failure distinctly. Propagate Error::DurabilityUncertain typed and stop; never flatten it into an ordinary failed-record String. Learn the preferred model only from the FIRST valid answering pair, before its Store save, retaining it even if later records need fallback or its save fails.
  ```rust
  let mut ids=eligible_ids;
  if request.source==Source::OpenRouter { ids.reverse(); }
  for uuid in ids {
      // Fresh artwork lookup and provider call outside locks; then exact-marker Store merge.
      // Record this UUID's actual save result before continuing to the next one.
  }
  ```
  Implement these loop operations, not a second UI-only batch. Generation parsing/validation/installation refusal occurs before accepted side effects; a real provider failure after acceptance is persisted as specified.
- [ ] **Green:** AI application and workflow/provider tests; blocked provider does not block unrelated Store writes; all RAII leases clear on every error/return path.
- [ ] **Commit:** `git add src/application.rs src/lib.rs tests/application_ai.rs && git commit -m "feat(app): share AI admission and retryable batch operations"`.

### Task 14: Extract Python browser worker with explicit v1 protocol

**Files:** Create `python/publicator_da/{__init__,browser,worker,pw_daemon}.py`, `src/publish_protocol.rs`, `tests/python/test_{browser,worker}.py`, `tests/publish_protocol.rs`; modify `src/lib.rs`, `pyproject.toml` test discovery and static publishing skill path instructions only as needed. Leave the old Python publisher executable until cutover. Narrow extra follow-through authorized by `task-14-application-followup.md`: modify `src/application.rs` and `tests/application_ai.rs` (or private unit tests) to retain the first valid model preference and propagate typed uncertain commits; do not redo the accepted Task13 concurrency fix.

**Interfaces:** Python `BrowserSession(open_command)`, `submit(job)->dict`, `close()`; `worker.serve(stdin,stdout,session_factory=BrowserSession)->int`. Rust serde tagged `WorkerCommand`/`WorkerEvent` use explicit `protocol:1`. One bounded JSON line ≤1 MiB. Exact messages:

```json
{"protocol":1,"type":"open","login_profile":"/data/.deviantart-login","work_profile":"/data/.deviantart-session","headless":false,"timezone":"Europe/Paris"}
{"protocol":1,"type":"ready"}
{"protocol":1,"type":"submit","attempt_id":"00000000-0000-4000-8000-000000000999","uuid":"00000000-0000-4000-8000-000000000001","image_path":"/data/.publicator/publish/00000000-0000-4000-8000-000000000999/upload.png","title":"Title","description":"Story","tags":["art"],"schedule":{"epoch":1800039600,"timezone":"Europe/Paris","year":2027,"month":1,"day":15,"hour":20,"minute":0},"price":0,"tiers":["Gold"],"galleries":["Featured"]}
{"protocol":1,"type":"scheduled","attempt_id":"00000000-0000-4000-8000-000000000999","uuid":"00000000-0000-4000-8000-000000000001"}
{"protocol":1,"type":"close"}
```

The example epoch matches 2027-01-15 20:00 Europe/Paris (19:00 UTC). `failed`/`unknown` include matching IDs plus `error`; `fatal` is a session error. Optional actual `url` on scheduled is emitted only if actually observed. Missing price means no premium toggle; zero is distinct. `PublishJob` has `protocol: u8`, attempt_id/uuid/title/description as String, image_path as PathBuf, tags/tiers/galleries as Vec<String>, schedule as ScheduleParts and price as Option<f64>. `ScheduleParts` has epoch:i64, timezone:String, year:i32 and month/day/hour/minute:u32. `SessionOpen` holds protocol, login_profile/work_profile:PathBuf, headless:bool and timezone:String. Use internally tagged `WorkerCommand::Open(SessionOpen) | Submit(PublishJob) | Close {protocol:u8}`. WorkerEvent's Scheduled/Failed/Unknown struct variants include protocol:u8, attempt_id/uuid:String and url:Option<String> or error:String; Ready/Fatal are session variants. Reject versions other than 1 after decoding. A close command requires no artwork result. Separately, `python -m publicator_da.worker --check-steps --skill PATH` is the explicit non-browser drift-check mode invoked by Rust check_steps; ordinary worker stdout is protocol only.

- [ ] **Red:** Port all browser-step optional no-op, cookie/session-copy/headless and STEPS tests; inject fake BrowserSession through `serve` for protocol tests. Reject wrong version/fields/IDs/types/WebP/nonzero minutes before browser mutation; log noise never stdout. Verify pre-final-click failure→failed, post-final-click exception→unknown and no next submit is processed after either terminal error.
  ```python
  def test_worker_never_reads_publication_database(tmp_path, monkeypatch):
      from publicator_da import worker
      import io
      forbidden = {"publications.json", "publicator.toml"}
      original = open
      def guarded(path, *args, **kwargs):
          assert str(path).rsplit("/", 1)[-1] not in forbidden
          return original(path, *args, **kwargs)
      monkeypatch.setattr("builtins.open", guarded)
      # Valid open/submit/close commands are built by a local protocol fixture.
      assert worker.serve(io.StringIO(valid_commands(tmp_path)), io.StringIO(), FakeSession) == 0
  ```
  Define `valid_commands` and `FakeSession` in `tests/python/test_worker.py`; FakeSession returns matching IDs, does no I/O. Also test the real subprocess framing with a test-only entry script injecting this fake session, not a production `--fake` switch.
- [ ] **Run red:** `nix develop -c env PYTHONPATH=python pytest tests/python -q` and `nix develop -c cargo test --no-default-features --test publish_protocol`.
- [ ] **Implement:** Copy retained selectors/session/cookies/STEPS from `deviantart.py`; remove imports of Store/workflow/entries/config/llm. Tags and structured schedule come from the job. Stop using `date` subprocess/locale parsing. Keep real login profile read-only and use a disposable working copy; preserve cookie reinjection/site-side auth verification. Wrap final Schedule click with an irreversible-attempt flag set BEFORE calling click; exceptions afterward are unknown. A completed existing flow yields scheduled without pretending to prove future public appearance.
  ```python
  try:
      submit_entry(page, job, before_commit=lambda: state.__setitem__("commit_attempted", True))
      return event("scheduled", job)
  except Exception as exc:
      return event("unknown" if state["commit_attempted"] else "failed", job, error=str(exc))
  ```
  Include normal type validation, newline+flush emission, bounded reads and teardown in finally. Session validation uses timezone_id and requires canonical source/work profiles to be disjoint trees (neither equal nor an ancestor of the other), so working-copy cleanup cannot erase the source profile. `check-steps` resolves the packaged skill from an explicit asset path and fails if missing.
- [ ] **Green:** Python browser/protocol tests and Rust framing tests pass offline; test AST/import audit excludes app modules and publication/TOML reads. Preserve the DOM prober as explicit Python Playwright tooling, not part of the job executor. Also close the named application follow-through: three-record actual preference B→(fallback C)→still B, plus production result/control-path coverage proving Error::DurabilityUncertain stays typed, stops the batch and releases leases. Ordinary known failures still continue. Include all application follow-through in this task's full original-BASE review package.
- [ ] **Commit:** `git add python src/publish_protocol.rs src/lib.rs src/application.rs tests/application_ai.rs tests/python tests/publish_protocol.rs pyproject.toml .claude/skills/publish-deviantart/SKILL.md && git commit -m "refactor(publish): isolate Python Playwright worker"`.

### Task 15: Durable publishing intents, receipts and reconciliation

**Files:** Create `src/publish_journal.rs`, `tests/publish_journal.rs`; refactor the existing row-level delivery transition in `src/workflow.rs` only as needed to reuse it inside one guarded Store mutation. Narrowly adjust `src/store.rs`/`src/config.rs` and their tests as needed so existing JSON symlink aliases and journal database identity agree with actual reads/writes/locks.

**Interfaces:** `ApprovedSnapshot` derives Clone/Serialize/Deserialize/PartialEq with database_path:PathBuf, uuid/platform/title/description/source_hash:String, schedule_epoch:i64, tags/tiers/galleries:Vec<String>, price:Option<f64>; `snapshot(store: &Store, uuid: &str)->Result<ApprovedSnapshot>` rereads fresh approved data/config/art; `PublishJournal::new(data_dir:&Path) -> Result<Self>`; methods `begin(&self, job:&PublishJob, approved:&ApprovedSnapshot) -> Result<()>`, `terminal(&self, event:&WorkerEvent) -> Result<()>`, `apply_scheduled(&self, store:&Store, attempt_id:&str) -> Result<()>`, `applied(&self, attempt_id:&str) -> Result<()>`, `recover(&self, store:&Store)->Result<RecoveryReport>`, `reconcile(&self, store:&Store, attempt_id:&str, outcome:ReconcileOutcome)->Result<()>`. `ReconcileOutcome::Scheduled {actual_url:Option<String>} | NotSubmitted`; `RecoveryReport {replayed:Vec<String>,blocked:Vec<UnresolvedAttempt>}`; `UnresolvedAttempt {attempt_id:String,uuid:String,reason:String,approved:ApprovedSnapshot}`. Pending/unknown/changed fingerprints block automatic retry for that UUID.

- [ ] **Red:** Crash after intent; scheduled receipt before Store save; Store save failure; crash after Store save before applied marker; changed content; invalid receipt IDs; repeated replay/reconcile; not-submitted unlock; no URL invention. Deterministically interleave an approved-field change after preliminary verification but before commit admission: the fresh in-transaction guard must reject it and retain the receipt. Unrelated-row edits must survive without spuriously blocking this target. Explicit reconciliation while another process holds the publisher lock returns Busy with unchanged journal/Store; it must not release or rewrite a still-active intent. Empty/malformed journals are errors, never deleted as if no intent existed. Add temporary JSON-symlink/target and shared-lock tests so receipt identity cannot name one database while mutation replaces the alias itself. Preserve truly missing JSON and data-dir config locality. Test newly-created journal directory entry fsync ordering/failures before dispatch.
  ```rust
  #[test]
  fn success_receipt_replay_does_not_need_a_browser() {
      let f=publish_fixture(); // local helper: valid DESCRIBED DA target and real image/config tags.
      let (job,snap)=job_for(&f); let j=PublishJournal::new(f.root.path()).unwrap();
      j.begin(&job,&snap).unwrap(); j.terminal(&scheduled(&job)).unwrap();
      assert_eq!(j.recover(&f.store).unwrap().replayed,vec![job.attempt_id.clone()]);
      assert!(j.recover(&f.store).unwrap().replayed.is_empty());
      assert_eq!(f.read()[0]["apparitions"][0]["state"],"published_or_scheduled");
  }
  ```
  Define `publish_fixture`, `job_for`, `scheduled` as file-local helpers using the protocol/snapshot APIs, with no worker process at all.
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test publish_journal`.
- [ ] **Implement:** Per-attempt JSON beneath `.publicator/publish/<attempt_id>/receipt.json`, atomically replaced and file+parent-directory fsynced at each boundary. Make newly-created journal directory ancestry durable too: sync each created directory entry's parent up to the existing data_dir boundary before begin can succeed or a job can be sent. Fingerprint canonical approved-content/options/source hash, not temporary upload path, delivery state or URL (those change when recording success). Bind each intent to the resolved target database path as well as artwork/platform/content; recovery must never replay another --json database's attempt into the current one. Canonical identity must govern the Store's real reads/writes/lock, not just be a snapshot label: an existing JSON-file symlink must update its real target without replacing the link, or fail explicitly before remote work if unsupported. Resolve/reject dangling/cyclic aliases explicitly; preserve missing-file-as-empty and config/tags relative to data_dir. See task-15-parent-ruling.md for the concrete alias-replacement failure. Use pending→terminal→applied; terminal safe-failed/not-submitted may allow retry, unknown/pending may not. Persist receipt BEFORE Store recording.
  ```rust
  journal.terminal(&event)?;
  if matches!(event, WorkerEvent::Scheduled { .. }) {
      journal.apply_scheduled(store, &attempt_id)?;
  }
  // apply_scheduled loads the durable receipt, validates identity/database binding,
  // then checks the current target's approved fields AND applies its delivery
  // transition inside ONE fresh Store::mutate. Only after success mark applied.
  // Reuse it from recover/reconcile; never snapshot/read then independently save.
  ```
  Recovery is idempotent and never launches Python. `apply_scheduled` owns the guarded receipt application reused by recovery, reconciliation and Task16. Current approval/target fields and the delivery transition must be checked/applied under the same short Store mutation lock; a separate `snapshot` followed by `record_delivery` has a TOCTOU gap and is forbidden. Reuse the row-level workflow transition rather than nesting Store mutations. Full image hashing and other expensive I/O remain outside that lock: validate those inputs first, then compare the relevant fresh row fields under lock before applying. Preserve unrelated rows. On DurabilityUncertain, retain the receipt and stop without writing an applied marker or blindly retrying. Scheduled reconciliation with a changed fingerprint must not certify newer metadata as the submitted content: reject with stored/current comparison for human correction, never overwrite shared content automatically. Explicit NotSubmitted can release an old unknown attempt after human confirmation. `reconcile` acquires the same nonblocking `.publicator/publish.lock` used by Task16 and holds it through reconciliation; refuse Busy if a browser batch is active in another UI/CLI process. Introduce the small shared lock helper here and reuse it in Task16. Recovery/guarded application called under an already-held publisher lock must not recursively reacquire it. Reject unsafe attempt-ID paths.
- [ ] **Green:** All journal/workflow tests; simulate each durability boundary deterministically, including repeated already-recorded success with a historical pending platform.
- [ ] **Commit:** `git add src/publish_journal.rs src/workflow.rs src/store.rs src/config.rs src/lib.rs tests/publish_journal.rs tests/store.rs tests/config.rs && git commit -m "feat(publish): persist recoverable delivery receipts"`.

### Task 16: Rust publisher preflight and sequential worker lifecycle

**Files:** Create `src/publishing.rs`, `tests/publishing.rs`, `tests/support/fake-worker.sh`; extend `src/application.rs`. Narrowly adjust `src/publish_protocol.rs`/`tests/publish_protocol.rs` if needed for configured-timezone/fold=0 parity; no protocol format change.

**Interfaces:** `PublishRequest {uuids:Option<Vec<String>>, headless:bool}`; `PublishReport {published:usize,failed:usize,unknown:usize,unattempted:usize,error:Option<String>}`; `Publisher::new(worker:PathBuf,skill:PathBuf) -> Self`; async `publish(&self,app:&Application,request:PublishRequest,cancel:Arc<AtomicBool>)->Result<PublishReport>` and `check_steps(&self)->Result<()>`. Add `Application::with_publisher(self, publisher: Publisher) -> Self` before wrapping it in Arc and async `Application::publish(&self, request: PublishRequest, cancel: Arc<AtomicBool>) -> Result<PublishReport>`; an unconfigured publisher is an explicit error. Private transport functions: async `send_line(writer: &mut (impl tokio::io::AsyncWrite + Unpin), command: &WorkerCommand) -> Result<()>` and `read_matching_terminal(reader: &mut (impl tokio::io::AsyncBufRead + Unpin), job: &PublishJob, deadline: tokio::time::Instant) -> Result<WorkerEvent>`. Application acquires/releases publishing busy state and refuses new publish while its accepted AI saves are in flight.

- [ ] **Red:** Historical-only no-op; all batch/tag/image/options preflight before spawn; stale approval rejected; genuine mixed-case source WebP converted to a valid non-WebP worker upload; unsupported media rejected; configured non-Paris timezone and fold=0 epoch/wall-time consistency; headless forwarding; success→save→next ordering; first safe failure/unknown stops; stderr pressure; wrong version/ID/order/duplicate/overlong JSON; session crash; job timeout; cancel-after-current; journal save failure; another UI/CLI process holds publish lock; success persistence failure retains receipt. No pending/unknown intent is resent. Port the registered-cell remaining/retry expectation from `tests/test_server.py::test_selections_all_options_success_and_retry_restore` (around line 570): with two selected DA targets, a first success and a second safe failure leave exactly the failed DA cell pending; restart/retry submits only that cell, and a subsequent repeat is a no-op. Add a historical unregistered pending apparition to the successful row to prove it neither inflates executable remaining counts nor causes a successful DA cell to be resent.
  ```sh
  # tests/support/fake-worker.sh, success fixture; variants are generated per test in its tempdir.
  while IFS= read -r line; do
    kind=$(printf '%s' "$line" | jq -r .type)
    case "$kind" in
      open) printf '%s\n' '{"protocol":1,"type":"ready"}' ;;
      submit) printf '%s' "$line" | jq -c '{protocol:1,type:"scheduled",attempt_id,uuid}' ;;
      close) exit 0 ;;
      *) exit 2 ;;
    esac
  done
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test publishing` (jq/coreutils are dev-only fixture dependencies).
- [ ] **Implement:** Nonblocking `.publicator/publish.lock` flock across the browser batch; not the Store lock. Recover before target selection. Resolve only registered pending DESCRIBED targets, validate entire selected batch and exact art identity/tags/schedule representation before open, then reread/fingerprint before each send. Reuse images::prepare_upload for supported WebP sources rather than rejecting that existing feature; never send raw WebP. Keep staging separate from the new receipt directory owned by journal.begin. Derive SessionOpen and job schedule timezone/wall parts from configured scheduling rules, not hardcoded Paris (Paris is only the default), retaining core fold=0 parity. apply_scheduled already persists the applied marker; do not write it a second time. Read task-16-parent-ruling.md for the named interface risks. Start one Python process with piped stdin/stdout/stderr and isolated Linux process group; drain diagnostics concurrently with bounded retained logs. Ready deadline 90 s, per-artwork deadline 600 s, graceful cleanup 5 s then terminate the process group. Each exchange deadline includes writing the open/submit frame as well as reading the reply; close writes are bounded too. Test a non-reading worker with a large valid frame so write_all cannot hang outside the deadline. These are named constants with a `ponytail:` ceiling comment, not per-widget magic values.
  ```rust
  journal.begin(&job,&approved)?;       // durable before any possible effect
  send_line(&mut stdin,&WorkerCommand::Submit(job.clone())).await?;
  let event=read_matching_terminal(&mut stdout,&job,deadline).await?;
  journal.terminal(&event)?;            // durable receipt before local state
  // For Scheduled, journal.apply_scheduled(app.store(), &job.attempt_id)
  // performs the atomic fingerprint guard + delivery transition and applied marker;
  // only then dispatch another job. Use the actual Application Store accessor.
  ```
  If a sent job loses its event, keep intent unresolved/unknown; never turn it into safe-failed based on exit code. After terminal errors send close/cleanup and leave unsent jobs unattempted. Cancellation only prevents the next send. On pre-send revalidation failure, mark that record unattempted, not remotely failed. Counts include only registered executable targets.
- [ ] **Green:** Publisher/journal/Python tests and all core tests; inspect fake-worker call log to prove success persistence precedes next artwork and no Python receives TOML/database paths. Explicitly close the relocated Task 7 remaining-cell obligation using the real publisher selection path, saved Store state/report and restart/retry worker call log—not a test-only alternative selector. Preserve per-cell identities, successful-cell exclusion and registered-only counts; Task 21 (Native Publish, recovery, Calendar) must retain the resulting saved selection/remaining behavior in the native UI; Task 20 owns Describe, not publishing.
- [ ] **Commit:** `git add src/publishing.rs src/application.rs src/publish_protocol.rs src/lib.rs tests/publishing.rs tests/publish_protocol.rs tests/support/fake-worker.sh && git commit -m "feat(publish): coordinate validated worker batches in Rust"`.

### Task 17: Port CLI operations and preserve exit/consent semantics

**Files:** Modify `src/bin/publicator.rs`; create `tests/cli.rs`; add new Rust packages/internal test wrappers in `flake.nix` without switching existing app names yet. Narrowly adjust `src/application.rs`/its tests if needed to preserve an empty marked batch as a no-service/no-consent no-op (`task-17-parent-note.md`).

**Interfaces:** Subcommands `validate`, `echo-first`, `generate-meta`, `vision-models`, `prompt-audit`, `da-publish`, `check-steps`. Existing app wrappers eventually prepend their subcommand. `da-publish` keeps mutually exclusive `--uuid`/`--all`, default first, `--headless`, `--check-steps`, data-dir/json and verbosity. Add mutually exclusive `--reconcile-attempt ID --outcome scheduled|not-submitted [--url URL]` for human receipt reconciliation; never mix reconciliation with normal submission flags. UI is a separate binary.

- [ ] **Red:** Run compiled CLI from an unrelated temporary CWD; verify default first eligible target versus --all versus --uuid using fake-worker call logs, check-only --check-steps cannot fall through for --all/--uuid, preserved output/exit statuses (including echo-first no-match error), missing-data errors, `--json` config locality, all-flag exclusivity, unknown source/Claude rejection, no implicit AI source, local model required, Enter/default-yes, y/yes, explicit n/N/no/NO decline, invalid-answer refusal, EOF, -y and non-TTY consent cases with no markers written after refusal. A valid empty marked local batch succeeds without local-provider/model setup or consent even when fake llmfit/Ollama are unavailable; preserve basic argument/model validation and own this no-work behavior in the shared Application path. `vision-models` must not call Ollama. `echo-first` prints image/title/configured-zone schedule without host-locale `date`.
  ```rust
  #[test]
  fn removed_provider_is_an_explicit_cli_error() {
      let status=std::process::Command::new(env!("CARGO_BIN_EXE_publicator"))
          .args(["generate-meta","--source","claude"]).status().unwrap();
      assert!(!status.success());
  }
  ```
- [ ] **Run red:** `nix develop -c cargo test --no-default-features --test cli`.
- [ ] **Implement:** Clap is argument parsing only. Operations call the shared library/application. Generate-meta retains publication-driven exact-marker selection, default OpenRouter catalogue model and optional timeout; never accept image arguments as a new interface. Default CLI publication means the first eligible pending DA target, not Publisher's None=all batch semantics. Preserve --check-steps as check-only/then-exit even with legacy --all/--uuid combinations; reject check-only plus reconciliation. Preserve the -y short alias. Read task-17-parent-findings.md for the named source bugs and regressions. Prompt local installation `[Y/n]` once per batch; `stdin.is_terminal()` + no `-y` is failure before generation/persistence. Publish 0/0 no-op exits success, any failed/unknown/unresolved result exits nonzero; explicit missing UUID remains an error.
  ```rust
  let allow_install = if args.yes { true } else if std::io::stdin().is_terminal() {
      eprint!("Install {}? [Y/n] ", model.ollama_name);
      let mut answer=String::new();
      let read=std::io::stdin().read_line(&mut answer)?;
      read != 0 && matches!(answer.trim().to_ascii_lowercase().as_str(), "" | "y" | "yes")
  } else { false };
  ```
  Reject EOF/refusal/unrecognized answers without mutation; only Enter (the displayed default), y/yes or -y authorizes installation. In particular 'no' must never become consent merely because it differs from 'n'. No prompting if nothing eligible or model already installed. Keep app options distinct rather than mechanically giving every command every flag.
- [ ] **Green:** CLI plus core/provider/publishing tests, with actual terminal/non-terminal consent IO exercised through a disposable PTY or genuinely equivalent production-path evidence (not only pure parser strings). Test installed/missing/empty work, EOF/refusal unchanged data/no pull/chat and fake accepted requests; no production fake-mode switch or real local service. Build `nix build .#publicator-core`; smoke validate/check-steps/prompt-audit against a disposable external directory. Existing Python apps remain runnable until the final gate.
- [ ] **Commit:** `git add src/bin/publicator.rs tests/cli.rs flake.nix && git commit -m "feat(cli): port publication tools to shared Rust operations"`.

## Milestone C — native Iced UI

### Task 18: Native application shell and revision-safe drafts

**Files:** Create `src/bin/publicator-ui.rs`, `src/ui/{mod,drafts}.rs`, `tests/ui_drafts.rs`; add optional desktop manifest declarations/exports and Nix desktop dependencies.

**Interfaces:** `ui::run(application:Arc<Application>, options:UiOptions)->iced::Result`; `UiOptions {openrouter_model:Option<String>, ai_timeout:Duration}`. `App` owns active `Tab::{Pick,Schedule,Describe,Publish,Calendar}`, fresh publication snapshot, search authorization, draft map, busy/result display and pending navigation intent. `Draft` stores title/description field text, each field's revision/dirty state, last edit time and last sent values. `drafts::edit`, `due`, `begin_save`, `saved`, `failed`, `is_dirty` are pure methods tested outside rendering. Messages include Edited, Tick, SaveFinished, Navigate, CloseRequested and LoadFinished; async task results carry UUID plus sent field revisions.

- [ ] **Red:** Typing revision2 while save1 completes retains revision2/dirty; title completion cannot clear newer description; failed save blocks tab/close/refresh/AI; 349ms no autosave, 350ms due; phase switch flushes; force discard is explicit; malformed JSON produces error screen not empty workflow.
  ```rust
  #[test]
  fn old_completion_keeps_newer_input_dirty() {
      let mut d=Draft::new("Old".into(),"D".into());
      d.edit(Field::Title,"First".into(),0); let sent=d.begin_save();
      d.edit(Field::Title,"Newer".into(),10); d.saved(&sent);
      assert_eq!(d.title(),"Newer"); assert!(d.is_dirty());
  }
  ```
  Introduce concrete `Field`, `Draft::new`, revision/tick integer units and `SentDraft` in `drafts.rs`; no Iced types in that module.
- [ ] **Run red:** `nix develop -c cargo test --features desktop --test ui_drafts`.
- [ ] **Implement:** Iced state/update/view with `Task`/`Subscription`. Use a 50ms dirty-only timer to detect the 350ms debounce; do not issue a network/save every tick. Serialize per-record saves and apply completion only to the sent field revisions. Flush all relevant drafts and await saves before dependent action; failed flush keeps current view and dirty state. Intercept native close rather than letting the window disappear first.
  ```rust
  if field.revision == sent_revision && field.text == sent_text {
      field.dirty = false;
  }
  // A completion never assigns field.text from its older request snapshot.
  ```
  `Task::perform` wraps async operations; blocking Store/image/Git work uses Tokio spawn_blocking. No sync provider/file scan in view(). Native close while publishing offers wait/cancel-after-current; force termination warns of unknown outcome.
- [ ] **Green:** Draft tests plus desktop compile and real empty-window smoke using Xvfb/software renderer. Keep Iced frontend free of HTTP server/HTML/Leptos dependencies.
- [ ] **Commit:** `git add Cargo.toml Cargo.lock src/bin/publicator-ui.rs src/ui src/lib.rs tests/ui_drafts.rs flake.nix && git commit -m "feat(ui): add native shell and safe draft lifecycle"`.

### Task 19: Native Pick and Schedule stories

**Files:** Create `src/ui/{pick,schedule}.rs`, `tests/ui_workflow.rs`; extend `src/ui/mod.rs`.

**Interfaces:** `pick::view(state:&App)->Element<'_,Message>` and `schedule::view(state:&App)->Element<'_,Message>`; messages route Search/Pick/Delete and date/approval/return operations through Application guards/workflow, not direct JSON writes. Core Candidate and DateEdit are the only file/date intent types.

- [ ] **Red:** Search defaults25/all configured paths; limit0 no candidates; stale authorization cannot delete; confirmed candidate-only deletion; pick moves to Schedule after saved; Custom retains exact input/profile absence; unavailable suggestion blocks approval; frozen date cannot edit; no prompt/delete/search controls on Schedule cards.
  ```rust
  #[test]
  fn custom_edit_persists_without_inventing_a_profile() {
      let f = Fixture::new(); f.write(serde_json::json!([row(1, "PICKED")]));
      edit_date(&f.store, &id(1), DateEdit::Custom("2027-01-01T19:30".into()),
          epoch("2026-12-01T00:00:00Z")).unwrap();
      let saved = f.read();
      assert_eq!(saved[0]["scheduleIsManual"], true);
      assert!(saved[0].get("scheduleProfile").is_none());
      assert_eq!(saved[0]["apparitionTimestampIfDifferentThanSubmission"],
          epoch("2027-01-01T18:30:00Z"));
  }
  ```
  Add UI message-dispatch/flush assertions around the same actual persistence behavior; an enum-shape-only assertion is not UI coverage.
- [ ] **Run red:** `nix develop -c cargo test --features desktop --test ui_workflow`.
- [ ] **Implement:** Built-in image/card/checkbox/text-input/pick-list/button widgets, read-only preset list and explicit Custom date/time text inputs validated by the core. Image thumbnails load lazily in background. Distinguish loading/error/missing preview. Configure labels, keyboard order and visible capacity/saved errors; no browser-clock calculation.
  ```rust
  let edit=DateEdit::Custom(format!("{}T{}",state.custom_date,state.custom_time));
  // Dispatch async Application::mutate(uuid, |store| workflow::edit_date(store,uuid,edit,now)).
  ```
- [ ] **Green:** UI workflow/core tests; exercise both views in the native acceptance fixture, including keyboard navigation/scroll and no-data startup. Capture browser-backed native-window evidence at Task22.
- [ ] **Commit:** `git add src/ui tests/ui_workflow.rs && git commit -m "feat(ui): port candidate picking and scheduling"`.

### Task 20: Native Describe, local consent and AI batches

**Files:** Create `src/ui/describe.rs`, `tests/ui_describe.rs`; extend `src/ui/mod.rs` and narrowly share demand-driven preview scheduling with Pick (`src/ui/pick.rs`, an optional small common preview helper, `tests/ui_workflow.rs`). Close `task-20-lazy-preview-followup.md` before acceptance.

**Interfaces:** `describe::view(state: &App)->Element<'_,Message>`; messages SaveDraft/ApproveMetadata/ReturnToSchedule/GenerateOne/GenerateMarked/ConfirmLocalInstall; model choice uses canonical Source+ModelOption, not arbitrary JSON.

- [ ] **Red:** Only local/OpenRouter shown; no Claude default or button; valid model selection; actual selected-model fit/reason and available/installed status; installed/no-install-prompt, missing/explicit-confirm, cancel/no mutation/no pull/chat, bad/unknown fit and unavailable/error refusal, stale selection/status/consent rejection, including batch membership changed during consent/before asynchronous admission; consent not inferred from clicking another control; dirty flush before AI; accepted busy UUID disables edit/approval/return until persistence completes; unrelated records remain editable; batch errors keep retryable cards; no prompt/search/delete on Describe cards; multi-result search does not launch every preview, later visible/needed range loads its cards and cached/in-flight requests deduplicate through the shared Pick/Describe preview boundary.
  ```rust
  #[test]
  fn source_options_do_not_include_a_third_provider() {
      assert_eq!(available_sources(), &[Source::OpenRouter, Source::Local]);
  }
  ```
  `available_sources` is the shared fixed two-item list introduced in Task10, used by UI/CLI validation. Test message dispatch with delayed fake provider, not just this list.
- [ ] **Run red:** `nix develop -c cargo test --features desktop --test ui_describe`.
- [ ] **Implement:** Close Task19's carried lazy-preview finding per task-20-lazy-preview-followup.md using a small shared visible/needed demand boundary, bounded preparation and the existing persistent cache, not reduced search limits or a new framework. Render shared pair edits, per-field draft status and two-source model selection. Expose actual selected-model local-fit verdict/reasons and Ollama available/installed state through the Application's injected status dependency before generation. Prompt only for a Good, available, missing model; installed models need no install prompt/pull and bad/unknown/unavailable/error cannot solicit misleading consent or start generation. Read task-20-local-status-followup.md. Keep exact Source/ModelOption/action and frozen batch UUID identity across asynchronous status/confirmation/admission (never rescan an unscoped new batch after confirmation), and explicit confirmation before constructing install_missing=true. Test actual returned Tasks, fake calls and persisted bytes; not pure flags/lists or an independent default widget client. Single/batch Task handlers invoke Application methods from Task13; busy state ends after result/markers are saved, not at HTTP response arrival.
  ```rust
  let request=GenerationRequest {source:selection.source,model:selection.model.clone(),
      timeout:options.ai_timeout,install_missing:user_confirmed_install};
  ```
- [ ] **Green:** Describe/draft/application AI and actual lazy preview operation tests (not only task unit counts); carried eager-preview finding must be closed before this gate. Task22 additionally verifies real native scrolling. Native fixture verifies per-card generation and batch reverse ordering by fake-provider call log and persisted rows, not UI labels alone.
- [ ] **Commit:** `git add src/ui src/metadata/mod.rs tests/ui_describe.rs tests/ui_workflow.rs && git commit -m "feat(ui): port metadata review and two-source generation"`.

### Task 21: Native Publish, recovery, Calendar and shared prompt panel

**Files:** Create `src/ui/{publish,calendar,prompt}.rs`, `tests/ui_publish_calendar.rs`; extend `src/ui/mod.rs`. Narrowly repair `src/publishing.rs` and its tests for the named typed-uncertainty/stderr boundary (`task-21-publisher-error-followup.md`).

**Interfaces:** Each view returns `Element<'_,Message>`; `prompt::view(match_:&PromptMatch)->Element<'_,Message>` is shared between candidate cards/calendar. Messages SelectCell/SelectAll/SaveOptions/PublishSelected/CancelAfterCurrent/ReconcileAttempt/OpenActualUrl/JumpToPublication. UI tri-state select-all derives from eligible pending registered cells.

- [ ] **Red:** Partial/full/none selection; plural tier/gallery checkboxes allow multiple; success cells locked; fake result failure/retry skips success; safe failed vs unknown vs unattempted displayed; unresolved receipt requires explicit reconcile; historical-only no-op; Store durability uncertainty plus worker stderr stays typed, retains recoverable receipt and stops before next send with fresh reread/report handling; investigate/close the captured parallel Publisher descendant-cleanup failure with bounded process-state evidence rather than serializing the suite; every apparition calendar event independent; missing image text event; Nearest warning and all candidate lineages; URL opens only actual stored HTTP(S); internal jump flushes and reveals correct phase.
  ```rust
  #[test]
  fn selection_is_partial_when_only_one_pending_cell_is_checked() {
      assert_eq!(selection_state(&[true,false]),SelectionState::Partial);
      assert_eq!(selection_state(&[]),SelectionState::None);
  }
  ```
  Define `SelectionState`/`selection_state` locally in publish.rs and test actual workflow selection mutations separately through Fixture.
- [ ] **Run red:** `nix develop -c cargo test --features desktop --test ui_publish_calendar`.
- [ ] **Implement:** Delegate all mutations to workflow/Publisher/journal APIs. Close task-21-publisher-error-followup.md: Publisher diagnostic decoration must not convert Error::DurabilityUncertain into Error::Protocol when stderr is present; retain semantic type and surface bounded diagnostics separately, with production-boundary evidence. No speculative error framework. ReconcileScheduled displays approved snapshot/current mismatch and optional real URL, never silently overwrites newer fields. Calendar uses core timeline/month projection and same prompt component as Pick. Exact text, explicitly labelled uncertain candidate histories and Unknown are separate widget branches.
  ```rust
  match match_ {
      PromptMatch::Exact(version) => exact_panel(version),
      PromptMatch::Nearest(candidates) => not_archived_panel(candidates),
      PromptMatch::Unknown(reason) => unknown_panel(reason),
  }
  ```
  These are private widget constructors in prompt.rs, not three new component frameworks. Plain text remains plain text, including placeholder-like strings and markup in prompts/titles/errors.
- [ ] **Green:** Native UI/core/publisher tests; native fixture uses fake worker scheduled/failed/unknown cases and verifies saved success/restart reconciliation with no browser publishing. Carry Task 16's relocated Task 7 obligation into the actual native Publish view: after partial success/safe failure and restart, selection/remaining derives from saved registered pending cells; retry skips every successful cell even when its publication still has an unregistered historical pending apparition. Historical apparitions remain visible in Calendar without inflating executable pending counts.
- [ ] **Commit:** `git add src/ui tests/ui_publish_calendar.rs && git commit -m "feat(ui): add publishing recovery and calendar views"`.

## Milestone D — native/Nix acceptance and cutover

### Task 22: Real native-window acceptance and packaged runtime assets

**Files:** Create `examples/ui_acceptance.rs`, `scripts/native-ui-smoke.sh`, `tests/packaging.rs`; modify `flake.nix`, `.gitignore` and test fixture modules as necessary. Do not switch the old Nix app aliases yet.

**Interfaces:** Finalize the `publicator-core` Nix package introduced in Task17 (no desktop feature, CLI bin), and add `publicator-desktop` (desktop feature, GUI bin). Worker is an executable wrapper with Python/Playwright environment; inject its absolute path and the static skill asset into Rust startup using the already-implemented `PUBLICATOR_WORKER`/`PUBLICATOR_SKILL` pair consistently in CLI/GUI wrappers and startup lookup; do not introduce mismatched parallel names. Read task-22-native-input-followup.md. Other shell-out dependencies come from wrapped PATH. Test example constructs the same `ui::run` application with disposable Fixture, HTTP fixtures and fake worker; no production fake-mode switch.

- [ ] **Red:** Packaged CLI/UI can locate schema/skill/worker when cwd is an external directory and source checkout is absent; missing skill fails check-steps; GUI closure has Linux windowing/graphics/software-renderer dependencies; CLI core build doesn't depend on Iced. Native shell test must fail if a required tool/window does not launch, not print pass because a screenshot file exists.
  ```sh
  nix build .#publicator-core .#publicator-desktop
  nix develop -c cargo test --features desktop --test packaging
  nix develop -c bash scripts/native-ui-smoke.sh
  ```
- [ ] **Implement:** Package with `rustPlatform.buildRustPackage` + cargoLock. Wrap shared CLI runtime with Git/ImageMagick/llmfit and the worker closure when publishing is used; wrap GUI with needed X11/Wayland/xkb/graphics libraries. Reqwest uses Rustls; OpenSSL is included for configured digest lookup. The test/dev shell, not either shipped package, owns pytest, jq, Xvfb, x11vnc, noVNC/websockify, Playwright test-browser tooling and window-control helpers.
- [ ] **Implement native harness:** Allocate a temporary directory/display and loopback ports; trap cleanup of fixture, X server, VNC and bridge processes. Start the acceptance example with an empty fixture, then history-only, then mixed-data scenarios. Print only the actual noVNC URL and artifact directory once ready. Example status/call logs and publications live in its disposable directory. Do not use the default data directory if fixture arguments are absent.
  ```sh
  work=$(mktemp -d /tmp/publicator-native.XXXXXX)
  trap 'kill "${bridge:-}" "${vnc:-}" "${app:-}" "${xvfb:-}" 2>/dev/null || true' EXIT
  # Bind noVNC/websockify to 127.0.0.1; x11vnc is inaccessible outside loopback.
  # Use the pinned Iced software backend documented for 0.14 in the Xvfb fixture.
  ```
  Complete the real process commands and readiness probes in the shell file during this task; the final harness contains no commented-out stand-ins for launch steps.
- [ ] **Native/browser verification:** Load playwright-cli skill at execution. Drive the actual noVNC canvas: all five tabs, visible card/image load, keyboard entry/focus (including real multiline description input, Unicode/markup literals and exact persisted internal newlines after navigation/reopen), resize/scroll (specific newly visible Pick/Describe cards load correct images, not just cache counts), date Custom and approval, metadata autosave race/failure, two-source selection/install refusal, tier/gallery multiselect, fake publish failure/retry/unknown reconcile, refresh/reopen and close-with-dirty behavior. Read fixture publication/call logs to prove changes; inspect browser console/bridge logs and native stderr. Record screenshots and assertion logs under `/tmp`, never Git. Do not claim headless reducer tests alone fulfill this step.
- [ ] **Green:** `nix develop -c cargo test --all-targets --features desktop`, Python worker suite, `nix flake check`, Nix core/desktop builds, native/browser acceptance, and all preserved non-effectful CLI smoke cases from an external temp data dir. Compile both normal GPU and software paths; no claim of real DA selector validation.
- [ ] **Commit:** `git add examples/ui_acceptance.rs scripts/native-ui-smoke.sh tests/packaging.rs tests/common flake.nix .gitignore && git commit -m "test(ui): verify native workflow and packaged assets"`.

### Task 23: Switch Nix apps, delete old application and remove Claude toolchain

**Files:** Modify `flake.nix`, `flake.lock`, `pyproject.toml`, `AGENTS.md`, `docs/README.md`, `docs/ROADMAP.md`, all affected `docs/features/*.md`, and ADR0009's supersession link; create `docs/adr/0010-rust-core-and-native-iced-interface.md`; delete `src/publicator/` after schema/browser assets have moved and remove replaced Python/HTTP/JS tests, including the old root `tests/conftest.py` which imports deleted publicator modules. Keep `tests/python/`, Rust tests and static publishing skill. This roadmap is the owner-authorized plan/checkpoint exception; do not commit separate execution reports or temporary fixtures.

**Interfaces:** Existing Nix app aliases now execute Rust CLI/GUI. `login` remains real Firefox shell wrapper; `pw-daemon` remains the explicit Python browser prober. pyproject contains browser-tool metadata/test configuration only; no Python llm/jsonschema runtime. Package names from Task22 are unchanged.

- [ ] **Red audit:** Build an exact old-test→new-test disposition list using the matrix below and the actual `pytest --collect-only` list. Every deleted Python assertion either has a Rust/Python-worker equivalent or is explicitly HTTP/DOM-only and no longer reachable. A source audit must fail while old Claude/llm integration remains.
  ```sh
  rg -n 'claude-code|llm-claude-cli|llm-openrouter|ps\.llm|claude_model|DEFAULT_MODEL' \
      flake.nix pyproject.toml src python tests
  ```
  Before deletion this should find the legacy implementation; afterward production/dependency matches must be absent. Negative tests intentionally mentioning the rejected `claude` source and historical ADR text are not runtime dependencies.
- [ ] **Switch:** Remove the Claude input, vendored plugin/overlay used only by llm, llm/OpenRouter Python packages and obsolete Python app wrappers; regenerate the lock with only necessary Nix input changes. Route each preserved app alias to its new subcommand/binary. Preserve runtime Git/ImageMagick/llmfit and Playwright browser pins. Do not remove the static publishing skill or rename data paths as cosmetic cleanup.
  ```nix
  # app wrapper body, with the matching Rust subcommand supplied by each alias:
  exec ${core}/bin/publicator validate "$@"
  # ui invokes the desktop binary, never a browser/HTTP server.
  ```
- [ ] **Delete/update:** Remove all old `src/publicator` application code, old HTML/JS renderers and their obsolete interface tests only after the disposition audit and Task22 pass. Update feature Does/Run/Code/Tests/Config/Data/Decisions/Verify blocks and actual maintained docs. Use writing-adrs for cross-cutting ADR0010 and mark ADR0009 superseded without reusing its number or erasing history. Remove the old tests/conftest.py imports/autouse globals fixture; any remaining browser fixtures live under tests/python and import only publicator_da. Set pyproject pytest paths to tests/python and pythonpath to python only at cutover. Document `.publicator/`, persistent locks, `.thumbs/`, browser profiles and unknown-outcome reconciliation. The code repo's ignore rules are not a substitute for documenting ignores needed in the separate data repo; do not edit Art's gitignore unasked.
- [ ] **Simplify then ponytail-review:** Inspect the actual full diff, reuse direct modules/built-in widgets/stdlib, remove speculative abstractions, duplicate provider/operation paths or long-lived compatibility bridges. Do not delete durability, validation, accessibility or executable regression checks. Apply concrete correctness/simplicity findings and rerun affected tests.
- [ ] **Final green:** `nix develop -c cargo fmt --check`; `nix develop -c cargo clippy --all-targets --features desktop -- -D warnings`; `nix develop -c cargo test --all-targets --features desktop`; `nix develop -c env PYTHONPATH=python pytest tests/python -q`; `nix flake check`; `nix build .#publicator-core .#publicator-desktop`; `nix run .#check-steps`; native/browser acceptance; preserved CLI smoke checks in disposable external data. Inspect `nix path-info -r` closures to confirm Claude/llm removal. Native acceptance must pass again after the final wrappers switch.
- [ ] **Commit:** `git add -A && git diff --cached --stat` and inspect every deletion; then `git commit -m "refactor!: switch Publicator to Rust and Iced"` with a body explaining removal of HTTP/Claude flags, unchanged data contracts and retained Python browser boundary. The owner's subsequent conditional instruction authorizes PARENT-ONLY fast-forward integration of every reviewed feature commit into master and a normal push AFTER all tasks, final native/package acceptance, whole-branch review/fixes and fresh verification pass; see sdd/final-integration-authorization.md. The Task23 worker must not integrate/push before its independent gate and parent synthesis. No force push, tag/release creation or real publishing is authorized.

## Coverage/disposition matrix

| Existing source/tests | New gate(s) |
|---|---|
| config/schema, `test_config.py` | Tasks1–2 `tests/config.rs`, `tests/store.rs`; prompt grammar/digest variants |
| `store.py`, `test_store.py` | Task2 Store atomicity/fresh reads/locks; Tasks4–7 persisted state |
| `workflow.py`, `test_workflow.py` | Tasks5–7; all explicit approvals, backwards locks, defaults and Custom cases |
| `scheduling.py`, `test_scheduling.py` | Tasks4–5 timezone/capacity/horizon; additional gap/fold tests |
| `images.py`, `test_images.py` | Task3 dedup/cache/authorization/real resize; Task22 lazy native previews |
| `migration.py`, `test_migration.py` | Task8 pure/idempotent/no-title-guess conversion |
| `prompts.py`, `test_prompts.py`, `test_prompt_audit.py` | Task9 Git blobs/history/grammar/nearest/search/audit; Task17 CLI |
| `llm_meta.py`, `test_llm_meta.py`, `test_vision_models.py` | Tasks10–13 HTTP/schema/fallback/order/local fit/consent/admission |
| `test_generate_meta.py` | Tasks6,11–13,17 marker merges/batch order/consent/exit status |
| `test_llm_meta_openrouter.py` | Deterministic full mock response by default; separately opt-in ignored live test |
| `deviantart.py`, `test_deviantart.py` | Task14 browser tests preserving selectors/STEPS/session behavior |
| `test_da_publish.py`, `test_platforms.py` | Tasks7,14–17 target selection, headless forwarding, retries, real protocol and recovery |
| `test_server.py` business/authorization/concurrency cases | Tasks2–7,9,13,15–16 and UI tests; no safety rule disappears with HTTP |
| `test_server.py` Origin/Host/content-type/body/obsolete-route details | Retire only with removal of that HTTP interface; worker has its own strict protocol tests |
| `test_page.py`/`test_prompt_view.py` behavior and JS autosave races | Tasks18–22 native labels/phase controls/shared prompt/dirty revision/flush/focus acceptance |
| HTML escaping, DOM selector/placeholder template implementation checks | Retire template-only assertions; native plain-text/markup-as-text fixtures replace trust-boundary expectations |
| `test_calendar.py` | Tasks8,21–22 per-apparition/shared date/title/actual URL/timezone/missing-art/native navigation |
| `test_docs.py` | Updated Rust/Python/Nix path checks in packaging tests plus documentation review; don't keep a Python non-browser runtime just for a regex path test |
| flake apps/runtime path assumptions | Tasks17,22–23 package from source-independent/external CWD and smoke existing aliases |

## Review checklist and stop conditions

- [ ] Map every spec section 1–12 to the tasks/matrix; no unexplained feature removal or unimplemented recovery path.
- [ ] Check declared interfaces against every snippet/task; fix name/type mismatches before approval.
- [ ] Scan for unfinished placeholders, empty implementation stubs, undefined fixture helpers and commands targeting files not created by an earlier/same task.
- [ ] Check sequencing: CLI/core validates before UI; Python extraction before Rust transport; journal before dispatch; native/package proof before deletion.
- [ ] Confirm default offline tests cannot install models/call remote providers/run a real publisher.
- [ ] Confirm CLI/core library requires no Iced feature; no permanent dual stack, extra web UI, database, provider/platform framework or unused dependency.
- [x] Parent self-review/dispositions recorded in `/tmp/publicator-rust-migration/2026-10-20-review.md`; spec and plan auto-approved as delegated. Remaining execution checkboxes intentionally stay unchecked.

If an execution-time API/dependency/renderer incompatibility appears, stop that task and replan the smallest seam; do not hide it by removing a safety test. Provider API/model churn is not authority to add a paid/Claude fallback. Worker browser uncertainty is not authority to retry a sent job or modify the original login profile. Failed subagent infrastructure requires same-protocol recovery or explicit owner-approved execution-mode fallback; registry presence alone is not account support.

**Execution instruction:** RESUMED by owner. Continue the exact Task20 fix2 checkpoint using fresh luna-6 implementers and independent reviews; do not rerun the obsolete Task6 script. Children never merge/push or remove the worktree. Only the parent integrates/pushes after all final gates and removes the worktree when clean afterward; no live artwork/provider/profile effects are authorized.

## Other backlog (unchanged)

- **Platform-independent scheduling.** Trigger `da-publish --all --headless` from an external scheduler (e.g. a GitHub Actions cron) instead of paying for DeviantArt's premium scheduling. Groundwork exists in the current Python apps/tests; the trigger itself is not started, and the PerimeterX login profile must exist on its runner. Revisit code/test paths after Rust cutover. This item is not part of the paused migration and does not authorize live scheduling now.
