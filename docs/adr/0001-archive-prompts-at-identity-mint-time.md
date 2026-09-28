# 0001. Archive prompts at identity-mint time, using git history as the store

| Field    | Value                                     |
|----------|-------------------------------------------|
| Date     | 2026-08-21                                |
| Status   | Accepted                                  |
| Deciders | theta (repo owner)                        |
| Branch   | `master`                                  |
| Commit   | `7408cf4`, `89592f7`                      |

## Context

Images generated through the `huggingface_prompts` submodule are named by
`identify_image.sh`:

```sh
echo $(basename $1)_$(sha1sum $1 | cut -d ' ' -f 1)_$(uuidgen)
```

The embedded `sha1` is the digest of the prompt file's contents at generation
time. It is a reference to content that the script never archives — a dangling
pointer by construction. Referential integrity is never enforced when the
reference is created, so it can only be hoped for when the reference is read.

An audit of the real data directory (`../Art`) on 2026-08-21 measured the cost:

| | |
|---|---|
| Distinct prompt-content digests referenced by image filenames | 421 |
| …recoverable from `huggingface_prompts` (all refs + reflog + dangling) | 58 (14 %) |
| …permanently lost, never committed | 363 (86 %) |

The loss mechanism is the normal working rhythm: edit a prompt, generate several
images, edit again, generate again, commit once at the end. Every intermediate
version is unreachable forever. No prompt is embedded in the `.webp` metadata,
so there is no secondary recovery channel.

A decision is needed now because the "select an image, see its prompt" feature
cannot be built usefully on a store that holds 14 % of its referents, and any
read-side cleverness would be papering over a write-side defect.

## Decision

We will make the archive write happen in the same operation that mints the
reference. `identify_image.sh` stages and commits the prompt file immediately
before computing its digest:

```sh
git -C "$repo" add -- "$target"
git -C "$repo" diff --cached --quiet -- "$target" \
    || git -C "$repo" commit -q -m "snapshot: ${target#"$repo"/}" -- "$target"
```

**Git history is the archive.** There is no second store. A digest minted after
this change names content that git holds, and the read side resolves it by
digesting every blob in the repository (`git cat-file --batch-all-objects`).

Unchanged content stages clean, so `git diff --cached --quiet` skips the commit:
one commit per actual prompt tweak, not per generated image.

## Alternatives Considered

### `objects/<digest>` sidecar directory in the submodule

`identify_image.sh` copies the prompt to a content-addressed directory that is
committed alongside the prompt files. Reading is a plain `cat` with no git
plumbing.

Rejected: it is a second store that duplicates content git already holds, and
two stores can disagree. It carries no lineage — the file's path history and
renames would need a separate source. Worst, it can be silently corrupted:
nothing enforces that `objects/<d>` actually hashes to `<d>`, so the store can
lie in exactly the way this whole design exists to prevent. Git blobs are
self-verifying; a plain directory is not.

### Append-only JSONL manifest

One `prompts.jsonl` holding `{digest, path, content, timestamp}` per version.
Single file, trivially indexable.

Rejected: it reinvents git, badly. Concurrent appends conflict, content needs
escaping, and it introduces a store that can disagree with the tracked prompt
files sitting next to it in the same repository.

### Leave the write side alone; make the read side clever

Resolve `basename` against `HEAD`, or fuzzy-match prompt text.

Rejected on measurement. Over 426 `(digest, basename)` pairs, the naive
`basename → HEAD` lookup is correct 14 times, **silently wrong 404 times**, and
finds no such basename 8 times. It returns a plausible, well-formed, incorrect
prompt 95 % of the time. Cleverness cannot recover information that was never
recorded; it can only disguise its absence.

## Consequences

### Positive

- No second store, therefore nothing to drift or corrupt.
- Lineage and rename tracking come free: git already records paths across commits.
- Self-verifying — git validates blob contents itself.
- The read side's index cache can be keyed on `git rev-parse HEAD`, because
  every archive write is now a commit. Under a sidecar design that key would
  have been silently stale.
- No migration: history *is* the archive, so the 58 recoverable versions resolve
  on day one. There is no backfill script to write, run, or get wrong.

### Negative / Trade-offs

- Changes the generation habit: every prompt tweak now produces an auto-commit
  with a generic `snapshot: <path>` message, in a repository whose history is
  currently curated by hand (`fix(wuwaifu_tongues): improve prompt`). The log
  gets noisier day to day; squashing or amending later is manual work.
- The 363 already-lost versions are not recovered by this or anything else.
- The read side needs git plumbing rather than a filesystem lookup.

### Neutral

- The write-side change lives in `huggingface_prompts`, a separate GitHub repo
  (`M4jor-Tom/huggingface_prompts`), while the feature lives in `publicator.py`.
  This is a cross-repo change.
- Failure handling of the archive write is a separate decision — see [[0004]].

## Maintenance

### How to verify

```bash
# in a throwaway clone of huggingface_prompts
printf 'a prompt\n' > t/p && ./identify_image.sh t/p          # commits, prints identity
git log --oneline -1                                          # "snapshot: t/p"
./identify_image.sh t/p                                       # no new commit (unchanged)
printf 'edited\n' > t/p && ./identify_image.sh t/p            # new commit

# the digest in the printed identity must be findable:
git cat-file --batch-all-objects --batch-check='%(objectname) %(objecttype)' \
  | awk '$2=="blob"{print $1}' | while read o; do
      git cat-file blob "$o" | sha1sum; done | grep "<digest from the identity>"
```

### Gotchas

- `sha1sum <file>` is **not** the git blob OID — git hashes `"blob <len>\0" + content`.
  The index must digest blob *contents*, never use `git hash-object` output as the key.
- Resolve `$1` to an absolute path before passing it through `git -C <dir>`:
  `-C` makes git interpret pathspecs relative to `<dir>`, not the caller's cwd,
  so a relative `$1` silently addresses the wrong file.
- `.gitignore` contains `prompt`. Generating from that scratch file produces an
  identity that is unarchivable by construction — a permanent failure, unlike
  the transient ones. Warn in those terms.
- The read-side lookup must scan **all** history including unreachable objects
  (`--batch-all-objects`), not just `HEAD`, or superseded versions vanish.

### Related

- Commits: `7408cf4` (spec), `89592f7` (fail-open revision); implemented in `huggingface_prompts` `97aaf63`, `2d4ba4a` and here in `032c896` (read side)
- Branch: `master`
- Feature doc: `docs/features/prompt-mapping.md`
- ADRs: [[0002]] read side, [[0003]] grammar ownership, [[0004]] refines this decision's failure handling
