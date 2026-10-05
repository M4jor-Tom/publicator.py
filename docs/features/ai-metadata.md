# AI metadata (title/description via llm CLI and Ollama)

- **Does:** Generates title + description for publications marked with `[[PUBLICATOR:TODO_GENERATE]]`, or for a gallery image, using local Ollama, OpenRouter free, or Claude. OpenRouter 429s walk its dynamic free vision catalogue; local fit decisions come from llmfit.
- **Run:** `nix run .#generate-meta -- --data-dir ../Art --source {local,openrouter,claude} [--model MODEL] [--timeout S] [-y] [-v]` (publication-driven, no image arguments; source required; local requires a model; `-y` accepts missing-model installation)
- **Run:** `nix run .#publish-next -- [--ai-model ID] [--openrouter-model ID] [--ai-timeout S] [-v]` (selector + Generate with AI → `POST /ai {path, source, model}`; model is a catalog object or null for Claude)
- **Code:** `src/publicator/llm_meta.py` (`generate_metadata`, `VISION_MODELS`, `local_model_status`, `prepare_local_model`, `run_llm`, `_attempt`, `_fallbacks`), `src/publicator/apps/generate_meta.py` (`generate_pending`, `main_args`), `src/publicator/images.py` (`thumb_for_ai`, `ensure_thumb`), `src/publicator/webui/server.py` (`GalleryHandler.do_POST` `/ai`, local status), `src/publicator/webui/page.py` (`aiGen`, `render_page`), `src/publicator/apps/publish_next.py` (`main`), `flake.nix` (Ollama, llmfit, llm providers)
- **Tests:** `tests/test_llm_meta.py` (provider routing, Ollama and llmfit behavior, schema retry, 429 fallback); `tests/test_generate_meta.py` (marker selection, consent, writes and failures); `tests/test_page.py` and `tests/test_server.py` (model options and endpoint)
- **Config:** no model IDs in `publicator.toml`; paired model objects explicitly map `ollama_name`, `openrouter_name`, and `llmfit_name` in `llm_meta.VISION_MODELS`. llmfit owns fit classification; non-Good or unassessable models are unavailable. Ollama must already be running; publicator does not start its daemon.
- **Data:** `publications.json` exact sentinel in `description` and/or DeviantArt `urlElsePublicationName`; `<data-dir>/.thumbs/` for AI thumbnails; `$OPENROUTER_KEY` at runtime; `$HOME/.claude` logged-in session.
- **Decisions:** `docs/adr/0005-retry-without-schema-when-a-model-rejects-it.md`, `docs/adr/0006-walk-the-free-vision-catalogue-on-a-429.md`, `docs/adr/0007-local-vision-models-with-ollama-and-llmfit.md`
- **Verify:** `nix develop -c pytest tests/test_llm_meta.py -q`
- **Verify:** `nix develop -c pytest tests/test_page.py -q`
- **Verify:** `nix develop -c pytest tests/test_llm_meta_openrouter.py -q` (live; needs network + `$OPENROUTER_KEY`)
- **Verify:** `env -u OPENROUTER_KEY nix develop -c pytest tests/test_llm_meta_openrouter.py -q` (must be RED when online, not skipped)
- **Verify:** `nix run .#generate-meta -- --data-dir ../Art --source openrouter` (publication-driven; requires marked records and `$OPENROUTER_KEY`; local generation additionally requires a running Ollama service)

## How it works

1. Entry: `aiGen(cardId, path)` posts selected `{path, source, model}` (`model` is null for Claude). `generate_meta.main_args` requires `--source`, reads `publications.json`, and selects records with the exact marker in either supported field; no positional image args.
2. `GalleryHandler.do_POST` `/ai` canonicalizes full model objects against `VISION_MODELS`, accepts only local/OpenRouter/Claude source, and rejects unknown objects. Local requests also require current fit and service availability.
3. Both callers downscale through `images.thumb_for_ai`; batch generation uses `<data-dir>/.thumbs` rather than a temporary directory.
4. `generate_metadata(image_path, model_option, *, source, ...)`: local uses Ollama's vision API, OpenRouter uses the object's explicit `openrouter_name` with `-a`, and Claude keeps the Read-path `llm` invocation. All share the title/description prompt and schema.
5. `local_model_status` asks `llmfit info <llmfit_name> --json` and checks installed tags through Ollama. GUI generation pulls a missing fitting model on use; CLI asks `[Y/n]` (Enter=yes), while `-y` bypasses consent.
6. `_attempt` schema: builds `in_prompt = prompt + _JSON_FALLBACK` once. `model in _NO_SCHEMA` → one call with `in_prompt`, no `--schema`. Else `--schema` + bare prompt; `rc != 0` with `NO_SCHEMA_SUPPORT` in stderr → `_NO_SCHEMA.add(model)`, re-run.
7. `_invoke(run, argv, prompt, timeout)` wraps `run`, converts `subprocess.TimeoutExpired` to `RuntimeError("llm timed out after {timeout}s")`, and logs argv, prompt, elapsed/rc at DEBUG (`-v` via `publicator.setup_logging`).
8. `run_llm` holds a `deadline = monotonic() + timeout` for the WHOLE call and loops `_fallbacks(model, run)` — the model, then (for `openrouter/` ids only) every other free vision id from `llm openrouter models --free --json`, same-vendor last — handing each attempt only the remaining seconds, breaking on `rc == 0`, a stderr without `RATE_LIMITED`, or a spent budget; a swap logs at INFO. Still `rc != 0` after the last candidate raises `RuntimeError("llm failed ({rc}): {stderr}")`. With a schema, `_loads_json` parses stdout's outermost `{...}` (fences tolerated); `JSONDecodeError` → `RuntimeError("llm returned non-JSON with schema")`.
9. `generate_metadata` strips `title`/`description`; either empty raises `RuntimeError("AI reply missing title/description: ...")`; returns `(title, description)`.
10. `do_POST` maps `RuntimeError` to `400 {"error"}`, else `200 {"title", "description"}`; `aiGen` writes them into the card's `.f-title`/`.f-description`, or `d.error || "HTTP <status>"` into `.err`.
11. `generate_pending` resolves each marked publication's art via `entries.find_art_path`, replaces only fields still equal to the sentinel, and calls `atomic_write_json` after each successful record. A failure restores that record's old fields, reports UUID/path/error, continues the batch, and makes the CLI exit nonzero.

## Invariants

- **The Claude path passes the image as a Read-able PATH (`-o allowedTools Read -o cwd DIR`, basename in the prompt), never `-a`.** `llm -a` is broken through llm-claude-cli; `run_llm` else-branch, `tests/test_llm_meta.py::test_run_llm_builds_argv_and_parses_schema`, `::test_generate_metadata_returns_title_desc`.
- **`openrouter/*` ids use `-a <abspath>` and carry NO claude-cli `-o` options.** llm-openrouter models have no Read tool and no `-o` options; `generate_metadata` prefix branch, `tests/test_llm_meta.py::test_generate_metadata_openrouter_uses_attachment`.
- **A `--schema` rejection (stderr has "does not support schemas") is retried once, schema in the prompt, and the model memoised in `_NO_SCHEMA`.** llm refuses client-side before any HTTP call, so the retry costs one spawn (ADR 0005); `test_llm_meta.py::test_run_llm_retries_without_schema_then_remembers_the_model`.
- **A 429 walks to the next eligible free vision model; every other non-zero exit still raises at once.** The `:free` pool is shared upstream, so only re-selection helps (ADR 0006); `run_llm` guard `RATE_LIMITED not in (cp.stderr or "")`, `tests/test_llm_meta.py::test_run_llm_429_walks_to_another_vendor_before_a_sibling`, `::test_run_llm_non_429_failure_is_not_retried`.
- **Only `openrouter/*` ids walk, and models are listed only after a 429.** `_fallbacks` is a generator that returns after its first yield otherwise, so the claude path and every happy path spawn nothing extra; `tests/test_llm_meta.py::test_run_llm_429_on_the_claude_path_never_lists_models`.
- **An unlistable or reshaped catalogue re-raises the original 429 unchanged.** The rescue path must never become the failure (ADR 0004 precedent), but the catch is the narrow `(OSError, ValueError, KeyError, TypeError)` so our own bugs still raise; `tests/test_llm_meta.py::test_run_llm_unlistable_models_still_raise_the_original_429`.
- **`timeout` budgets the whole call, not each attempt.** Otherwise the walk turns one 300 s ceiling into N x 2 x 300 s on a synchronous `/ai` the gallery tab blocks on (ADR 0006); `run_llm` `deadline`, `tests/test_llm_meta.py::test_run_llm_stops_walking_once_the_timeout_budget_is_spent`.
- **Candidates come from `llm-openrouter`'s cache, never from a direct fetch of openrouter.ai.** `register_models` mints the ids `llm -m` accepts from that same copy, so a listed candidate always resolves; a fresher list could offer one that answers `Unknown model`, which is not a 429 and would abort the walk (ADR 0006 alternative G); `llm_meta.FREE_MODELS_ARGV`.
- **Both recoveries live under `run_llm` (`_attempt` owns `--schema`, the loop owns `-m`); callers never know about either.** Hoisting would rebuild argv per caller (ADR 0005 alternative E, ADR 0006 alternative F); `rg -n -- '--schema' src/` hits only `src/publicator/llm_meta.py`.
- **Every AI caller hands the model a thumbnail via `images.thumb_for_ai`.** Same visual info for a fraction of tokens on a 300px-bound call; both gallery and batch use the data-dir `.thumbs` cache.
- **`/ai` accepts explicit provider sources and canonical catalog objects only.** Local/OpenRouter objects must exactly match `VISION_MODELS`; Claude accepts no model object, and local fit/availability is enforced server-side (`docs/features/gallery-ui.md`).
- **Permissive JSON parsing (outermost `{...}`) still raises on unparseable output; empty title/description also raises.** Permissive parsing must not decay into returning `{}` (ADR 0005); `tests/test_llm_meta.py::test_run_llm_bad_json_with_schema_raises`, `::test_generate_metadata_empty_raises`.
- **The live OpenRouter test skips ONLY when offline (with a warning); online with no `$OPENROUTER_KEY` fails.** A credential-gated skip rots unnoticed; `tests/test_llm_meta_openrouter.py::test_openrouter_generates_title_and_description` (`_online()` gate, then `assert os.environ.get("OPENROUTER_KEY")`).
- **The live test targets a model that does NOT advertise `structured_outputs` (`SCHEMALESS_VISION_MODEL`).** Its purpose is to exercise the fallback end to end; `tests/test_llm_meta_openrouter.py` docstring.
- **Provider IDs are explicit catalog values, not derived from another provider's name.** Local/OpenRouter selection resolves against `VISION_MODELS`; Claude retains its module model. The CLI's OpenRouter default is `OPENROUTER_MODEL`; local requires explicit `--model`.
- Relies on the header select listing OpenRouter (free) first as gallery default while the server default (`ai_model`, used when the body omits `model`) stays `claude-cli-opus` (commit d3153d5), owned by `docs/features/gallery-ui.md`.

## Gotchas

- Don't "fix" a future schema failure by hunting a free model id that supports structured outputs: that is how the original bug arose (`:free` ids churn). The fallback handles it; ADR 0005 alternative A.
- A 404 or `Unknown model` on the live test's model means the id died, not the fallback. Pick another schema-less free vision model with the curl|jq in `tests/test_llm_meta_openrouter.py` and change `SCHEMALESS_VISION_MODEL`.
- `_NO_SCHEMA` is process-global mutable state. Any test exercising the retry must `monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())` or it poisons later tests expecting `--schema` in argv.
- Schema-rejection detection string-matches llm's error text (`NO_SCHEMA_SUPPORT = "does not support schemas"`, llm 0.31.1 per ADR 0005). Re-check on an `llm` bump; the live test catches drift. Failure is a loud `RuntimeError`, never a wrong answer.
- Worst-case wall clock for a schema-rejecting model is 2x timeout: both llm spawns get the same `timeout`. In practice the first dies client-side in about a second.
- The free pool 429s constantly (shared upstream, `is_byok: false`); `run_llm` now walks the other free vision ids rather than failing. There is still no *cross-provider* fallback: a 429 never moves you to the claude path, and a missing `$OPENROUTER_KEY` is not a 429.
- A walk can hand you metadata from a model you did not select — the INFO line naming which id answered is the only signal, so don't silence it. Eligibility is "free + takes images", which admits non-captioners like `nvidia/nemotron-3.5-content-safety:free`.
- Everything stubs through the one `run=` seam, the listing included; `free_vision_models` takes an already-parsed list, so it tests with a literal. The plugin's own cache is on disk (1 h, shared across processes) and serves its last good copy when openrouter.ai is down, so the listing needs neither a key nor a network.
- Any non-schema, non-429 exit becomes a RuntimeError in the card's `.err` line or on generate-meta's stderr. `Unknown model` stays in that bucket on purpose (ADR 0006 alternative E): a dead id must be loud, not papered over.
- `thumb_for_ai` degrades to the full-res file: `ensure_thumb` copies when `convert` is absent/fails, and an unwritable cache dir (`OSError`) returns the original. Costlier, not broken.
- The `generate-meta` app includes Ollama and llmfit in its runtime closure; the Ollama service itself must be started separately. Batch uses persistent `<data-dir>/.thumbs`; without ImageMagick, the existing thumbnail fallback may pass the original image.
- The vision call is slow (default timeout 300 s per image); the publication-driven batch processes each marked record and atomically saves successful results one at a time.
- The claude-cli path needs `claude` on PATH and a logged-in subscription; the flake apps add it via runtimeInputs, a bare `python -m publicator.apps...` outside nix does not.
- llm-claude-cli 0.1.3 is vendored from PyPI in `flake.nix` (hash-pinned); llm-openrouter comes from nixpkgs `ps.llm-openrouter`. No `llm install` state anywhere.

## Open items

- Batch local generation requires an already-running Ollama service; publicator does not manage its lifecycle.
- Gallery default selection (OpenRouter, first `<option>`) and server fallback default (`ai_model` = claude) differ by design; `test_page_offers_both_models` asserts presence only, so a reorder would go unnoticed.
