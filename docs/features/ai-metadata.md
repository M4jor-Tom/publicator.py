# AI metadata (title/description via llm CLI and Ollama)

- **Does:** Generates shared publication title/description through local Ollama, OpenRouter free, or Claude. Every accepted Phase 3 generation failure persists the exact TODO marker in requested fields, unconditionally; generate-meta consumes it later. OpenRouter walks its dynamic free vision catalogue; vision-models reports local host fit.
- **Run:** `nix run .#generate-meta -- --data-dir ../Art --source {local,openrouter,claude} [--model MODEL] [--timeout S] [-y] [-v]` (publication-driven, no image arguments; source required; local requires a model; `-y` accepts missing-model installation)
- **Run:** `nix run .#publish-next -- [--ai-model ID] [--openrouter-model ID] [--ai-timeout S] [-v]` (selector + Generate with AI → `POST /ai {uuid, source, model, fields}`; model is a catalog object or null for Claude)
- **Run:** `nix run .#vision-models` (fit verdicts for configured local vision models; requires neither a data directory nor a running Ollama service)
- **Code:** `src/publicator/llm_meta.py` (`TODO_GENERATE`, `ProviderUnavailable`, `generate_metadata`, `VISION_MODELS`, `local_model_fit`, `local_model_status`, `prepare_local_model`, `run_llm`, `_attempt`, `_fallbacks`), `src/publicator/apps/generate_meta.py` (`generate_pending`, `main_args`), `src/publicator/apps/vision_models.py` (host-fit CLI), `src/publicator/images.py` (`thumb_for_ai`, `ensure_thumb`), `src/publicator/webui/server.py` (`GalleryHandler.do_POST` `/ai`, local status), `src/publicator/webui/page.py` (`render_page`), `src/publicator/apps/publish_next.py` (`main`), `flake.nix` (Ollama, llmfit, llm providers)
- **Tests:** `tests/test_llm_meta.py` (provider routing, Ollama and llmfit behavior, schema retry, model fallback); `tests/test_vision_models.py` (fit report and unavailable status); `tests/test_generate_meta.py` (marker selection, consent, writes and failures); `tests/test_page.py` and `tests/test_server.py` (model options and endpoint)
- **Config:** no model IDs in `publicator.toml`; paired model objects explicitly map `ollama_name`, `openrouter_name`, and `llmfit_name` in `llm_meta.VISION_MODELS`. llmfit owns fit classification; only `Good` models fit, and unassessable models are unavailable. The `vision-models` fit report does not need Ollama; local generation still requires its service, which publicator does not start.
- **Data:** `publications.json` exact sentinel in publication `title` and/or `description`; `<data-dir>/.thumbs/` for AI thumbnails; `$OPENROUTER_KEY` at runtime; `$HOME/.claude` logged-in session.
- **Decisions:** `docs/adr/0005-retry-without-schema-when-a-model-rejects-it.md`, `docs/adr/0007-local-vision-models-with-ollama-and-llmfit.md`, `docs/adr/0008-walk-the-free-vision-catalogue-on-any-error.md`
- **Verify:** `nix run .#vision-models`
- **Verify:** `nix develop -c pytest tests/test_vision_models.py tests/test_llm_meta.py -q`
- **Verify:** `nix develop -c pytest tests/test_page.py -q`
- **Verify:** `nix develop -c pytest tests/test_llm_meta_openrouter.py -q` (live; needs network + `$OPENROUTER_KEY`)
- **Verify:** `env -u OPENROUTER_KEY nix develop -c pytest tests/test_llm_meta_openrouter.py -q` (must be RED when online, not skipped)
- **Verify:** `nix run .#generate-meta -- --data-dir ../Art --source openrouter` (publication-driven; requires marked records and `$OPENROUTER_KEY`; local generation additionally requires a running Ollama service)

## How it works

The UI accepts generation only for a fresh SCHEDULED UUID with a canonical model,
source, stored artwork and nonempty requested-field list. Rejected requests do not
write markers. After acceptance, provider/model failures save
`[[PUBLICATOR:TODO_GENERATE]]` in every requested field, regardless of local fit
or prior hosted failures. Results and markers merge into fresh stored data before
reporting saved; unrequested manual fields survive. Unexpected exceptions remain
visible server errors, not a successful generation response.

Generation never approves metadata. Blank or marker-bearing metadata cannot
advance/publish. The CLI replaces only fields still equal to the exact marker,
saves successes individually via the locked store merge, preserves review state,
and returns nonzero on failures. No apparition is needed for generation.

## Invariants

- **The Claude path passes the image as a Read-able PATH (`-o allowedTools Read -o cwd DIR`, basename in the prompt), never `-a`.** `llm -a` is broken through llm-claude-cli; `run_llm` else-branch, `tests/test_llm_meta.py::test_run_llm_builds_argv_and_parses_schema`, `::test_generate_metadata_returns_title_desc`.
- **`openrouter/*` ids use `-a <abspath>` and carry NO claude-cli `-o` options.** llm-openrouter models have no Read tool and no `-o` options; `generate_metadata` prefix branch, `tests/test_llm_meta.py::test_generate_metadata_openrouter_uses_attachment`.
- **A `--schema` rejection (stderr has "does not support schemas") is retried once, schema in the prompt, and the model memoised in `_NO_SCHEMA`.** llm refuses client-side before any HTTP call, so the retry costs one spawn (ADR 0005); `test_llm_meta.py::test_run_llm_retries_without_schema_then_remembers_the_model`.
- **Any failed OpenRouter candidate advances to the next eligible free vision model; the final failure is preserved.** Only known transport/HTTP-unavailable errors use `ProviderUnavailable`; unknown-model, auth/configuration, and invalid-output errors remain ordinary `RuntimeError`s after the walk and still persist requested UI markers after acceptance. `run_llm`, ADR 0008, `tests/test_llm_meta.py::test_run_llm_retries_after_any_candidate_failure`.
- **Only `openrouter/*` ids walk, and models are listed only after the selected candidate fails.** `_fallbacks` is a generator that returns after its first yield otherwise, so the Claude path and every happy path spawn nothing extra; `tests/test_llm_meta.py::test_run_llm_429_on_the_claude_path_never_lists_models`.
- **An unlistable or reshaped catalogue does not replace the original model error.** The rescue path must never become the failure (ADR 0004 precedent), but the catch is the narrow `(OSError, ValueError, KeyError, TypeError)` so our own bugs still raise; `tests/test_llm_meta.py::test_run_llm_unlistable_models_still_raise_the_original_429`.
- **`timeout` budgets the whole call, not each attempt.** Otherwise the walk turns one 300 s ceiling into N x 2 x 300 s on a synchronous `/ai` the gallery tab blocks on (ADR 0008); `run_llm` `deadline`, `tests/test_llm_meta.py::test_run_llm_stops_walking_once_the_timeout_budget_is_spent`.
- **Candidates come from `llm-openrouter`'s cache, never from a direct fetch of openrouter.ai.** `register_models` mints the ids `llm -m` accepts from that same copy, so listed fallback IDs are resolvable; stale external listings could waste the bounded walk on repeated `Unknown model` errors; `llm_meta.FREE_MODELS_ARGV`.
- **Both recoveries live under `run_llm` (`_attempt` owns `--schema`, the loop owns `-m`); callers never know about either.** This avoids duplicating provider retry logic at call sites (ADR 0005 alternative E); `rg -n -- '--schema' src/` hits only `src/publicator/llm_meta.py`.
- **Every AI caller hands the model a thumbnail via `images.thumb_for_ai`.** Same visual info for a fraction of tokens on a 300px-bound call; both gallery and batch use the data-dir `.thumbs` cache.
- **`/ai` accepts explicit provider sources and canonical catalog objects only.** Local/OpenRouter objects must exactly match `VISION_MODELS`; Claude accepts no model object, and local fit/availability is enforced server-side (`docs/features/gallery-ui.md`).
- **Permissive JSON parsing (outermost `{...}`) still raises on unparseable output; empty title/description also raises.** Permissive parsing must not decay into returning `{}` (ADR 0005); `tests/test_llm_meta.py::test_run_llm_bad_json_with_schema_raises`, `::test_generate_metadata_empty_raises`.
- **The live OpenRouter test skips ONLY when offline (with a warning); online with no `$OPENROUTER_KEY` fails.** A credential-gated skip rots unnoticed; `tests/test_llm_meta_openrouter.py::test_openrouter_generates_title_and_description` (`_online()` gate, then `assert os.environ.get("OPENROUTER_KEY")`).
- **The live test targets a model that does NOT advertise `structured_outputs` (`SCHEMALESS_VISION_MODEL`).** Its purpose is to exercise the fallback end to end; `tests/test_llm_meta_openrouter.py` docstring.
- **Provider IDs are explicit catalog values, not derived from another provider's name.** Local/OpenRouter selection resolves against `VISION_MODELS`; Claude retains its module model. The CLI's OpenRouter default is `OPENROUTER_MODEL`; local requires explicit `--model`.
- Relies on the header select listing OpenRouter (free) first as gallery default while the server default (`ai_model`, used when the body omits `model`) stays `claude-cli-opus` (commit d0b7d1e), owned by `docs/features/gallery-ui.md`.

## Gotchas

- Don't "fix" a future schema failure by hunting a free model id that supports structured outputs: that is how the original bug arose (`:free` ids churn). The fallback handles it; ADR 0005 alternative A.
- A 404 or `Unknown model` on the live test's selected model now advances to the next catalogue candidate; the INFO log identifies when another model answered. Keep `SCHEMALESS_VISION_MODEL` current so the live test continues to exercise the intended model.
- `_NO_SCHEMA` is process-global mutable state. Any test exercising the retry must `monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())` or it poisons later tests expecting `--schema` in argv.
- Schema-rejection detection string-matches llm's error text (`NO_SCHEMA_SUPPORT = "does not support schemas"`, llm 0.31.1 per ADR 0005). Re-check on an `llm` bump; the live test catches drift. Failure is a loud `RuntimeError`, never a wrong answer.
- The timeout budget covers the complete schema retry and catalogue walk, not each invocation separately.
- `run_llm` walks other free vision ids after any OpenRouter candidate failure. There is still no *cross-provider* fallback: OpenRouter errors never silently move generation to Claude or local Ollama.
- A walk can hand you metadata from a model you did not select — the INFO line naming which id answered is the only signal, so don't silence it. Eligibility is "free + takes images", which admits non-captioners like `nvidia/nemotron-3.5-content-safety:free`.
- Everything stubs through the one `run=` seam, the listing included; `free_vision_models` takes an already-parsed list, so it tests with a literal. The plugin's own cache is on disk (1 h, shared across processes) and serves its last good copy when openrouter.ai is down, so the listing needs neither a key nor a network.
- Unknown-model, auth/configuration, transport, timeout, and HTTP failures all advance through the OpenRouter candidate list. After exhaustion, known transport and HTTP 408/429/5xx errors are `ProviderUnavailable`; unknown-model or auth/configuration errors remain ordinary `RuntimeError`s, and accepted UI requests still persist markers.
- `thumb_for_ai` degrades to the full-res file: `ensure_thumb` copies when `convert` is absent/fails, and an unwritable cache dir (`OSError`) returns the original. Costlier, not broken.
- The `generate-meta` app includes Ollama and llmfit in its runtime closure; the Ollama service itself must be started separately. Batch uses persistent `<data-dir>/.thumbs`; without ImageMagick, the existing thumbnail fallback may pass the original image.
- The vision call is slow (default timeout 300 s per image); the publication-driven batch processes each marked record and atomically saves successful results one at a time.
- Refresh publish-next after external CLI generation; requests reread the current database.
- The claude-cli path needs `claude` on PATH and a logged-in subscription; the flake apps add it via runtimeInputs, a bare `python -m publicator.apps...` outside nix does not.
- llm-claude-cli 0.1.3 is vendored from PyPI in `flake.nix` (hash-pinned); llm-openrouter comes from nixpkgs `ps.llm-openrouter`. No `llm install` state anywhere.

## Open items

- Local generation requires an already-running Ollama service; publicator does not manage its lifecycle.
