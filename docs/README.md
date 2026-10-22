# docs/

Feature maintenance docs live in `features/`. Each describes Does / Run / Code / Tests / Config / Data / Decisions / Verify, then flow, invariants, and operational gotchas. Read the owning feature before changing code.

## Features

- [Configuration](features/config.md) — data-directory configuration and validation.
- [Publications store](features/publications-store.md) — JSON data contract, locks, atomic writes and recovery state.
- [Images and thumbnails](features/images-thumbnails.md) — candidate discovery, cache and upload preparation.
- [Gallery UI](features/gallery-ui.md) — native Iced Pick / Schedule / Describe / Publish workflow.
- [AI metadata](features/ai-metadata.md) — OpenRouter and local Ollama generation and consent.
- [Scheduling](features/scheduling.md) — timezone-aware dates and capacity.
- [Calendar](features/calendar.md) — read-only native history and apparition navigation.
- [Prompt mapping](features/prompt-mapping.md) — content-addressed Git archive mapping.
- [DeviantArt publishing](features/deviantart-publish.md) — Rust orchestration and retained Python Playwright boundary.

## Architecture decisions

- [0009 Separate interfaces, application operations and Store](adr/0009-separate-interfaces-application-operations-and-store.md) — historical Python/HTTP decision, superseded for the application interface by:
- [0010 Rust core and native Iced interface](adr/0010-rust-core-and-native-iced-interface.md).

Feature-specific behavior belongs with its owner. Preserve historical ADR/roadmap context; do not treat it as current runtime guidance.

## Maintaining

- Document each symbol in the feature that owns its implementation; other files link there.
- `src/` and `assets/publicationsSchema.json` are Rust application/code assets. `python/publicator_da/` is browser-only. The publishing skill remains a static manual/recovery contract.
- Update feature docs, tests and code together. `tests/docs.rs` checks concrete paths in maintained docs; glob/placeholders and historical ROADMAP/ADR paths are intentionally excluded.
- Add architecture decisions with the next ADR number; mark superseded decisions without erasing history.
- Plans, execution reports, screenshots and temporary fixtures belong in `/tmp`, not Git.
