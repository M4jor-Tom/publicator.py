# 0008. Walk the free vision catalogue after any failed model attempt

| Field    | Value                                      |
|----------|--------------------------------------------|
| Date     | 2026-10-10                                 |
| Status   | Accepted                                   |
| Deciders | theta (repo owner)                         |
| Branch   | `master`                                   |

## Context

The previous behavior advanced through OpenRouter's dynamic free vision
catalogue only when a candidate returned HTTP 429. Other failures—including stale requested IDs,
transient transport errors, and malformed structured output—stop the walk even
when other catalogue candidates remain. The model list comes from the same
`llm-openrouter` cache that `llm -m` resolves, so fallback IDs are usable without
hardcoding a list. The gallery should try those alternatives after any failed
candidate attempt while keeping one overall timeout budget.

## Decision

We will have `run_llm` continue to the next candidate for any failed OpenRouter
attempt: a non-zero `llm` exit, a runtime/timeout error, unparseable structured
output, or metadata missing its required title/description. It stops at the first
successful validated response or when the whole-call deadline is spent. If all
candidates fail, it raises the last failure
with its existing classification (`ProviderUnavailable` only for recognized
transport/HTTP-unavailable errors). Candidate listing remains lazy and fail-open.

This changes only the model walk for `openrouter/*` requests. Claude and local
Ollama remain explicit provider choices; this decision does not add automatic
cross-provider fallback.

## Alternatives Considered

### Continue only on 429

**Rejected.** A different candidate may work after any candidate-level failure,
including a stale selected ID or a transient error. The user explicitly requested
fallback beyond 429s.

### Retry other providers automatically

**Not included.** That would silently change the selected provider and its
credentials/runtime requirements. Provider selection remains explicit in the
gallery.

### Remove the shared deadline

**Rejected.** Exhausting a long candidate list must not multiply the configured
per-image timeout.

## Consequences

### Positive

- OpenRouter requests can recover from more than rate limits without hardcoded
  model IDs.
- The success path still incurs no catalogue listing or extra model call.

### Negative / Trade-offs

- Configuration failures such as a missing API key are repeated for each
  candidate, although total time remains bounded by the configured deadline.
- A fallback may return a different model's style; the existing INFO log exposes
  when that happens.

### Neutral

- After exhaustion, the final error remains visible; only recognized transport
  and HTTP failures qualify for the existing deferred-generation action.

## Resumption (for Agent)

### Current state

Implemented in `run_llm`; unit tests cover a non-429 error, timeout, malformed
structured output, and missing metadata followed by a successful fallback. This replaces the earlier 429-only behavior.

### Key files / entry points

| File | Role |
|------|------|
| `src/publicator/llm_meta.py` | Candidate walk, total timeout, error classification |
| `tests/test_llm_meta.py` | Candidate ordering, retries, timeout, and error tests |
| `docs/features/ai-metadata.md` | User-facing flow and invariants |

### Next steps

1. None; this decision is implemented.

### How to verify

```bash
nix develop -c pytest tests/test_llm_meta.py -q
```

### Gotchas

- Candidate discovery depends on the installed `llm-openrouter` cache. If it
  cannot be read, the original model failure remains the result.
- The timeout applies to the whole walk, not separately to each model.
- `ProviderUnavailable` remains narrowly classified; retrying a configuration
  error does not make it eligible for “Mark for later.”

### Related

- [[0005]] — schema fallback remains inside `_attempt`.
