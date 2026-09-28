# 0002. Resolve image→prompt to a three-state type, never a nullable string

| Field    | Value                                     |
|----------|-------------------------------------------|
| Date     | 2026-08-21                                |
| Status   | Accepted                                  |
| Deciders | theta (repo owner)                        |
| Branch   | `master`                                  |
| Commit   | `7408cf4`                                 |

## Context

[[0001]] fixes the write side going forward, but 363 of 421 referenced prompt
versions are permanently lost and will never resolve. Prompt lookup is therefore
*intrinsically* uncertain, not transiently so — uncertainty is a permanent
feature of this feature, not a bug to be engineered away.

The filename carries two things of very different strength:

```
mommy_tentacles _ d346b72547a97aed…fddbd3 _ ccf13edf-…-4a729c4a9df6 .webp
└── LINEAGE ──┘   └──── VERSION (strong) ──┘   └──── ignored ─────┘
    weak hint         content-addressed,
    may be stale,     exact, immutable
    renamed, ambiguous
```

- **Version** — a content digest. Identifies exactly which prompt text.
- **Lineage** — identifies which logical prompt *file*. Weak: files get renamed,
  the same basename exists in several directories (`mommy_tentacles` appears in
  three), and two files can hold identical content.

The measured danger is that approximate answers are indistinguishable from exact
ones at a glance. The naive `basename → HEAD` lookup is silently wrong on 404 of
426 pairs: it returns a plausible, well-formed, incorrect prompt. Observed cases
include an image named `mommy_tentacles_…` whose digest matches a blob stored at
`Heartsync__adult/topless_mommy_tentacles`, and images named `hot_mommy.json_…`
whose basename does not exist at `HEAD` at all.

The user decided that near-misses should still be shown (they are useful), which
makes the conflation risk permanent and structural rather than avoidable.

## Decision

We will represent the result of resolution as three distinct types, and
`Nearest` will carry **no prompt text field at all**:

```python
@dataclass(frozen=True)
class Exact:
    version: PromptVersion               # .text is the prompt. Certain.

@dataclass(frozen=True)
class Nearest:
    candidates: tuple[Lineage, ...]      # ranked, never filtered. No .text.

@dataclass(frozen=True)
class Unknown:
    reason: str

PromptMatch = Exact | Nearest | Unknown
```

The ambiguity lives in the type. A caller cannot render a near-miss as if it
were the real prompt, because it does not have the field to render. This is the
primary anti-conflation device and it must survive into the UI: `Nearest` is
rendered with warning styling and an explicit sentence, and prompt-text search
(`?prompt=`) matches `Exact` only.

`Nearest.candidates` is **plural** because the data demands it. Ranking uses
`difflib.SequenceMatcher` over normalized directory names — it **orders and
never discards**.

## Alternatives Considered

### `resolve() -> str | None`

The obvious signature: the prompt text, or `None` if not found.

Rejected: it has only two states for a three-state problem, so "near-miss" must
be encoded either as `None` (throwing away useful information the user asked
for) or as a `str` (making it indistinguishable from an exact hit at every call
site downstream). The second is precisely the 404-silently-wrong failure, moved
from the git lookup into the type system.

### `(text: str, confidence: Literal["exact", "nearest"])`

Return the text plus a flag the caller is expected to check.

Rejected: the flag is ignorable and the text is always right there. Every call
site, every template, every future refactor is one forgotten `if` away from
displaying an approximate prompt as authoritative. Correctness that depends on
remembering a convention is not correctness. Making the field *absent* is
enforced by the language; a flag is enforced by discipline.

### Single best guess via fuzzy matching

Rank candidates and return the winner, hiding the rest.

Rejected: silently picking one winner is the exact bug class this design exists
to eliminate. Ranking is retained as an *ordering* over `Nearest.candidates`,
but it never filters, and the plural is preserved all the way into the UI.

## Consequences

### Positive

- Conflating exact and approximate prompts becomes a type error rather than a
  code-review question.
- The model absorbed a late reversal of [[0004]] without any change to the read
  side, because it was already built to turn dangling references into
  `Nearest`/`Unknown` — 363 of them already exist.
- A grammar with no `lineage` group ([[0003]]) needs no special case: such a
  repository simply yields only `Exact` or `Unknown`.

### Negative / Trade-offs

- Every consumer must destructure three cases; there is no one-line
  "just give me the text" path. This is intentional friction.
- Prompt-text search covers `Exact` only, so today 86 % of images are
  unsearchable by prompt content. The result header states the excluded count
  rather than quietly shrinking the result set.
- Rendering `Nearest` well costs real UI work — a warning treatment plus a
  candidate list — for an answer that is explicitly not the answer.

### Neutral

- `PromptVersion.paths` is a tuple: content-addressing genuinely collapses two
  prompt files that ever held identical text, and renames add historical paths.
  This does not weaken `Exact` — the *text* is certain regardless of which file
  it was read from; only the label is plural.

## Maintenance

### How to verify

```bash
nix develop -c pytest tests/test_prompts.py tests/test_prompt_view.py -q
nix develop -c pytest -q            # full suite must stay green
```

The load-bearing test: commit a prompt, edit it, commit again, resolve the
**old** digest, assert the **old** text comes back. That is the 404-silently-
wrong case frozen into a test.

### Gotchas

- Follow the existing one-way import rule: `webui` imports `prompts`, never the
  reverse. Same discipline as `deviantart.py`.
- Lineage lookup must search paths across **all history**, not `HEAD` —
  otherwise `hot_mommy.json`, which no longer exists at `HEAD`, resolves to
  nothing.
- Cache the index on `git rev-parse HEAD`. That key is sound only because of
  [[0001]]: every archive write is a commit, so a moving HEAD is a complete
  invalidation signal. A failed archive write adds nothing and moves nothing, so
  the cache stays correct there too.
- `find_candidates` shuffles and caps. Filtering *after* sampling returns
  near-nothing — when a filter is active, bypass the sample and order the work
  `parse → resolve → filter → cap at SEARCH_LIMIT → drop already-published (sha512)`,
  so the expensive sha512 hashing runs only on the capped survivors. Do not modify `find_candidates` itself.
- Do not let the ranking heuristic filter. It orders `Nearest.candidates`; the
  plural reaches the UI intact.

### Related

- Commits: `7408cf4` (spec), `89592f7`; implemented in `e590215`, `032c896`, `f46bcd2`, `bbf7df8`, `8790179`
- Branch: `master`
- Feature doc: `docs/features/prompt-mapping.md`
- ADRs: [[0001]] write side, [[0003]] grammar ownership, [[0004]] why dangling refs stay possible
