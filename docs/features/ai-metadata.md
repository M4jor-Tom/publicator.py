# AI metadata (title/description via llm CLI)

- **Does:** Generates a DeviantArt title + description for an artwork via the `llm` CLI with a JSON schema, routing by model-id prefix: key-free `claude-cli-opus` via the Read tool, or `openrouter/*` via `-a` + `$OPENROUTER_KEY`.
- **Run:** `nix run .#generate-meta -- [--model ID | --openrouter] [--timeout S] [--json] [-v] IMAGE...` (no data dir; `claude` comes from flake runtimeInputs, `$OPENROUTER_KEY` from the environment)
- **Run:** `nix run .#publish-next -- [--ai-model ID] [--openrouter-model ID] [--ai-timeout S] [-v]` (header `select#ai-model` + "Generate with AI" button → `POST /ai {path, model}`)
- **Code:** `src/publicator/llm_meta.py` (`generate_metadata`, `run_llm`, `DEFAULT_MODEL`, `OPENROUTER_MODEL`, `NO_SCHEMA_SUPPORT`, `_NO_SCHEMA`), `src/publicator/apps/generate_meta.py` (`main`), `src/publicator/images.py` (`thumb_for_ai`, `ensure_thumb`), `src/publicator/webui/server.py` (`GalleryHandler.do_POST` `/ai` branch, `GalleryHandler.ai_model`, `GalleryHandler.openrouter_model`), `src/publicator/webui/page.py` (`PAGE_TEMPLATE`, `aiGen`, `render_page`), `src/publicator/apps/publish_next.py` (`main`), `flake.nix` (vendored `llm-claude-cli` 0.1.3, `ps.llm-openrouter`, `apps.generate-meta`)
- **Tests:** `tests/test_llm_meta.py` (offline, injected `run=` seam: provider argv, schema retry + memo, error raises), `tests/test_llm_meta_openrouter.py` (live through `SCHEMALESS_VISION_MODEL`; skips offline, asserts key online, 3 attempts), `tests/test_page.py::test_page_offers_both_models` (both model ids present in the header select)
- **Config:** none; model ids are module constants in `src/publicator/llm_meta.py` with CLI overrides on both apps. ADR 0005: not `publicator.toml` keys because generate-meta runs with no data dir.
- **Data:** `.thumbs/<sha512[:32]><ext>` (gallery `cache_dir`; the `-resize 300x300>` output the model sees; generate-meta uses a `TemporaryDirectory` instead), `$OPENROUTER_KEY` (read at runtime by llm-openrouter, never stored), `$HOME/.claude` (logged-in `claude` CLI session, outside repo and data dir)
- **Decisions:** `docs/adr/0005-retry-without-schema-when-a-model-rejects-it.md`
- **Verify:** `nix develop -c pytest tests/test_llm_meta.py -q`
- **Verify:** `nix develop -c pytest tests/test_page.py -q`
- **Verify:** `nix develop -c pytest tests/test_llm_meta_openrouter.py -q` (live; needs network + `$OPENROUTER_KEY`)
- **Verify:** `env -u OPENROUTER_KEY nix develop -c pytest tests/test_llm_meta_openrouter.py -q` (must be RED when online, not skipped)
- **Verify:** `nix run .#generate-meta -- --openrouter path/to/art.png` (end to end through the schema fallback)

## How it works

1. Entry: `aiGen(cardId, path)` in `page.py` reads `select#ai-model` and POSTs `/ai {path, model}`; or `generate_meta.main` parses `IMAGE...` with `--model` / `--openrouter` (exclusive; the latter is `store_const` of `OPENROUTER_MODEL` into `model`).
2. `GalleryHandler.do_POST` `/ai`: `model = data.get("model") or self.ai_model`; anything outside `(self.ai_model, self.openrouter_model)` gets `400 {"error": "model not allowed: ..."}`.
3. Caller downscales: `images.thumb_for_ai(path, cache_dir, name)` calls `ensure_thumb` (`convert path -resize 300x300> thumb` on a miss, `shutil.copy2` on `CalledProcessError`/`FileNotFoundError`) and returns the original path on `OSError`.
4. `generate_metadata(image_path, model, *, run, timeout)`: `openrouter/` prefix → `run_llm(model, _PROMPT_ATTACH, schema=TITLE_DESC_SCHEMA, attach=abspath)`; else `run_llm(model, _PROMPT.format(name=basename), schema=..., cwd=dirname(abspath))`.
5. `run_llm` argv: `attach` set → `llm -m M -a IMG`; else `llm -m M -o allowedTools Read -o cwd DIR -o timeout T`. `run` defaults to `partial(_default_run, timeout=timeout)`: `subprocess.run(argv, input=prompt, capture_output=True, text=True, timeout)`.
6. `run_llm` schema: builds `in_prompt = prompt + _JSON_FALLBACK` once. `model in _NO_SCHEMA` → one call with `in_prompt`, no `--schema`. Else `--schema` + bare prompt; `rc != 0` with `NO_SCHEMA_SUPPORT` in stderr → `_NO_SCHEMA.add(model)`, re-run.
7. `_invoke(run, argv, prompt, timeout)` wraps `run`, converts `subprocess.TimeoutExpired` to `RuntimeError("llm timed out after {timeout}s")`, and logs argv, prompt, elapsed/rc at DEBUG (`-v` via `publicator.setup_logging`).
8. `run_llm`: any other `rc != 0` raises `RuntimeError("llm failed ({rc}): {stderr}")`, no retry. With a schema, `_loads_json` parses stdout's outermost `{...}` (fences tolerated); `JSONDecodeError` → `RuntimeError("llm returned non-JSON with schema")`.
9. `generate_metadata` strips `title`/`description`; either empty raises `RuntimeError("AI reply missing title/description: ...")`; returns `(title, description)`.
10. `do_POST` maps `RuntimeError` to `400 {"error"}`, else `200 {"title", "description"}`; `aiGen` writes them into the card's `.f-title`/`.f-description`, or `d.error || "HTTP <status>"` into `.err`.
11. `generate_meta.main` prints a `Title:` block per image (flush=True) unless `--json`, which buffers one array; a failed image prints `{image}: {e}` to stderr and the loop continues; exit 1 if `len(done) < len(images)`.

## Invariants

- **The Claude path passes the image as a Read-able PATH (`-o allowedTools Read -o cwd DIR`, basename in the prompt), never `-a`.** `llm -a` is broken through llm-claude-cli; `run_llm` else-branch, `tests/test_llm_meta.py::test_run_llm_builds_argv_and_parses_schema`, `::test_generate_metadata_returns_title_desc`.
- **`openrouter/*` ids use `-a <abspath>` and carry NO claude-cli `-o` options.** llm-openrouter models have no Read tool and no `-o` options; `generate_metadata` prefix branch, `tests/test_llm_meta.py::test_generate_metadata_openrouter_uses_attachment`.
- **A `--schema` rejection (stderr has "does not support schemas") is retried once, schema in the prompt, and the model memoised in `_NO_SCHEMA`.** llm refuses client-side before any HTTP call, so the retry costs one spawn (ADR 0005); `test_llm_meta.py::test_run_llm_retries_without_schema_then_remembers_the_model`.
- **Only the schema-rejection marker triggers a retry; any other non-zero exit raises immediately.** 429s / upstream timeouts must not be silently re-spent; `run_llm` guard `NO_SCHEMA_SUPPORT in (cp.stderr or "")`, `tests/test_llm_meta.py::test_run_llm_other_failure_is_not_retried`.
- **The retry lives in `run_llm`, the only place `--schema` enters argv; callers never know about it.** Hoisting would duplicate argv per caller (ADR 0005 alternative E); `rg -n -- '--schema' src/` hits only `src/publicator/llm_meta.py`.
- **Every AI caller hands the model a thumbnail via `images.thumb_for_ai`, never the full-res file.** Same visual info for a fraction of tokens on a 300px-bound call; `server.py` `/ai` branch, `src/publicator/apps/generate_meta.py` inside a `TemporaryDirectory`.
- Relies on `/ai` accepting only `ai_model` and `openrouter_model` (HTTP 400 otherwise, since the id reaches subprocess argv), owned by `docs/features/gallery-ui.md`.
- **Permissive JSON parsing (outermost `{...}`) still raises on unparseable output; empty title/description also raises.** Permissive parsing must not decay into returning `{}` (ADR 0005); `tests/test_llm_meta.py::test_run_llm_bad_json_with_schema_raises`, `::test_generate_metadata_empty_raises`.
- **The live OpenRouter test skips ONLY when offline (with a warning); online with no `$OPENROUTER_KEY` fails.** A credential-gated skip rots unnoticed; `tests/test_llm_meta_openrouter.py::test_openrouter_generates_title_and_description` (`_online()` gate, then `assert os.environ.get("OPENROUTER_KEY")`).
- **The live test targets a model that does NOT advertise `structured_outputs` (`SCHEMALESS_VISION_MODEL`).** Its purpose is to exercise the fallback end to end; `tests/test_llm_meta_openrouter.py` docstring.
- **`OPENROUTER_MODEL`/`DEFAULT_MODEL` are module constants (CLI-overridable), not `publicator.toml` keys.** generate-meta runs with no data dir (ADR 0005 "Neutral"); imported as argparse defaults by `src/publicator/apps/publish_next.py` and `src/publicator/apps/generate_meta.py`.
- Relies on the header select listing OpenRouter (free) first as gallery default while the server default (`ai_model`, used when the body omits `model`) stays `claude-cli-opus` (commit d3153d5), owned by `docs/features/gallery-ui.md`.

## Gotchas

- Don't "fix" a future schema failure by hunting a free model id that supports structured outputs: that is how the original bug arose (`:free` ids churn). The fallback handles it; ADR 0005 alternative A.
- A 404 on the live test's model means the id died, not the fallback. Pick another schema-less free vision model with the curl|jq in `tests/test_llm_meta_openrouter.py` and change `SCHEMALESS_VISION_MODEL`.
- `_NO_SCHEMA` is process-global mutable state. Any test exercising the retry must `monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())` or it poisons later tests expecting `--schema` in argv.
- Schema-rejection detection string-matches llm's error text (`NO_SCHEMA_SUPPORT = "does not support schemas"`, llm 0.31.1 per ADR 0005). Re-check on an `llm` bump; the live test catches drift. Failure is a loud `RuntimeError`, never a wrong answer.
- Worst-case wall clock for a schema-rejecting model is 2x timeout: both llm spawns get the same `timeout`. In practice the first dies client-side in about a second.
- The free OpenRouter pool independently 429s / idle-times-out; those surface as `llm failed (1): ...` with no retry in `run_llm`. The live test retries 3x at its own level, so red means our bug.
- Any non-schema non-zero exit (including a missing `$OPENROUTER_KEY`) becomes a RuntimeError in the card's `.err` line or on generate-meta's stderr; there is no cross-provider fallback.
- `thumb_for_ai` degrades to the full-res file: `ensure_thumb` copies when `convert` is absent/fails, and an unwritable cache dir (`OSError`) returns the original. Costlier, not broken.
- generate-meta's flake app adds only `[ claude ]` as extra runtimeInputs (the `app` helper adds `python`; no imagemagick — dev shell and publish-next have it), so outside the dev shell the model may get the original.
- The vision call is slow (default timeout 300 s per image); generate-meta streams each result unless `--json`, which must buffer for a valid array.
- The claude-cli path needs `claude` on PATH and a logged-in subscription; the flake apps add it via runtimeInputs, a bare `python -m publicator.apps...` outside nix does not.
- llm-claude-cli 0.1.3 is vendored from PyPI in `flake.nix` (hash-pinned); llm-openrouter comes from nixpkgs `ps.llm-openrouter`. No `llm install` state anywhere.

## Open items

- generate-meta's flake app lists no imagemagick, so its `thumb_for_ai` downscale only happens if `convert` is already on the caller's PATH: is the copy fallback acceptable, or should `pkgs.imagemagick` join its runtimeInputs?
- Gallery default selection (OpenRouter, first `<option>`) and server fallback default (`ai_model` = claude) differ by design; `test_page_offers_both_models` asserts presence only, so a reorder would go unnoticed.
