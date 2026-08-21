# 0003. Declare the image filename grammar in `publicator.toml`, not in publicator

| Field    | Value                                     |
|----------|-------------------------------------------|
| Date     | 2026-08-21                                |
| Status   | Accepted                                  |
| Deciders | theta (repo owner)                        |
| Branch   | `master`                                  |
| Commit   | `7408cf4`                                 |

## Context

`publicator.py` is code only; the publication *data* lives in a separate data
directory (`../Art`) alongside the `huggingface_prompts` submodule. The naming
convention `<basename>_<sha1>_<uuid>.<ext>` is invented and produced by
`identify_image.sh` inside that data world. It is not publicator's convention.

Hardcoding a regex for it in `prompts.py` would put a data-repo concern in the
code repo, in direct contradiction of the split already documented in
`CLAUDE.md`, and would make any other art repo — or a future change to the
convention — a code change.

The question is where to draw the agnosticism boundary. Publicator cannot be
*fully* agnostic: it must know which part of a filename identifies the prompt
text, or it cannot look anything up. What it must not know is the *shape* that
carries that information.

## Decision

We will have the art repo declare its own grammar in `publicator.toml`, and have
publicator depend only on named **roles**:

```toml
[prompts]
repo = "huggingface_prompts"
filename = '^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[^.]+$'
version_hash = "sha1"
```

| Key | Required | Meaning to publicator |
|---|---|---|
| `repo` | yes | prompts git repo, resolved against the data dir like `tags` |
| group `version` | yes | key into the archive — *which exact prompt text* |
| group `lineage` | no | weak hint — *which logical prompt file* |
| `version_hash` | yes | digest the art repo used, so publicator hashes archive contents the same way |

`version_hash` is part of the contract because publicator digests blob contents
to build its index; hardcoding `sha1` would leave a second piece of grammar
buried in the code. `hashlib.new(cfg["version_hash"])` removes it.

The UUID is not captured — publicator has no use for it, so `ImageIdentity`
carries two fields, not three.

Validated at config load (a trust boundary), not at first lookup: the pattern
compiles, a `version` group exists, `version_hash` is in
`hashlib.algorithms_available`. A missing `[prompts]` section turns the feature
off — every lookup returns `Unknown`, nothing raises.

## Alternatives Considered

### Hardcode the regex in `prompts.py`

Simplest to write; the pattern is right next to the code that uses it.

Rejected: it puts a data-repo convention in the code repo, contradicting the
code/data split this project is organised around, and makes the convention
unchangeable without a release.

### A template mini-language: `"{lineage}_{version}_{instance}"`

More readable than a regex; no regex knowledge required of the config author.

Rejected: it would mean inventing, documenting and maintaining a parser plus
greediness rules — `{lineage}` itself contains `_` in every real filename
(`topless_mommy_tentacles`), so the template needs a disambiguation rule that a
regex already expresses precisely. More code, more edge cases, to express
strictly less. `re` named groups already *are* this feature.

### Fixed separator + ordered field list

```toml
separator = "_"
fields = ["lineage", "version", "instance"]
```

Rejected: it happens to work for today's grammar via `rsplit("_", 2)`, but bakes
in assumptions — separator-delimited, fixed field count, lineage-first — that
are not part of the contract. It is under-general in exchange for no real
simplicity gain.

### Derive the grammar from `identify_image.sh`

Single source of truth: the script that produces the names also defines them.

Rejected: it means parsing shell, from another repository, at runtime. That is
strictly worse than the drift problem it solves.

## Consequences

### Positive

- Publicator never knows the shape of a filename, only the roles it needs.
- Another art repo with a different convention works by configuration alone.
- Tests can exercise a deliberately different grammar
  (`prompt-<version>.png`, `version_hash = "sha256"`, no `lineage` group),
  which *proves* agnosticism rather than asserting it.
- Dropping the unused UUID capture simplified `ImageIdentity` from three fields
  to two.

### Negative / Trade-offs

- **Two-place truth**: `identify_image.sh` *produces* the grammar while
  `publicator.toml` *declares* it, in two different repositories. They can drift
  and nothing can prevent it.
- The config author must write a Python `re` pattern with named groups — a real,
  if small, barrier compared to a template string.

### Neutral

- Drift is made loud rather than prevented: `#prompt-audit` reports
  `parsed : N of M files` as its first line, which drops to `0` if the grammar
  stops matching. Unmissable.
- A grammar without a `lineage` group is legal and needs no special case in the
  resolver — see [[0002]].

## Resumption (for Agent)

### Current state

Design approved and committed; **implementation pending**. `publicator.toml` in
`../Art` does not yet have a `[prompts]` section.

### Key files / entry points

| File | Role |
|------|------|
| `src/publicator/config.py` | `load_config` — add `[prompts]` load + validation (~25 lines) |
| `src/publicator/prompts.py` | to create: consumes the compiled pattern and `version_hash` |
| `src/publicator/apps/prompt_audit.py` | to create: reports `parsed : N of M` (drift detector) |
| `../Art/publicator.toml` | data repo: add the `[prompts]` section |
| `docs/superpowers/specs/2026-08-21-prompt-mapping-design.md` §3, §7 | full spec |

### Next steps

1. Extend `load_config` with a `prompts` key; validate pattern, `version` group,
   and `version_hash` there, raising `ValueError` that names the problem.
2. Add the `[prompts]` section to `../Art/publicator.toml` (data repo, commit separately).
3. Consume it in `prompts.py`; never reference `sha1` or the filename shape in code.
4. Add `apps/prompt_audit.py` + the flake app entry, with `parsed` as line one.

### How to verify

```bash
nix develop -c pytest tests/test_config.py tests/test_prompts.py -q
cd ../Art && nix run <this>#prompt-audit     # `parsed` must be non-zero
rg -n 'sha1|_\[0-9a-f\]\{40\}' src/publicator/   # must return nothing
```

That last command is the real check: any filename-shape literal left in `src/`
means the abstraction leaked.

### Gotchas

- `repo` resolves against the data dir / `--data-dir`, exactly like `tags` —
  not via `__file__`. See the code/data split in `CLAUDE.md`.
- Validate at config load, not at first lookup. A bad pattern discovered while
  rendering a gallery card is far harder to diagnose than one that fails at startup.
- Use TOML literal strings (single quotes) for the pattern so backslashes need
  no escaping.
- `load_config` currently returns a flat dict of plain values; the compiled
  pattern is a live object. Keep the compile in `config.py` so validation and
  compilation stay in one place.
- Do not add a fallback default grammar. A missing `[prompts]` means the feature
  is off, not that publicator guesses.

### Related

- Commits: `7408cf4` (spec), `89592f7`
- Branch: `master`
- Spec: `docs/superpowers/specs/2026-08-21-prompt-mapping-design.md` §3, §7
- Prior art: `docs/superpowers/specs/2026-08-04-publicator-toml-config-design.md`
- ADRs: [[0001]] write side, [[0002]] the roles this grammar feeds, [[0004]]
