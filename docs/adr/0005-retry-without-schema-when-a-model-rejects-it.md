# 0005. Retry without `--schema` when a model rejects it, instead of pinning a schema-capable model

| Field    | Value                                     |
|----------|-------------------------------------------|
| Date     | 2026-08-23                                |
| Status   | Accepted                                  |
| Deciders | theta (repo owner)                        |
| Branch   | `master`                                  |
| Commit   | `0e88998`, `4de25b3`                      |

## Context

`llm_meta.generate_metadata` asks the model for structured output by passing
`--schema` to the `llm` CLI. Through `llm-openrouter` that failed outright:

```
llm failed (1): Error: OpenRouter: openrouter/google/gemma-4-26b-a4b-it:free
does not support schemas
```

The mechanism, traced end to end:

- `llm-openrouter` sets each model's `supports_schema` from the OpenRouter
  catalogue: `supports_schema=has_parameter(model_definition, "structured_outputs")`.
- `llm` core refuses in `Response.__init__` (`llm/models.py:691`,
  version 0.31.1 as pinned by our nixpkgs) —
  `if self.prompt.schema and not self.model.supports_schema: raise ValueError(...)`.
- That guard runs **before any HTTP request**, and before
  `Attachment.content_bytes()` / `base64_content()` (both lazy), so a rejected
  call never reaches the API and never reads the image.

Measured against the live catalogue on 2026-08-23: of the 8 free vision models
on OpenRouter, **7 do not advertise `structured_outputs`** — including both
gemmas. `publish-next`'s `--openrouter-model` default was one of them, so that
code path could never have succeeded since the day it was written. Its comment
claimed the model had "both vision and structured_outputs"; the catalogue
disagreed, and free `:free` ids churn fast enough that any such claim rots.

A decision is needed because the failure is not incidental: the free tier is
the *only* key-free-adjacent option for this project, and structured output is
not something the free tier reliably offers.

## Decision

We will treat "this model rejects schemas" as a **recoverable condition handled
inside `run_llm`**, not as a model-selection problem.

`publicator/llm_meta.py`:

1. Match the literal marker `NO_SCHEMA_SUPPORT = "does not support schemas"`
   against the failed call's stderr.
2. On a match, re-run the same argv **without** `--schema`, appending the schema
   itself to the prompt (`_JSON_FALLBACK`) so the shape is still communicated.
3. Parse the reply with `_loads_json`, which takes the outermost `{...}` and so
   tolerates the markdown fences and prose a schema-less model adds.
4. Record the verdict in a module-level `_NO_SCHEMA` set, since schema support
   is a static property of a model — subsequent images in the same process skip
   `--schema` outright.

The retry lives in `run_llm` because that is the only function that owns the
`--schema` argv. Every caller — the gallery's `/ai` endpoint, `generate-meta`,
any future entry point — inherits the behaviour without knowing about it.

## Alternatives Considered

### A. Pin a free model that does support structured outputs

Change `OPENROUTER_MODEL` to a model whose `supported_parameters` include
`structured_outputs` and keep `--schema` unconditional.

**Rejected.** On 2026-08-23 exactly one free vision model qualified
(`dots-studio/dots-3-note-preview:free`), and it is a preview. This is also the
precise failure mode that produced the bug: the previous default was chosen the
same way and silently stopped qualifying. Pinning trades a permanent fix for a
lookup that must be redone every time the free tier churns — and the symptom of
it going stale is a hard failure in the gallery, not a warning.

### B. Never send `--schema` on the attachment path

`attach is not None` already means "API-key provider", so the code could simply
omit `--schema` there and always ask for JSON in the prompt. Deletes the
constant, the retry, and one test (~12 lines).

**Rejected**, though it is the smallest diff. It gives up genuine structured
output on the minority of OpenRouter models that do support it, and on any
future provider reached through the same attachment path. The retry is four
lines and strictly more capable; the cost of keeping it is one process spawn on
models that would have failed anyway.

### C. Query `supports_schema` through the `llm` Python API

`llm` is importable (it is a declared dependency and in the flake's Python env),
and `model.supports_schema` is a real boolean. Check it before building argv.

**Rejected on three counts.** For `openrouter/*` that attribute only exists
after `register_models()` runs, and that performs an HTTP GET of
`openrouter.ai/api/v1/models` behind a one-hour disk cache — so the "cleaner"
check is network-dependent and up to an hour stale. It would also drag plugin
loading into publicator's own process, abandoning the deliberate shell-out
design, and it would bypass the `run=` seam that every unit test in
`tests/test_llm_meta.py` injects through.

### D. Probe with `llm models --schemas -m <id>`

A CLI-only way to ask the same question, consistent with the shell-out design.

**Rejected.** It is still a subprocess spawn, but paid on *every* call including
the happy path, and it requires parsing a human-readable listing where empty
output means "unsupported". That is string-matching a *less* stable surface than
an exception message, for strictly more cost. The retry only pays its extra
spawn after a call that already failed.

### E. Put the retry in `generate_metadata` or in each caller

**Rejected.** `run_llm` builds the argv; hoisting the retry above it means
reconstructing argv outside the function that owns it, and both
`webui/server.py` and `apps/generate_meta.py` would need their own copy. One
guard where all callers already route through is the root-cause placement.

## Consequences

### Positive

- Works with *any* model regardless of its `structured_outputs` support, so the
  free tier churning no longer breaks metadata generation.
- The fix is inherited by every caller, present and future, because it sits
  below them all.
- Costs zero API tokens: the guard is client-side, so the failed attempt is a
  process spawn and nothing else.
- The `_NO_SCHEMA` memo bounds that spawn to once per model per process, rather
  than once per image — relevant for a `generate-meta` batch and for a
  long-lived gallery session.

### Negative / Trade-offs

- **We string-match a third-party error message.** If `llm` reworded it, the
  fallback would stop firing. Mitigations: the string is a fixed f-string in
  `llm/models.py`, `llm` is nixpkgs-pinned (0.31.1), so a bump is a deliberate
  act, and the failure mode is a loud `RuntimeError` carrying llm's real stderr
  — never a silently wrong answer.
- A prompt-instructed shape is a weaker guarantee than a provider-enforced one.
  `_loads_json` compensates by extracting the outermost object, and
  `generate_metadata` still raises when `title`/`description` come back empty.
- Worst-case wall clock for a schema-rejecting model is `2 × timeout`, since
  both invocations get the full budget. In practice the first dies client-side
  in about a second, so this is theoretical.
- `_NO_SCHEMA` is process-global mutable state. Tests must isolate it
  (`monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())`) or leak verdicts
  between cases.

### Neutral

- `OPENROUTER_MODEL` stays a module constant in `llm_meta.py` rather than moving
  to `publicator.toml`. It is an implementation default with a CLI override on
  both entry points, and `generate-meta` is defined by running with **no data
  dir** — a TOML key would be unreachable for exactly the entry point that most
  needs it. This is deliberately *not* the same case as the filename grammar of
  [[0003]], which is a contract with a counterparty living in the data repo.
- `apps/generate_meta.py` (commit `4de25b3`) ships alongside this but is not its
  own ADR: it is a thin entry point onto `generate_metadata` with no rejected
  alternatives worth recording.

## Maintenance

### If revisited

1. If `llm` is unpinned or bumped, re-check that the message at
   `llm/models.py:691` still contains `does not support schemas`; the live test
   is what catches a drift.
2. If OpenRouter's free vision tier gains broad `structured_outputs` support,
   the fallback becomes dormant rather than wrong — no action needed.
3. If a *third* provider is added, confirm its schema-rejection error also
   surfaces on stderr with a non-zero return code.

### How to verify

```bash
# offline: the retry, the memo, and the no-retry-on-other-errors guard
nix develop -c pytest tests/test_llm_meta.py -q

# live: real generation through the fallback (needs $OPENROUTER_KEY)
nix develop -c pytest tests/test_llm_meta_openrouter.py -q

# the gate itself: online with no key must be RED, not skipped
env -u OPENROUTER_KEY nix develop -c pytest tests/test_llm_meta_openrouter.py -q

# end to end through the app
nix run <this>#generate-meta -- --openrouter path/to/art.png

# which free vision models currently advertise structured_outputs
curl -s https://openrouter.ai/api/v1/models | jq -r '.data[]
  | select(.id|endswith(":free"))
  | select(.architecture.input_modalities|index("image"))
  | [.id, (.supported_parameters|index("structured_outputs")|if .==null then "NO-SCHEMA" else "schema" end)]
  | @tsv'
```

### Gotchas

- **Do not "fix" a future schema failure by hunting for a model id that supports
  structured outputs.** That is alternative A, it is what caused this bug, and
  the free tier will churn out from under it again.
- The failed first call is *free* — it never reaches the network. Do not
  "optimise" it away by pre-flighting a support check (alternatives C and D);
  both cost more than the thing they avoid.
- `_NO_SCHEMA` is module state. A test that exercises the retry must patch it or
  it will poison later tests that expect `--schema` in argv.
- The free pool is independently flaky (429s, upstream idle timeouts) and those
  are *not* schema failures — they must not be retried by this path. The live
  test retries them at its own level instead.
- `_loads_json` is deliberately permissive about fences and prose but still
  raises on genuinely unparseable output; do not soften it into returning `{}`.

### Related

- Commits: `0e88998` (the fallback), `4de25b3` (the `generate-meta` entry point)
- Branch: `master`
- ADRs: [[0003]] — contrasted, not superseded: that ADR moves the filename
  grammar *out* of code into `publicator.toml`; this one deliberately keeps a
  model id *in* code, because it has no data-dir counterparty.
