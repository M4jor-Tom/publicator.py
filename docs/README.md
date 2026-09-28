# docs/

One file per feature in `features/`. Every feature file has the same shape, so it
can be grepped: an eight-line block (**Does / Run / Code / Tests / Config / Data /
Decisions / Verify**; Run and Verify repeat, one command per line), then `## How it works` (the real call flow), `## Invariants`
(rules with the function or test that enforces them), and optional `## Gotchas` and
`## Open items` (config.md adds a `## Keys` table). `tests/test_docs.py` fails when any doc names a repo path that no
longer exists.

First visit: read `CLAUDE.md` (architecture and gotchas), then the feature you touch.

## Features, in pipeline order

- [Configuration (publicator.toml)](features/config.md) — every key, its default, where it is validated and consumed
- [Publications store (publications.json + schema)](features/publications-store.md) — source-of-truth writers, lookups, schema validation, validate/echo-first CLIs
- [Images and thumbnails](features/images-thumbnails.md) — candidate discovery, content-addressed .thumbs cache, /thumbs allow-list, non-webp uploads
- [Gallery UI (publish-next)](features/gallery-ui.md) — human-review gallery: AI metadata, schedule, stage, batch-publish to DeviantArt
- [AI metadata (title/description via llm CLI)](features/ai-metadata.md) — title + description via `llm` CLI, Claude or OpenRouter, schema fallback
- [Scheduling (slots and weekly profiles)](features/scheduling.md) — server-side weekly slot instants and TZ-anchored picker resolution
- [Calendar tab](features/calendar.md) — read-only month grids of every DA apparition, server-side tz bucketing
- [Prompt mapping (image -> generation prompt)](features/prompt-mapping.md) — content-addressed lookup of an image's prompt in the archive repo: Exact, Nearest or Unknown
- [DeviantArt publishing (Playwright submit flow)](features/deviantart-publish.md) — drives Firefox through DA's submit form for pending entries

## Decisions

`adr/` records why a design was chosen over its alternatives. Each feature file
links the ADRs that constrain it; do not reverse one without a new ADR.

- [0001 Archive prompts at identity-mint time](adr/0001-archive-prompts-at-identity-mint-time.md)
- [0002 Resolve prompts to a three-state type](adr/0002-resolve-prompts-to-a-three-state-type.md)
- [0003 Declare the filename grammar in publicator.toml](adr/0003-declare-the-filename-grammar-in-publicator-toml.md)
- [0004 Fail open when prompt archiving fails](adr/0004-fail-open-when-prompt-archiving-fails.md)
- [0005 Retry without --schema when a model rejects it](adr/0005-retry-without-schema-when-a-model-rejects-it.md)

## Not built yet

`ROADMAP.md`. Done work is never listed there; it lives in `features/`.

## Maintaining

- A rule or symbol is documented once, in the file whose **Code** block owns the
  module that defines it; other files point there instead of restating it.
- Non-Python contracts: the STEPS registry mirrors `.claude/skills/publish-deviantart/SKILL.md`
  (see deviantart-publish.md), `flake.nix` defines every `nix run` app, and
  `src/publicator/publicationsSchema.json` is the data schema (see publications-store.md).
- Code change: update the feature file's block and invariants in the same commit.
- New feature: add `features/<slug>.md` in the same shape and a line above.
- New decision: add `adr/000N-*.md` and link it from the feature's **Decisions** line.
- Design specs and implementation plans are not kept: once shipped, the feature
  file is the truth and git history holds the rest.
