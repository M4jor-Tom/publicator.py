# 0009. Separate UI/CLI interfaces, application operations and Store

| Field    | Value |
|----------|-------|
| Date     | 2026-10-19 |
| Status   | Accepted |
| Deciders | theta (repo owner) |

## Context

Publicator offers a human-review web UI and command-line entry points over the
same publication data. Both must preserve completed-phase state, approved shared
content and per-platform delivery results, including when another request or CLI
changes the JSON file during a slow provider/browser call.

The existing separation is by responsibility, not just directory: `apps/` holds
CLI adapters and some batch orchestration; callable application operations live
in `workflow.py` and `platforms.py`. The HTTP server also retains interface-level
orchestration. Recording those actual boundaries avoids either treating Store as
the whole application or requiring the UI to shell out to CLI commands.

## Decision

We retain the existing in-process architecture:

```text
UI (webui/page.py + webui/server.py)     CLI (apps/*.py)
                    \                 /
                     application operations
          workflow.py / platforms.py / provider helpers
                              |
                  Store: validated locked mutation
                              |
                      publications.json
```

- **UI and CLI are distinct interfaces, not required mirrors.** The UI owns
  HTTP/body/origin validation, rendered-path allow-lists, interaction state and
  request/batch guards; CLI adapters own arguments, prompts and exit status.
  They dispatch intent to Python operations, not subprocess CLI facades. A UI
  edit does not require a matching CLI command. Search/candidate deletion are
  interface/file actions, not publication-state writes.
- **Application operations own publication intent and phase transitions.**
  `workflow.py` implements picking, date/metadata edits and approvals, returns to
  earlier phases, selection/options, generated-result merges and delivery
  completion. `platforms.publish_selected` validates stored targets and dispatches
  only registered executable publishers. Provider/browser helpers perform their
  external work outside Store mutation locks.
- **Store owns persistence, not user interaction.** `store.mutate_publications`
  locks, rereads the current file, applies an operation's callback, validates the
  entire result with data-directory config and atomically replaces the JSON via
  a same-directory temporary file. Its thread lock and advisory file lock cover
  only that short mutation. Readers may load Store directly; invalid JSON is an
  error, not an empty workflow.
- **Persisted state is authoritative.** `publication.state` records completed
  phases; apparitions retain delivery state/options and actual URLs. Interface
  caches, session storage and in-flight guards do not replace these records.
  Requests and retries resolve fresh stored records; slow generation results
  merge into the current record, not an old whole-file snapshot.

This records the implementation, not a clean-room dependency graph. The server
still orchestrates AI calls and busy UUIDs; `apps.generate_meta.generate_pending`
coordinates CLI batches. Both save through `workflow`. Legacy Store convenience
functions forward to workflow operations through local imports; the separate
Store marker-only merge helper remains for existing callers. Those compatibility
entry points are not a second state-transition design or an instruction to move
new business rules into Store.

## Alternatives Considered

### Make the UI a read-only view of CLI commands

Rejected: picking, drafts, approvals and selection are real UI mutations. They
need callable application operations; executing command-line entry points would
couple HTTP behavior to argument parsing, prompts and process exit semantics.

### Require identical UI and CLI capabilities

Rejected: interfaces have different interaction contracts. For example, local
model installation has UI install-on-use confirmation versus a CLI `[Y/n]` prompt
or `-y`. Shared operations preserve data rules without forcing command parity.

### Put transitions in HTTP handlers or in Store

Rejected: handler-owned rules would not serve other interfaces, while a Store
that owns interactions/provider orchestration would mix persistence with use-case
policy and risk holding locks through slow work. Operation callbacks keep state
checks next to mutations without making the storage mechanism the application.

### Derive workflow state from content or keep a RAM-only queue

Rejected: content alone does not record explicit human approvals. A process-local
queue also loses edits on restart and disagrees with external CLI changes.
Persisted completed-phase and delivery states remain separate and authoritative.

## Consequences

### Positive

- Interfaces reuse phase rules and persistence without calling one another.
- Refresh/reopen and retry reconstruct work from disk and skip saved successes.
- Short validated writes prevent stale whole-file replacement during AI/browser
  work; unrelated reads and supported edits can continue.

### Negative / Trade-offs

- Some request/batch orchestration remains in interface modules; `apps/` is not a
  universal application-service package and the import graph is not strictly
  one-way because of the Store forwarding functions.
- Provider/browser effects and recording their result cannot form one atomic
  transaction. An accepted remote submission followed by a disk-write failure
  still requires human reconciliation.
- File locks serialize mutations; they do not provide a distributed job queue or
  a cross-process reservation for an entire provider/browser operation.

### Neutral

- JSON plus stdlib locks remains sufficient for this local, trusted-human tool;
  this decision adds no service layer framework, database or CLI commands.
- Feature-specific contracts and their rationale stay in `docs/features/`.

## Resumption (for Agent)

### Current state

Implemented. This ADR documents the existing boundaries; it introduces no runtime
change. Preserve UI interactions and persisted publication state when maintaining
or simplifying them.

### Key files / entry points

| File | Role |
|------|------|
| `src/publicator/webui/server.py` | HTTP adapter, UI orchestration and operation dispatch |
| `src/publicator/webui/page.py` | Pure renderer and browser interaction client |
| `src/publicator/apps/` | CLI arguments/startup; metadata batch orchestration |
| `src/publicator/workflow.py` | Publication operations and state transitions |
| `src/publicator/platforms.py` | Registered-publisher preflight and dispatch |
| `src/publicator/store.py` | Fresh reads, locks, validation, atomic replacement and compatibility forwards |
| `src/publicator/entries.py` | State constants and metadata-approved target readers |
| `src/publicator/config.py` | Data-directory configuration and publication validation |

### Next steps

None required for this record. For a future change, locate the owning feature,
reuse its operation and preserve Store validation rather than inventing a parallel
write path.

### How to verify

```sh
nix develop -c pytest tests/test_docs.py tests/test_store.py tests/test_server.py tests/test_platforms.py tests/test_generate_meta.py -q
```

These checks exercise documentation paths, persisted transitions, HTTP dispatch,
concurrent-result merges, publisher retry and CLI generation with test doubles.

### Gotchas

- Pass data-directory config explicitly, especially when `--json` points outside
  it; Store's fallback config otherwise comes from CWD.
- Do not hold Store mutation locks during provider/browser work or overwrite the
  database from a pre-call snapshot.
- AI success and manual approval paths are not interchangeable; follow the
  current operations and tests rather than inferring state from nonempty fields.
- UI busy guards are process-local. Persisted state and Store locks are the data
  boundary shared with CLI processes.

### Related

- [Gallery UI](../features/gallery-ui.md)
- [Publications store](../features/publications-store.md)
- [AI metadata](../features/ai-metadata.md)
- [DeviantArt publishing](../features/deviantart-publish.md)
