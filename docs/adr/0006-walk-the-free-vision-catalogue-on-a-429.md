# 0006. Walk the other free vision models when OpenRouter rate-limits, instead of failing the call

| Field    | Value                                     |
|----------|-------------------------------------------|
| Date     | 2026-09-29                                |
| Status   | Accepted                                  |
| Deciders | theta (repo owner)                        |
| Branch   | `master`                                  |
| Commit   | `8fae9e2`                                 |

## Context

Generating a title + description from the gallery failed outright:

```
llm failed (1): Error: Error code: 429 - {'error': {'message': 'Provider returned error',
 'code': 429, 'metadata': {'raw': 'google/gemma-4-26b-a4b-it:free is temporarily
 rate-limited upstream. ...', 'provider_name': 'Google AI Studio', 'is_byok': False,
 'limit_source': 'upstream_provider_shared_pool', ...}}}
```

Three fields in that body determine the shape of the fix:

- `limit_source: upstream_provider_shared_pool` — the exhausted quota belongs to
  the pool OpenRouter shares between *every* free-tier user and the vendor.
- `is_byok: false` — OpenRouter called the vendor on its own credentials.
  `$OPENROUTER_KEY` authenticates our request but buys no quota on a `:free` id.
- `provider_name: Google AI Studio` — saturation is per upstream **vendor**, so
  `google/gemma-4-31b-it:free` shares the exact bucket that just refused us.

The wait is therefore not ours to shorten: no backoff empties someone else's
bucket. The condition is "this vendor is full right now" — a model-*selection*
problem, not a timing one.

Until now `run_llm` raised on any non-zero exit that was not [[0005]]'s schema
rejection, so the gallery card showed OpenRouter's raw JSON body and the image
got no metadata. The production default (`OPENROUTER_MODEL`) had been 429ing
since at least 2026-09-28, when commit `a9d9500` recorded the same symptom in
passing while fixing a different churn problem.

Pinning a better id is not a fix, because `:free` ids churn on two independent
axes:

- **Disappearance.** `nvidia/nemotron-nano-12b-v2-vl:free` left the catalogue
  within five weeks and `llm` started answering "Unknown model" (`a9d9500`).
- **Saturation.** An id that still exists can be unusable for a day, which is
  this case.

Meanwhile there are always alternatives: 8 free vision models on 2026-08-23
([[0005]]) and 8 again on 2026-09-29, spread over five vendors. A 429 on one of
them says nothing about the other seven.

## Decision

We will treat an OpenRouter 429 as **a recoverable condition handled inside
`run_llm`**, exactly as [[0005]] treats a schema rejection: the failed model is
replaced, never re-waited.

`publicator/llm_meta.py`:

1. Match `RATE_LIMITED = "Error code: 429"` against the failed call's stderr,
   the way `NO_SCHEMA_SUPPORT` is matched. (The text is the `openai` SDK's
   `f"Error code: {status} - {body}"` in `openai/_base_client.py`, relayed
   through `llm-openrouter` — not llm's own wording.)
2. `_fallbacks(model, run)` yields the requested model first, then — only for
   `openrouter/` ids — every other free vision id. It is a **generator**, so the
   happy path never pays for the listing.
3. Get that list from the **installed `llm-openrouter` plugin**, by running
   `llm openrouter models --free --json` through the same injected `run` seam
   every other call uses — *not* by fetching `openrouter.ai/api/v1/models`
   ourselves. See alternative G: the plugin's `register_models()` builds the ids
   `llm -m` will accept from the identical `fetch_cached_json` copy, so anything
   it lists is guaranteed to resolve.
4. `free_vision_models(catalogue)` is a **pure** function adding the vision
   clause `--free` cannot express: `"image" in architecture.input_modalities`.
5. Candidates sharing the rate-limited model's **vendor** sort last, being the
   likeliest to share the saturated upstream pool.
6. `timeout` budgets the **whole call**, not each attempt: `run_llm` holds a
   `deadline`, hands each attempt only the remaining seconds, and stops walking
   once it is spent. Without this the walk would silently turn a 300 s ceiling
   into N x 2 x 300 s on a request the gallery tab is blocking on.
7. `run_llm` loops the candidates, breaking on success or on any non-429 failure,
   and raises the last error when every candidate is rate-limited. A swap logs at
   INFO, so metadata never silently comes from a model the user did not pick.
8. An unlistable or reshaped catalogue logs a warning and yields nothing further,
   so the original 429 surfaces unchanged — fail open, per [[0004]]. The catch is
   narrow (`OSError`, `ValueError`, `KeyError`, `TypeError`) so our own bugs still
   raise instead of silently disabling the rescue.

`_attempt` is a pure extraction of the old `run_llm` body (argv construction plus
the [[0005]] schema retry), giving the loop a single-model unit to call. Nothing
in that inner path changed behaviour.

## Alternatives Considered

### A. Retry the same model with exponential backoff

**Rejected.** `limit_source: upstream_provider_shared_pool` with `is_byok: false`
says the bucket is drained by third parties. Backoff blocks a synchronous `/ai`
request for an unbounded wait with no reason to expect the next attempt to land,
and the observed outage lasted more than a day.

### B. Pin a different `:free` id in `OPENROUTER_MODEL`

**Rejected.** This is the manual step the change exists to delete, and [[0005]]
alternative A rejected the same move for schema support. It cannot work in
general either: an id that 429s today is fine tomorrow, and an id pinned today
may vanish outright (`a9d9500`).

### C. Hardcode an ordered fallback list of free vision ids

**Rejected**, and not merely as rot: it is *fatal*. Only a 429 continues the
walk, so the first entry that has since disappeared answers "Unknown model" —
which is not a 429 — and the loop breaks and raises **mid-walk**, never reaching
the healthy ids below it. A hardcoded list therefore degrades from "slightly
stale" to "worse than no fallback" the first time any entry churns out, silently,
with no failing test. Asking the plugin costs one cached subprocess, paid only
after a 429 has already happened.

### D. Fall back to the key-free `claude-cli-opus` path

**Rejected** as the general answer, though it is what a human does by hand. It
silently moves an explicit provider choice to the other provider, changes the
prompt shape (Read tool vs attachment), and assumes a logged-in `claude` CLI that
an OpenRouter user may deliberately not have. Staying inside the tier the user
chose keeps the fallback predictable — and the gallery's model select already
makes the cross-provider move a one-click manual choice.

### E. Also walk on `Unknown model` / 404

**Rejected deliberately**, for the *requested* model. A vanished id is a
permanent condition that should stay loud so the constant actually gets updated;
[[0005]]'s maintenance notes already read "Unknown model" as "the id died, pick
another". Folding it into the walk would hide a stale default forever. Note this
is only safe because of decision 3: since candidates come from the same cache
`llm -m` resolves against, a *fallback* id cannot be unknown either.

### F. Put the walk in `generate_metadata` or in `webui/server.py`

**Rejected**, same reasoning as [[0005]] alternative E: `run_llm` owns argv, and
`-m <model>` *is* argv. Hoisting the loop means rebuilding argv outside the
function that owns it, in both callers.

### G. Fetch `openrouter.ai/api/v1/models` ourselves with `urllib`

**Rejected** — this was the first implementation, and it is wrong on four counts.
`llm-openrouter` already exposes the same data as `llm openrouter models --free
--json`, and (i) `register_models()` mints the `openrouter/<id>` set that `llm -m`
accepts from the very same `fetch_cached_json` file, so a live fetch can offer an
id minted in the last hour whose "Unknown model" is not a 429 and aborts the walk
— re-creating alternative C's failure from *fresher* data; (ii) the plugin's cache
is on disk and shared across processes, where a `functools.cache` is per-process,
so every `generate-meta` invocation re-paid the GET; (iii) `fetch_cached_json`
serves its last good copy when openrouter.ai is unreachable, whereas our fetch
raised and left no fallback at all; (iv) a subprocess goes through the `run=`
seam the unit tests already inject, deleting a second stubbing convention.

This does **not** contradict [[0005]] alternative C, which rejected importing
`llm` *in-process*: this is a subprocess, consistent with the shell-out design,
and does not bypass the test seam. Importing `llm_openrouter` directly for
`get_supports_images` would contradict it, and was not taken.

### H. Remember the model that last answered and start the walk there

**Rejected**, though two reviewers reached for it by analogy with `_NO_SCHEMA`.
The analogy breaks: schema support is a *static* property of a model, which is
why memoising it is safe; saturation is transient. The saving is one ~1 s spawn
per image, and the only way to collect it is to stop trying the id the user
explicitly selected. With no expiry, a single blip pins a long-lived
`publish-next` process to another vendor for its whole life, and the model that
recovered is never reconsidered. Paying the spawn is the cheaper mistake.

## Consequences

### Positive

- A saturated free tier no longer ends metadata generation: the call lands on
  another vendor, in practice within about a second.
- Both entry points inherit it (`/ai`, `generate-meta`), as with [[0005]].
- Churn is now self-healing on the saturation axis — the candidate set is
  whatever `llm` itself can resolve today, never a constant that ages.
- Zero cost on the happy path: `_fallbacks` is a generator, so a successful first
  call never lists anything.
- The listing works offline and without a key, because the plugin serves its last
  good copy on an HTTP error.
- `OPENROUTER_MODEL` becomes a *preference* rather than a single point of failure.

### Negative / Trade-offs

- **The model that answers may not be the one that was asked for.** Logged at
  INFO (visible without `-v`), but a `generate-meta` batch can mix voices across
  images. Acceptable only because every title/description is human-reviewed in
  the gallery before it reaches DeviantArt.
- **The `/ai` allow-list now constrains only the *requested* id.**
  `GalleryHandler.do_POST` still rejects anything outside
  `(ai_model, openrouter_model)` with 400, but after a 429 the subprocess may run
  a different id. That id comes from `llm`'s own cache and is always
  `"openrouter/"`-prefixed by our code, and argv is a list (never a shell), so it
  cannot present itself as a flag — but the guard no longer means "this exact
  string reaches `llm -m`". Restated in `docs/features/gallery-ui.md`.
- Eligibility is "free + accepts images", which is broader than "can caption
  art": `nvidia/nemotron-3.5-content-safety:free` currently passes the filter. A
  walk can land on a model that answers unhelpfully; `_loads_json` and the
  empty-field guard catch the malformed cases, human review catches the rest.
- A second third-party message is now string-matched. Same trade-off [[0005]]
  accepted: a rewording stops the fallback firing, and the failure stays a loud
  `RuntimeError` carrying the real stderr rather than a wrong answer. The marker
  lives in the `openai` SDK, a different package from [[0005]]'s — but the same
  nixpkgs closure, so a bump is still a deliberate act.
- Worst case is one `llm` spawn per eligible id, now bounded by the `timeout`
  deadline rather than by the hope that 429s return quickly. Retiring that hope
  also retires [[0005]]'s `2 x timeout` hand-wave: the budget is the budget.
- The plugin's cache is up to an hour stale, so a model added in the last hour is
  not offered as a fallback. That is the correct direction to be stale in — it
  matches what `llm -m` will accept.

### Neutral

- `OPENROUTER_MODEL` stays `openrouter/google/gemma-4-26b-a4b-it:free` even
  though that is the id which 429s. With the walk in place the default is only
  the first candidate, and rewriting it would re-enact alternative B.
- The claude-cli path is untouched: `_fallbacks` returns after the first yield
  for any non-`openrouter/` id, having neither a sibling to move to nor a key to
  reach one — and so never lists models.
- No new dependency. `ps.llm-openrouter` was already in `flake.nix`'s `pyPkgs`;
  this change only stops re-implementing a part of it.

## Maintenance

### Current state

Implemented and green. `src/publicator/llm_meta.py` carries `RATE_LIMITED`,
`FREE_MODELS_ARGV`, `free_vision_models`, `_fallbacks`, `_attempt`, and the
budgeted candidate loop in `run_llm`. Six cases in `tests/test_llm_meta.py` cover
the filter, the vendor ordering, exhaustion, fail-open, the timeout budget, and
the two paths that must *not* walk. Nothing pending.

### How to verify

```bash
# offline: the walk, its order, the budget, fail-open, and the no-walk guards
nix develop -c pytest tests/test_llm_meta.py -q

# live: real generation (needs network + $OPENROUTER_KEY)
nix develop -c pytest tests/test_llm_meta_openrouter.py -q

# end to end; watch stderr for "... answered instead of the rate-limited ..."
nix run .#generate-meta -- --openrouter path/to/art.png

# the candidate list itself — needs no key, and works from cache when offline
nix develop -c llm openrouter models --free --json \
  | jq -r '.[] | select(.architecture.input_modalities|index("image")) | .id'
```

If that last command prints one id, the walk has nowhere to go.

### Gotchas

- **Do not silence the INFO swap line.** It is the only signal that a published
  title came from a model other than the one selected in the gallery.
- The walk fires on 429 **only**. A dead *requested* id still raises at once, on
  purpose — see alternative E.
- Everything stubs through one seam: `run=`. There is no separate catalogue seam
  to patch, and `free_vision_models` takes the parsed list, so it is testable
  with a literal.
- `_fallbacks` catches a narrow tuple, not `Exception`. Widening it back would
  turn a typo in the filter into a silently disabled rescue plus one WARNING
  line — and would make the "never lists models" test unable to fail.
- A candidate that answers with unparseable output ends the walk: `run_llm`
  breaks on `returncode == 0`, and `_loads_json` raises afterwards. Widening the
  walk to cover that would risk burning every candidate on a genuinely bad prompt.
- The budget is enforced between attempts, so one pathological candidate can
  still overshoot the deadline by its own remaining slice. Bounded by roughly one
  `timeout`, not by N of them.

### Related

- [[0005]] — the sibling decision this one follows in shape (recoverable
  condition, handled in `run_llm`, inherited by all callers).
- [[0004]] — the fail-open precedent for a helper that must not become the
  failure it guards.
- `docs/features/ai-metadata.md` — the feature doc, flow step 8 and invariants.
- `docs/features/gallery-ui.md` — the `/ai` allow-list, restated by this change.
- Commit `a9d9500` — the churn fix that first recorded gemma's 429.
