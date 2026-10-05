# 0007. Local vision models through Ollama, with fit decisions delegated to llmfit

| Field    | Value |
|----------|-------|
| Date     | 2026-10-01 |
| Status   | Accepted |
| Deciders | theta (repo owner) |

## Context

Metadata generation previously depended on hosted providers. Local vision inference
avoids sending artwork to a remote provider and allows generation without a
provider API key, while using Ollama's installed local service and model registry.
The selectable local catalogue is paired explicitly with OpenRouter models. Each
entry carries separate `ollama_name`, `openrouter_name`, and `llmfit_name` IDs;
provider identifiers are not reliably derivable from one another.

Local vision models have materially different resource requirements. Duplicating
hardware/model-fit heuristics in publicator would make them incomplete and stale;
GitHub's llmfit already owns host detection and model fit evaluation.

## Decision

- Use Ollama's local HTTP API for model inventory, pull, and vision generation.
  Ollama must already be running; publicator does not start or manage its daemon.
- Keep all three tool IDs explicit in the paired model catalog. Never construct
  the llmfit or OpenRouter identifier from the Ollama tag (or vice versa).
- Ask `llmfit info <llmfit_name> --json` for each local model's fit verdict.
  A `Good` verdict is eligible; `Too Tight`, failed, or unassessable results are
  unavailable. Publicator reports llmfit's reason rather than reimplementing its
  hardware calculations.
- A fitting model absent from Ollama remains usable: GUI generation indicates
  "Install on use" and pulls it as part of the generation request. The CLI checks
  fit first, then asks `[Y/n]` before pulling (Enter accepts); `-y` accepts
  without prompting. A non-interactive CLI without `-y` aborts before changing
  publications.
- Both hosted and local providers use the shared metadata prompt and schema;
  batch and gallery calls share `generate_metadata` and thumbnail preparation.

## Consequences

- The model and artwork stay local for local inference, but the user is
  responsible for starting and maintaining Ollama.
- Fit availability follows the installed llmfit catalogue and host verdict;
  publicator does not promise a model will finish quickly or fit after system
  resources change.
- GUI installation consent is implicit in the explicit Generate action, while
  CLI installation requires confirmation unless the caller supplies `-y`.
- A paired catalogue needs maintenance when provider model IDs change.

## Related

- `docs/features/ai-metadata.md`
- `docs/features/gallery-ui.md`
- `docs/features/images-thumbnails.md`
