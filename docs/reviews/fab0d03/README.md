# Open review — fab0d03

Whole-codebase review of `fab0d03a49951d4482c1c997f30fb6939786306f`, not a review of
that commit's diff. This is a **shrinking backlog**, not a permanent audit transcript.
The original audit changed no application code. Publishing these documents fixes nothing.

## How to use and retire findings

- Each `## ID — ...` block is one independently removable item. IDs are stable; do not renumber.
- Recheck the cited code at the current HEAD before working; line numbers refer to the reviewed SHA.
- Read the owning feature contract, reproduce the stated trigger, then make the smallest shared fix.
- **Delete the whole block only after its closure criteria are demonstrated**, with a regression
  test where applicable. Keep proof in the fixing commit/PR, not a growing “resolved” section here.
- A disproven risk or intentional behavior needs reproducible evidence or an explicit accepted
  decision before removal. Mere absence of a failing test is not disproof.
- For policy-dependent items, update maintained feature docs when resolving the contract.
- Remove active cross-references to retired IDs; delete empty files and their index entries.
  Historical citations in validation.md are provenance, not open items. Move still-useful
  operational rules into their owning feature documentation before deleting their last home.
- Do not remove a coverage gap merely because the existing suite passes. Verify the specific
  request, persisted state, lifecycle action or fault transition named in the item.
- Shared fixture requirements can be removed individually once their dependent checks exist.
  Keep baseline evidence until the review directory is retired; Git retains the history afterward.
- No implementation, live-service validation, commit or push is authorized by this backlog alone.

## Index

| File | Scope |
|---|---|
| [Core](core.md) | Identity/configuration validation, migration, historical scheduling |
| [Core coverage](core-coverage.md) | Store faults, extensions, contention and date boundaries |
| [Metadata](metadata.md) | Newer edits, provider tests, deadlines, admission and persistence |
| [Images](images.md) | Destructive paths, artwork identity, codecs, cache and search parity |
| [Image coverage](image-coverage.md) | Masked hash tests, filesystem/cache/media policy probes |
| [Prompts](prompts.md) | Archive refresh, Git paths, Nearest contract and bounded search |
| [Publishing](publishing.md) | Browser effect, approved bytes, journal and subprocess ownership |
| [Worker/operator](worker-operator.md) | Prober IPC, manual recovery, protocol and abrupt termination |
| [UI](ui.md) | Drafts, close actions, model selection and completion ordering |
| [Native UI](native-ui.md) | Reproducible interactions and actual viewport geometry |
| [Test infrastructure](test-infrastructure.md) | CI, packaging, development launch, fixtures and coverage |
| [Fixtures](fixtures.md) | Offline synthetic-data requirements beyond fake image filenames |
| [Validation](validation.md) | Recorded commands/results, limits and future verification gates |

## Priority and evidence

- **P1:** safety, remote-effect or data-integrity concern; address before release sign-off.
- **P2:** material correctness, resilience, contract or testability work.
- **P3:** narrower resilience/coverage follow-up; not a claim of an observed incident.
- **Defect:** source establishes the failure path; stated prerequisites still matter.
- **Reproduced:** an actual audit command demonstrated the behavior.
- **Gap:** inspected tests do not establish the property; implementation may already be correct.
- **Risk / decision:** runtime investigation or explicit product policy is required.

The source reviewers recommended blocking safety sign-off on the P1 paths. That is not a claim
that every item is reproduced, newly introduced, or a reason to reject only the named commit.
The local malformed-response false-green and development worker launch failure were reproduced;
no live DeviantArt incident was observed. Full testability/coverage is **not established**.

## Work boundaries and sequencing

Start with trustworthy test oracles, then publishing, draft persistence, artwork identity and
historical-data safety. Prober IPC and unknown-outcome instructions are safety work too.
Use separate bounded tasks for core, metadata, images, publisher/browser, UI and infrastructure.
Writers overlap in `src/ui/mod.rs`, `src/images.rs` and shared fixtures: serialize or isolate them.

Reuse existing Store hooks, per-draft saving flags, action streams, HTTP fixtures, fake workers
and the native acceptance example. Do not add another framework, metadata cache, queue abstraction
or production fake mode merely to test these paths. Durable locks/receipts, process cleanup and
executable-fixture fork safety address real hazards; they are not simplification candidates.

Tests use disposable roots, generated media, loopback services and fake sessions only. Do not
access real Art/profile data, install models, spend credentials, log in or publish. An unknown
remote outcome always requires reconciliation before any repeat submission. Never erase its
receipt to unblock a test. Screenshots, raw logs and temporary datasets stay outside Git.
