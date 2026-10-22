# 0010. Rust core and native Iced interface

| Field | Value |
|---|---|
| Date | 2026-10-21 |
| Status | Accepted |
| Deciders | theta (repo owner) |

## Context

Publicator replaced the Python application and HTTP/HTML gallery with a Rust core, CLI, and native Iced UI. The DeviantArt browser interaction remains Python Playwright because it is a browser automation boundary, not application logic.

## Decision

```text
CLI (src/bin/)     Native Iced UI (src/ui/)
          \         /
     Rust application operations
       application.rs / workflow.rs
                |
        validated Store (store.rs)
                |
        publications.json
                |
 Rust publisher/journal -- bounded NDJSON --> Python Playwright worker
```

Rust owns configuration, schema validation, scheduling, publication state and persistence, metadata providers, durable publish intent/receipt recovery, CLI and UI. Both interfaces call the Rust application/Store; neither delegates business rules to the other.

The Python boundary is limited to `python/publicator_da/`: one validated versioned worker protocol, browser selectors/session handling, and an explicit DOM-probing daemon. The worker receives one approved job at a time and has no publication database or configuration path. The static `.claude/skills/publish-deviantart/SKILL.md` remains the manual browser-step contract and does not require the Claude CLI.

## Consequences

- The UI is a native Iced desktop application; there is no HTTP application server or browser-based gallery.
- `publications.json` remains authoritative. Validated mutations use persistent thread/file locks and atomic replacement; slow provider/browser operations occur outside the Store lock.
- `.publicator/` holds durable send intents, receipts and staged uploads. An unknown remote outcome requires explicit reconciliation before retry.
- Nix app aliases execute Rust binaries, except the real-Firefox human login wrapper and explicit Python Playwright DOM prober.
- Data directories remain separate from code. Persistent locks, `.publicator/`, `.thumbs/`, and browser profiles should be ignored by the data repository.
- Local Ollama dependency URL override `PUBLICATOR_OLLAMA_URL` is CLI-only; it is not a fake mode or additional production backend.
- No Python application compatibility bridge, alternate HTTP UI, database, or provider framework is retained.

## Alternatives considered

Retaining HTTP as a compatibility frontend would keep two UI stacks and require parallel maintenance. Embedding Python business logic would violate Rust ownership of validation and state. Both are rejected; Python remains only where real browser control is required.

## Related

- [0009 historical UI/CLI and Store decision (superseded for interface implementation)](0009-separate-interfaces-application-operations-and-store.md)
- [Gallery UI](../features/gallery-ui.md)
- [Publications store](../features/publications-store.md)
- [DeviantArt publishing](../features/deviantart-publish.md)
