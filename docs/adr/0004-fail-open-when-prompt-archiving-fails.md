# 0004. Fail open when prompt archiving fails — never block the image

| Field    | Value                                     |
|----------|-------------------------------------------|
| Date     | 2026-08-21                                |
| Status   | Accepted                                  |
| Deciders | theta (repo owner)                        |
| Branch   | `master`                                  |
| Commit   | `89592f7`                                 |

## Context

[[0001]] adds a `git add` + `git commit` to `identify_image.sh` before the
digest is computed. That raises a question it does not answer: what happens when
the commit cannot be made — the prompt is gitignored, the file is outside a
repository, the index is locked, the repo is mid-rebase?

The first draft failed closed: refuse to mint an identity, on the reasoning that
a missing image beats a dangling pointer.

That reasoning was wrong here, and the caller is why. `rename_image.sh` operates
on a fixed path:

```sh
downloaded_image_path="${HOME}/Downloads/image.webp"
new_image_identity=$(./identify_image.sh $1)
mv ${downloaded_image_path} ${new_image_path}
```

If `identify_image.sh` exits non-zero, the `mv` never happens and the image
stays at `~/Downloads/image.webp` — the exact path the *next* generation writes
to. Fail-closed does not preserve the image; it queues it for silent
destruction.

The owner stated the priority explicitly: **image > prompt > link between them**.
A lost link is an inconvenience. A lost image is unrecoverable work.

## Decision

We will make every archive guard warn and continue. `identify_image.sh` always
mints an identity and always exits 0:

```sh
warn() { echo "$*" >&2; }   # stderr: stdout is captured as the filename
```

Warnings go to **stderr**, because `rename_image.sh` captures stdout as the
filename — a warning on stdout would be spliced into the image's name.

The messages distinguish recoverability: a gitignored prompt can *never* be
archived, while a locked index or dirty repo is transient and the digest will
resolve retroactively once that content is committed.

The invariant of [[0001]] is downgraded accordingly, from *"a minted digest can
never dangle"* to **"the archive write is attempted in the same operation that
mints the reference"**.

## Alternatives Considered

### Fail closed (the first draft)

Refuse to mint; require a healthy repo.

Rejected on the caller's behaviour, as above: it converts an archive failure
into image loss, inverting the owner's stated priority. It also makes the
generation workflow hostage to unrelated git state, which is a poor trade for a
feature whose whole subject matter is a nice-to-have link.

### Fail closed, but copy the image somewhere safe first

Preserve the image under a fallback name, then refuse.

Rejected: it needs `identify_image.sh` to know about the image at all, which it
currently does not — it takes a *prompt* path and prints a name. Coupling the
identity minter to file management to work around a self-inflicted failure mode
is more machinery than simply not failing.

### Prompt the user interactively on failure

Ask whether to continue.

Rejected: the script is called non-interactively from `rename_image.sh` and its
stdout is captured. A prompt would hang or corrupt the filename.

## Consequences

### Positive

- An image is never lost to git trouble.
- Most failures are **self-healing**: the content is still in the working tree,
  so the next successful run over an unchanged file archives it and the digest
  resolves retroactively. Content is lost only if that file is edited before any
  commit ever succeeds.
- No read-side change was needed. [[0002]]'s model already resolves dangling
  references to `Nearest`/`Unknown` — 363 of them already exist. Absorbing this
  reversal without touching the resolver is evidence the three-state model was
  the right spine.

### Negative / Trade-offs

- Referential integrity is no longer guaranteed, only attempted. New dangling
  references remain possible, silently, at generation time.
- The cost is small in practice: the 86 % loss measured in [[0001]] came from
  *never attempting* an archive, not from attempts failing. Moving the attempt
  to the right moment closes essentially all of it.
- Warnings on stderr are easy to miss in a shell session; the audit is the real
  safety net.

### Neutral

- `#prompt-audit`'s `nearest` count becomes a genuine health metric rather than
  a guarantee: flat while archiving works, **rising** when git failures are
  dangling new references, and able to **fall** when a previously-failed prompt
  is committed later. A rise is the signal to go read the warnings.

## Resumption (for Agent)

### Current state

Design approved and committed; **implementation pending**. In practice this is
the same single edit as [[0001]] — do not implement fail-closed first.

### Key files / entry points

| File | Role |
|------|------|
| `huggingface_prompts/identify_image.sh` | the guards: warn + continue, never exit non-zero |
| `huggingface_prompts/rename_image.sh` | the caller whose behaviour forced this decision |
| `huggingface_prompts/test_identify_image.sh` | to create: the regression guard |
| `src/publicator/apps/prompt_audit.py` | to create: `nearest` as the health metric |
| `docs/superpowers/specs/2026-08-21-prompt-mapping-design.md` §4, §7, §8 | full spec |

### Next steps

1. Implement §4 of the spec with warn-and-continue in every branch.
2. Add `test_identify_image.sh` asserting, over a `mktemp -d` repo, in each of
   healthy / gitignored / not-a-repo / locked-index cases:
   - exit status is **0**
   - **stdout is exactly one line**, the identity, warnings on stderr only
   - the healthy case leaves the prompt committed
3. Build `#prompt-audit` and record the baseline numbers
   (`exact 885 / nearest 1600` as of 2026-08-21) to compare against later.

### How to verify

```bash
cd "$(mktemp -d)" && git init -q . && printf 'p\n' > p

# healthy
out=$(/path/to/identify_image.sh p); echo "exit=$?"; echo "$out" | wc -l   # 0, 1

# locked index -> must still succeed
touch .git/index.lock
out=$(/path/to/identify_image.sh p); echo "exit=$?"; echo "$out" | wc -l   # 0, 1

# gitignored -> must still succeed
rm .git/index.lock; echo p > .gitignore
out=$(/path/to/identify_image.sh p); echo "exit=$?"; echo "$out" | wc -l   # 0, 1
```

Any non-zero exit, or any stdout line count other than 1, is a regression back
to fail-closed.

### Gotchas

- **Warnings must never touch stdout.** `new_image_identity=$(./identify_image.sh $1)`
  captures it verbatim into the filename. This is a nastier bug than the one
  fail-open avoids, which is why it is a tested property rather than a comment.
- Do not "improve" this by adding an exit code for archive failure. Any non-zero
  exit re-arms the image-loss path in `rename_image.sh`.
- The gitignored case is *permanent*, not transient — `.gitignore` contains
  `prompt`. Say so in the warning; do not imply a retry will help.
- `set -e` in the script would silently reintroduce fail-closed. Do not add it.

### Related

- Commits: `89592f7` (this reversal), `7408cf4` (original spec)
- Branch: `master`
- Spec: `docs/superpowers/specs/2026-08-21-prompt-mapping-design.md` §4, §7, §8
- ADRs: refines [[0001]]; relies on [[0002]] absorbing dangling references
