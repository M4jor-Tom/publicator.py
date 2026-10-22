# AI metadata

- **Does:** Generates a validated title/description pair through OpenRouter or local Ollama; only a complete pair advances a publication to DESCRIBED.
- **Run:** `nix run .#generate-meta -- --help`; or use the native `ui` Describe phase.
- **Code:** `src/metadata/`, `src/application.rs`, `src/images.rs`, `src/bin/publicator.rs`, `src/bin/publicator-ui.rs`.
- **Tests:** `tests/metadata*.rs`, `tests/application_ai.rs`, `tests/cli.rs`, `tests/ui_describe.rs`.
- **Config:** Provider/model selection is explicit; local fit is assessed with llmfit. CLI-only `PUBLICATOR_OLLAMA_URL` overrides the Ollama endpoint.
- **Data:** `publications.json` stores title/description; `.thumbs/` holds AI input previews. Runtime credentials are read only when the selected provider needs them.
- **Decisions:** Local Ollama and OpenRouter only. No Claude CLI, Python `llm` bridge, automatic cross-provider fallback, or production fake mode.
- **Verify:** `nix develop -c cargo test --test metadata --test application_ai --test cli`

## How it works

All calls request both title and description and validate the response as a complete usable pair. UI acceptance saves the exact TODO marker in both fields on generation failure; the publication remains SCHEDULED and retryable. Generated results merge against fresh Store state so a newer manual edit is not overwritten. Batch admission validates the complete source document before any provider call; each successful record is persisted before processing the next.

OpenRouter requests use Rust HTTP clients and loopback fixtures in ordinary tests. Local generation uses an already-running Ollama service; Publicator does not start or install the service/model. `vision-models` reports local host fit. Install-on-use requires explicit consent in the GUI or CLI.

## Invariants

- A missing, blank or marker-bearing field cannot be approved or published.
- Provider work occurs outside Store locks; save merges only fields still carrying the exact marker.
- Invalid source data/config fails before provider admission and leaves source bytes unchanged.
- The `PUBLICATOR_OLLAMA_URL` override is CLI-only and selects a dependency endpoint; it is not a hidden test/fake switch.
- Tests use deterministic local/loopback fixtures and never require live provider credentials.

## Gotchas

- A complete source snapshot must validate before a batch starts; invalid rows are not reported as ordinary per-record provider failures.
- Local service/model installation has external effects and is outside routine validation.
