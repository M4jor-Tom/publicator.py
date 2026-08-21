# Prompt mapping — design

**Date:** 2026-08-21
**Status:** approved, ready for implementation planning

Select an image in the gallery, see the generation prompt that produced it.

## 1. The problem, measured

Images generated through `huggingface_prompts/identify_image.sh` are named
`<basename>_<sha1>_<uuid>.<ext>`, where `<sha1>` is `sha1sum` of the prompt file
at generation time. An audit of the real data dir (`../Art`) on 2026-08-21:

| | |
|---|---|
| Distinct prompt-content sha1s referenced by image filenames | **421** |
| …recoverable from `huggingface_prompts` (all refs + reflog + dangling) | **58 (14 %)** |
| …permanently lost, never committed | **363 (86 %)** |
| Hash-named image files in the data dir | 2 485 |

And the naive implementation — look the basename up at `HEAD` — measured over
426 `(sha1, basename)` pairs:

| Outcome | Count |
|---|---|
| Correct (HEAD content still hashes to the baked sha1) | 14 |
| **Silently wrong** (HEAD has drifted since generation) | **404** |
| Basename no longer exists at HEAD | 8 |

Two conclusions drive the whole design.

**The failure mode is silent wrongness, not breakage.** The naive lookup returns
a plausible, well-formed, *incorrect* prompt 95 % of the time. Observed cases:
an image named `mommy_tentacles_…` whose sha1 matches a blob stored at
`Heartsync__adult/topless_mommy_tentacles`; images named `hot_mommy.json_…`
whose basename does not exist at HEAD at all.

**The root cause is in the write side, not the read side.**
`identify_image.sh` mints a reference to content it never archives:

```sh
echo $(basename $1)_$(sha1sum $1 | cut -d ' ' -f 1)_$(uuidgen)
```

It is a dangling pointer by construction — referential integrity is never
enforced at write time, so it can only be hoped for at read time. Editing a
prompt, generating images, editing again, and committing once at the end
destroys every intermediate version. That is the 86 %.

No prompt is embedded in the `.webp` metadata, so there is no recovery channel
there. The 363 lost versions are unrecoverable by any means.

## 2. Identity model

The filename carries two different things. Conflating them is the bug generator.

```
mommy_tentacles _ d346b72547a97aed…fddbd3 _ ccf13edf-…-4a729c4a9df6 .webp
└── LINEAGE ──┘   └──── VERSION (strong) ──┘   └──── ignored ─────┘
    weak hint         content-addressed,
    may be stale,     exact, immutable
    renamed, ambiguous
```

- **Version** — a content digest. Identifies *exactly which prompt text*. Strong.
- **Lineage** — identifies *which logical prompt file*. Weak: prompt files get
  renamed, the same basename exists in several directories, and two files can
  hold identical content.

Publicator consumes these two **roles**. It does not know the shape that
carries them (§3).

## 3. Grammar belongs to the art repo

Publicator must be agnostic of the filename grammar. The art repo declares it in
`publicator.toml`:

```toml
[prompts]
repo = "huggingface_prompts"
filename = '^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[^.]+$'
version_hash = "sha1"
```

A `re` named-group pattern *is* this feature; a template DSL
(`"{lineage}_{version}_{instance}"`) would mean inventing greediness rules and a
mini-parser in order to express less.

The contract publicator depends on:

| Key | Required | Meaning to publicator |
|---|---|---|
| `repo` | yes | prompts git repo, resolved against the data dir like `tags` |
| `filename` group `version` | yes | key into the prompt archive — *which exact text* |
| `filename` group `lineage` | no | weak hint — *which logical prompt file* |
| `version_hash` | yes | digest the art repo used, so publicator hashes archive contents the same way |

`version_hash` is part of the contract because publicator hashes blob contents
to build its index. Hardcoding `sha1` would leave a second piece of grammar
buried in the code; `hashlib.new(cfg["version_hash"])` removes it.

The UUID is not captured — publicator has no use for it.

A grammar with no `lineage` group is legal: such a repo yields only `Exact` or
`Unknown`, never `Nearest`. The three-state model (§5) absorbs this with no
special case.

**Validation at config load** (trust boundary — fail there, not at first
lookup):

- pattern compiles, else `ValueError` quoting the regex error
- a `version` group exists, else `ValueError` naming the missing role
- `version_hash` in `hashlib.algorithms_available`, else `ValueError`
- `[prompts]` absent entirely → feature off; every lookup returns `Unknown`;
  nothing raises

## 4. Write side — archive at mint time (repo `huggingface_prompts`)

`identify_image.sh` archives the prompt at the moment it mints the reference to
it. Inserted before the existing `echo`:

```sh
warn() { echo "$*" >&2; }   # stderr: stdout is captured as the filename

# $1 may be relative to any cwd; git -C resolves pathspecs against -C, not cwd
target="$(CDPATH= cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
if repo="$(git -C "$(dirname -- "$target")" rev-parse --show-toplevel 2>/dev/null)"; then
    if git -C "$repo" check-ignore -q -- "$target"; then
        warn "WARNING: $1 is gitignored — this image's prompt can never be archived."
    elif ! git -C "$repo" add -- "$target" \
      || { ! git -C "$repo" diff --cached --quiet -- "$target" \
           && ! git -C "$repo" commit -q -m "snapshot: ${target#"$repo"/}"; }; then
        warn "WARNING: could not archive $1 — the link will dangle until this"
        warn "         content is committed. Re-run after fixing the repo, or"
        warn "         commit by hand; the digest resolves retroactively."
    fi
else
    warn "WARNING: $1 is not inside a git repository — prompt will not be archived."
fi
```

**Failure is never fatal.** Priority order is image > prompt > link between
them, so every guard warns and continues; the identity is always minted and
`rename_image.sh` always gets a name to move the file to. Fail-closed would be
actively harmful here: `rename_image.sh` moves a fixed `~/Downloads/image.webp`,
so a non-zero exit strands the image at a path the next generation overwrites —
trading a lost link for a lost image.

Warnings go to **stderr** because `rename_image.sh` captures stdout as the
filename (`new_image_identity=$(./identify_image.sh $1)`); a warning on stdout
would end up inside the image's name.

Both `$1` and the repo root are resolved to absolute paths first: `git -C <dir>`
interprets pathspecs relative to `<dir>`, so passing a cwd-relative `$1` through
it silently addresses the wrong file.

- **The invariant:** the archive write is *attempted in the same operation* that
  mints the reference. Git history *is* the archive — no second store, therefore
  nothing to drift.

  This is deliberately weaker than "a digest can never dangle". The 86 % of §1
  was caused by never attempting an archive, not by attempts failing — moving
  the attempt to the right moment closes essentially all of it. What remains is
  a rare git failure, and that case is usually **self-healing**: the content is
  still in the working tree, so the next successful run over an unchanged file
  archives it and the digest resolves retroactively. Content is lost only if
  that file is edited before any commit ever succeeds.

  The read side needs no adjustment for this: it was already built to resolve
  dangling references into `Nearest` or `Unknown` (§5), since 363 of them
  already exist.
- **Lineage comes free**, including rename tracking, because git already records
  paths across commits.
- **Self-verifying**: git validates blob contents itself.
- **One commit per actual tweak**, not per image — unchanged content stages
  clean and `git diff --cached --quiet` skips the commit. This is the history
  already being written by hand, moved to the moment that makes it true.
- **Fails open**: a dangling link beats a lost image (see above).
- The `check-ignore` guard is specifically load-bearing here: `.gitignore`
  contains `prompt`, so generating from that scratch file mints an identity that
  is unarchivable by construction — permanently, unlike the transient failures.
  It warns rather than refusing, but says so in those terms.

Alternatives rejected: an `objects/<digest>` sidecar (a second store that
duplicates git, carries no lineage, and can be silently corrupted — nothing
enforces that `objects/<d>` hashes to `<d>`); an append-only JSONL manifest
(reinvents git, with append conflicts and content escaping).

## 5. Read side — `src/publicator/prompts.py`

Pure, read-only, derives everything, stores nothing. Imports nothing from
`webui`; `webui` imports it. Stdlib plus a `git` subprocess — no new dependency,
matching how `images.py` shells to imagemagick and `llm_meta.py` to `claude`.

### Types

```python
@dataclass(frozen=True)
class ImageIdentity:
    version: str                 # required by the grammar contract
    lineage: str | None          # None when the grammar declares no lineage group

@dataclass(frozen=True)
class PromptVersion:
    version: str                 # content digest
    text: str
    paths: tuple[str, ...]       # every path this content was ever stored at
    committed: int               # committer timestamp, earliest commit holding it

@dataclass(frozen=True)
class Lineage:
    path: str                                # e.g. "anima_v1/hot_warrior.json"
    versions: tuple[PromptVersion, ...]      # newest first

@dataclass(frozen=True)
class Exact:
    version: PromptVersion

@dataclass(frozen=True)
class Nearest:
    candidates: tuple[Lineage, ...]          # ranked, never filtered

@dataclass(frozen=True)
class Unknown:
    reason: str

PromptMatch = Exact | Nearest | Unknown
```

**`Nearest` deliberately has no `.text`.** The ambiguity lives in the type, so a
caller cannot render a near-miss as if it were the real prompt — it does not
have the field to render. This is the primary anti-conflation device and it must
survive into the UI (§6).

### Index

Built from two `git` calls against the prompts repo:

1. `git cat-file --batch-all-objects --batch` — every blob's content, including
   unreachable ones (free recovery of anything reflog-only or dangling). Each is
   digested with `hashlib.new(version_hash)` to build `version → content`.
2. `git log --all --raw --no-abbrev --format=%ct` — one pass yielding
   `(commit time, blob oid, path)` from the `:<mode> <mode> <old> <new> M\tpath`
   raw lines, which gives `PromptVersion.paths`, `PromptVersion.committed`
   (earliest commit time seen for that blob), and lineage-with-renames.

Held in memory, **cached on `git rev-parse HEAD`**, checked per request. That key
is sound *because* of §4: every archive write is now a commit, so a moving HEAD
is a complete invalidation signal. Under a sidecar-store design it would have
been silently stale.

Scale today: 76 distinct blobs, sub-100 ms. If the repo ever grows enough for
this to matter, the upgrade path is persisting the index keyed on the same HEAD
oid — mark with a `ponytail:` comment naming that ceiling.

### Resolution

```
parse filename with the configured pattern
  no match                          -> not a prompt-bearing image (render nothing)
  version in index                  -> Exact(version)
  lineage is None (no such group)   -> Unknown("version not archived")
  lineage names >=1 path in *any*
    commit, with >=1 known version  -> Nearest(ranked candidates)
  otherwise                         -> Unknown(reason)
```

`Nearest.candidates` is **plural because the data demands it**: `mommy_tentacles`
exists as a prompt in three separate directories. Ranking uses
`difflib.SequenceMatcher` over normalized directory names — lowercase, strip
non-alphanumerics — so `picked/huggingface/Heartsync__NSFW-Uncensored-image`
scores an exact match against prompt dir `Heartsync__NSFW_Uncensored_image`, and
`mrfakename_anima_v1` scores high against `anima_v1`.

**Ranking orders; it never discards.** A fuzzy matcher that silently picks one
winner is exactly the bug class this design exists to eliminate.

Lineage lookup searches paths across *all history*, not `HEAD` — otherwise
`hot_mommy.json`, which no longer exists at HEAD, resolves to nothing.

## 6. UI — `src/publicator/webui/prompt_view.py`

A pure renderer beside `calendar_view.py`, following that precedent rather than
growing `page.py` past its current 434 lines.

Server-rendered `<details>` per gallery card. No JS, no new endpoint —
consistent with this codebase's server-side doctrine. `find_candidates` shuffles
and caps the gallery to a sample, so inline prompt text costs nothing at page
weight.

```
Exact     ▸ prompt · anima_v1/hot_warrior.json · 0e2d420
            <pre>…raw file text…</pre>

Nearest   ▸ prompt NOT ARCHIVED  ⚠
            This exact prompt was never committed and is unrecoverable.
            Other versions of the same prompt file — NOT what made this image:
              anima_v1/hot_warrior.json    3 known versions ▸
              other_dir/hot_warrior.json   1 known version  ▸

Unknown   (nothing rendered)
```

`Exact` and `Nearest` must be visually unmistakable, not merely differently
labelled — `Nearest` carries warning styling and the explicit sentence above.

`PromptVersion.paths` can hold more than one path — content-addressing collapses
two prompt files that ever held identical text, and renames add historical
paths. The summary line shows them **all**, ordered by the same directory
heuristic as §5, not an arbitrary first element. This does not weaken `Exact`:
the *text* is certain regardless of which file it was read from; only the label
is plural.

Prompt bodies render as `<pre>` raw, both plain-text and JSON prompt files: the
JSON files are already pretty-printed on disk, so parsing and re-rendering them
buys nothing.

### Search and grouping

Server-side query params on the gallery tab.

**`?prompt=<substring>`** — matches **`Exact` text only**. Matching `Nearest`
text would return images whose prompt merely resembles the query, which is the
conflation this design exists to prevent. The result header states the cost:

> 142 candidates not searched — prompt not archived.

The gap stays visible instead of quietly shrinking the result set.

**`?lineage=<path>`** — renders **two separate lists**, never merged:

- *from this prompt* — version ∈ that lineage's versions (exact)
- *possibly from this prompt* — lineage hint only, version unarchived

**Sampling interaction.** `find_candidates` shuffles and caps, so filtering
after sampling would return near-nothing. When a filter is active the sample is
bypassed: walk all publicable images, then `parse → resolve → filter → dedupe by
sha512 → cap`. That order matters — sha512 hashing is the expensive step and now
runs only on survivors. `find_candidates` itself is untouched.

## 7. Verification — `#prompt-audit`

A small app (~25 lines over the resolver), matching the existing tiny-app
pattern of `echo-first` / `validate` / `check-steps`.

```
parsed   : 2485 of 11111 files          ← drops to 0 if the grammar drifts
exact    :  885 images /  58 versions
nearest  : 1600 images / 363 versions   ← flat = healthy; rising = archive failing
unknown  :    0
```

Two lines earn their place. `nearest` is how the §4 invariant is proven to hold
months from now without reading any code. Because §4 fails open, this number is
a genuine health metric rather than a guarantee: it stays flat while archiving
works, **rises** when git failures are silently dangling new references, and can
**fall** when a previously-failed prompt is committed later and its digest
resolves retroactively. A rise is the signal to go read the warnings. `parsed` addresses the one honest cost
of §3: `identify_image.sh` *produces* the grammar while `publicator.toml`
*declares* it, in two different repos, and they can drift. Nothing can prevent
that, so it is made loud — drift turns `parsed` to zero, which is unmissable.
Deriving the grammar from `identify_image.sh` instead would mean parsing shell,
which is worse than the problem.

## 8. Testing

`tests/test_prompts.py`, against fixture repos built with `git init` in
`tmp_path` — no submodule, no network, deterministic.

The load-bearing test is **resolution of a superseded version**: commit a
prompt, edit it, commit again, resolve the *old* digest, assert the *old* text
comes back. That is the 404-silently-wrong case of §1 frozen into a test.

Also:

- **Agnosticism** — a fixture using a deliberately different grammar,
  `prompt-<version>.png` with `version_hash = "sha256"` and no `lineage` group,
  resolving `Exact` and `Unknown` correctly. This proves §3 rather than
  asserting it.
- Filename parsing: basenames containing underscores
  (`topless_mommy_tentacles`); non-conforming names yield no match.
- Ambiguous basename across two directories → both candidates returned, ranked
  by the image's own directory.
- A renamed prompt file still resolves via its old basename.
- Config validation: missing `version` group, unknown `version_hash`, and
  uncompilable pattern each raise at load with a message naming the problem.
- Missing `[prompts]` section, and a configured `repo` that does not exist, both
  return `Unknown` without raising.

`tests/test_prompt_view.py` asserts the render distinction: an `Exact` renders
its text; a `Nearest` renders the warning and does **not** render any prompt
body as if it were the image's own.

### Write side

`huggingface_prompts/test_identify_image.sh` — a plain `sh` check in that repo,
guarding the one property that now protects the *image* rather than the link.
Over a throwaway repo in `mktemp -d`, in each of: healthy repo, gitignored
prompt, not-a-repo, and repo made uncommittable (e.g. a stale `index.lock`):

- exit status is **0** in every case
- **stdout is exactly one line**, the identity, with warnings on stderr only —
  a warning leaking to stdout would be spliced into the image's filename
- the healthy case leaves the prompt committed; the failure cases leave the
  identity resolvable to nothing, which is the accepted trade

This is the check that would catch a regression back to fail-closed.

## 9. No migration

Because git history *is* the archive under §4, the 58 recoverable versions
resolve on day one. There is no backfill script to write, run, or get wrong.

## 10. Scope

**Files touched**

| File | Change | Approx. |
|---|---|---|
| `huggingface_prompts/identify_image.sh` | best-effort archive at mint (other repo) | ~+16 |
| `huggingface_prompts/test_identify_image.sh` | new — image-preservation check | ~25 |
| `src/publicator/prompts.py` | new — parse, index, resolve | ~150 |
| `src/publicator/webui/prompt_view.py` | new — pure renderer | ~60 |
| `src/publicator/config.py` | `[prompts]` load + validation | ~25 |
| `src/publicator/webui/server.py` | query params, wiring | ~20 |
| `src/publicator/webui/page.py` | call the renderer | ~10 |
| `src/publicator/apps/prompt_audit.py` + flake app | new | ~25 |
| `tests/test_prompts.py`, `tests/test_prompt_view.py` | new | — |
| `../Art/publicator.toml` | `[prompts]` section (data repo) | +4 |
| `CLAUDE.md` | document the prompt-mapping architecture | — |

**Deliberately not built:** prompt capture into `publications.json`; coupling to
`llm_meta` metadata generation; prompts on the calendar tab; an `objects/`
mirror; a persistent index; any client-side JS. The calendar one is nearly free
later — `publications.json` already stores `files[].basename` in the hash-named
form — but it was not requested.

**Explicitly accepted:** 363 prompt versions are gone and this feature will
never show them. It shows honest emptiness plus labelled near-misses instead.
