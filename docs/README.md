# docs/

One file per feature in `features/`. Every feature file has the same shape, so it
can be grepped: an eight-line block (**Does / Run / Code / Tests / Config / Data /
Decisions / Verify**; Run and Verify repeat, one command per line), then `## How it works` (the real call flow), `## Invariants`
(rules with the function or test that enforces them), and optional `## Gotchas` and
`## Open items` (config.md adds a `## Keys` table). `tests/test_docs.py` fails when any doc names a repo path that no
longer exists.

First visit: read `AGENTS.md` (architecture and gotchas), then the feature you touch.

## Features, in pipeline order

- [Configuration (publicator.toml)](features/config.md) — every key, its default, where it is validated and consumed
- [Publications store (publications.json + schema)](features/publications-store.md) — source-of-truth writers, lookups, schema validation, validate/echo-first CLIs
- [Images and thumbnails](features/images-thumbnails.md) — candidate discovery, content-addressed .thumbs cache, /thumbs allow-list, non-webp uploads
- [Gallery UI](features/gallery-ui.md) — Search and runtime path filtering, durable Pick/Schedule/Describe/Publish phases, registered-platform grid and success-aware retry
- [AI metadata (title/description via llm CLI)](features/ai-metadata.md) — title + description via `llm` CLI, Claude or OpenRouter, schema fallback
- [Scheduling (slots and weekly profiles)](features/scheduling.md) — server-side weekly slot instants and TZ-anchored picker resolution
- [Calendar tab](features/calendar.md) — read-only month grids of every platform apparition, server-side tz bucketing, prompt popover per thumbnail
- [Prompt mapping (image -> generation prompt)](features/prompt-mapping.md) — content-addressed lookup of an image's prompt in the archive repo: Exact, Nearest or Unknown
- [DeviantArt publishing (Playwright submit flow)](features/deviantart-publish.md) — drives Firefox through DA's submit form for pending entries

## Architecture decisions

`adr/` records cross-cutting architecture: component boundaries, responsibilities,
alternatives and trade-offs. Feature-specific behavior, constraints, rationale
and operational checks live with their owning feature in `features/`, not in
separate ADRs. The former feature ADRs have been consolidated into
[prompt mapping](features/prompt-mapping.md), [configuration](features/config.md)
and [AI metadata](features/ai-metadata.md); their history remains in Git.

- [0009 Separate UI/CLI interfaces, application operations and Store](adr/0009-separate-interfaces-application-operations-and-store.md)

## Maintaining

- A rule or symbol is documented once, in the file whose **Code** block owns the
  module that defines it; other files point there instead of restating it.
- Non-Python contracts: the STEPS registry mirrors `.claude/skills/publish-deviantart/SKILL.md`
  (see deviantart-publish.md), `flake.nix` defines every `nix run` app, and
  `src/publicator/publicationsSchema.json` is the data schema (see publications-store.md).
- Code change: update the feature file's block and invariants in the same commit.
- New feature: add `features/<slug>.md` in the same shape and a line above.
- Feature decision: maintain the owning feature's **Decisions**, flow and invariants.
- Architecture decision: add the next `adr/NNNN-*.md`, link it above and from
  affected features. Preserve numbering; do not reuse former feature ADR numbers.
- Design specs, implementation plans, and task trackers live in `/tmp`, never in
  Git. Feature files record non-code-obvious constraints, decisions, and operational
  knowledge; do not duplicate behavior that is obvious from reading the code.
