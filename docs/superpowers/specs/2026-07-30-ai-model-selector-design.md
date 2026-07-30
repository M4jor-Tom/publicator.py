# AI model selector for publish-next

**Date:** 2026-07-30
**Status:** approved

## Goal

In the `publish-next` UI, let the user choose — per session, via a global header
dropdown — which model writes the AI title/description for a piece:

- **Claude (subscription)** — today's key-free default (`llm-claude-cli` driving the
  logged-in `claude` CLI, vision via the Read tool).
- **OpenRouter (free)** — a free, vision-capable OpenRouter model, vision via an
  image attachment.

The selection applies to every card's *Generate with AI* button. No per-card
choice, no persistence across restarts.

## Non-goals (YAGNI)

- Per-card model dropdown.
- Automatic fallback between providers.
- More than two providers.
- Persisting the choice across process restarts.

## Changes

### 1. `flake.nix` — add `llm-openrouter` to the python env

Prefer the nixpkgs `ps.llm-openrouter` if it exists in the pinned nixpkgs;
otherwise vendor it from PyPI as a hash-pinned `buildPythonPackage`, mirroring the
existing `llm-claude-cli` derivation. Add it to the `pyInterp.withPackages` list so
`llm` discovers it via its entry point.

The **API key stays out of the flake**: `llm-openrouter` reads `OPENROUTER_KEY`
from the process environment, which `nix run` inherits. Reproducibility and the
key-free Claude path are both preserved.

### 2. `publish_next.py` CLI — `--openrouter-model`

Add `--openrouter-model` with a free vision default, e.g.
`openrouter/google/gemini-2.0-flash-exp:free` (swappable at any time since
OpenRouter's free tier churns). `--ai-model` remains the Claude default.

Both values are wired onto `GalleryHandler` as class attributes (like the existing
`ai_model`), so the handler and page builder can read them.

### 3. Header UI — model `<select>`

Add `<select id="ai-model">` to the header with two `<option>`s: *Claude
(subscription)* with value = the `--ai-model` id, *OpenRouter (free)* with value =
the `--openrouter-model` id. Default selection = the Claude id.

`aiGen()` reads `document.getElementById('ai-model').value` instead of the current
baked-in `AI_MODEL` constant. Both ids are injected into the page via new
placeholders (replacing the single `__AI_MODEL__`).

### 4. `/ai` handler — allow-list the model

Validate the incoming `model` against an allow-list of exactly the two configured
ids (`{ai_model, openrouter_model}`); reject anything else with HTTP 400. This is a
small hardening step: today any client-supplied string is forwarded straight into
the `llm` subprocess argv.

### 5. `llm_meta.generate_metadata` — provider branch

Branch by provider on the model id:

- **`openrouter/…`** → attachment path: `llm -m <model> --schema <schema> -a <image>
  "<prompt>"` with a "look at the attached artwork" prompt and **none** of the
  claude-cli `-o allowedTools/cwd/timeout` options.
- **anything else** → today's Read-tool path unchanged.

This is exactly the second-provider branch the module's docstring already
anticipated.

## Data flow

1. Page load → header dropdown defaults to Claude.
2. User switches the dropdown to OpenRouter.
3. User clicks *Generate with AI* on a card.
4. `POST /ai {path, model}` → allow-list check.
5. `generate_metadata` routes to the attachment path.
6. `llm -m openrouter/… --schema … -a thumb "prompt"` runs (key from
   `OPENROUTER_KEY`).
7. `{title, description}` returned → fills the card's fields.

## Error handling

- Missing `OPENROUTER_KEY`, or a free model that ignores the JSON schema, raises
  `RuntimeError` → surfaced in the card's existing `.err` line. User retries or
  switches back to Claude.
- **No** silent cross-provider fallback — the selected model is the one that runs.
- Model not in the allow-list → 400, surfaced the same way.

## Testing

Extend `test_llm_meta.py`, reusing the existing injected-`run` harness:

- An `openrouter/…` model builds argv containing `-a <image>` and **no**
  `-o allowedTools` / Read path.
- A non-openrouter id keeps the Read-tool path (existing assertions still hold).
- The attachment path still emits `--schema <TITLE_DESC_SCHEMA>`.

No browser/e2e test for the dropdown itself — it's a one-line value read; the
provider branch is where the logic lives and gets the unit test.
