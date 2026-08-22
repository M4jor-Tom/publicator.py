# Prompt Mapping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select an image in the publish-next gallery and see the generation prompt that produced it, distinguishing an exact match from a labelled near-miss.

**Architecture:** The art repo's `huggingface_prompts` git history *is* the prompt archive; `identify_image.sh` commits a prompt before minting the digest that references it. Publicator's read side (`prompts.py`) digests every blob in that repo to build a `version → text` index, and resolves an image filename to one of three distinct types — `Exact`, `Nearest`, `Unknown` — so a near-miss can never be rendered as the real prompt. The filename grammar itself is declared by the art repo in `publicator.toml`, so publicator never knows the shape of a filename, only the roles it needs.

**Tech Stack:** Python 3.12 (stdlib only: `re`, `hashlib`, `difflib`, `subprocess`, `dataclasses`), `git` plumbing, `ThreadingHTTPServer`, pytest, Nix flake apps. No new pip dependencies.

**Spec:** `docs/superpowers/specs/2026-08-21-prompt-mapping-design.md`
**Decisions:** `docs/adr/0001`–`0004`

## Global Constraints

- **No new pip dependencies.** `pyproject.toml` dependencies stay `["jsonschema", "playwright", "llm"]`. Stdlib plus `git` subprocess only.
- **Python ≥ 3.12** (`requires-python` in `pyproject.toml`). `hashlib.file_digest`, PEP 604 unions and `match`/`isinstance` narrowing are all available.
- **No client-side JS for this feature.** All rendering and all filtering happen server-side. The served page runs in `firefox --private-window` with resist-fingerprinting; never add browser-side logic.
- **One-way imports:** `webui/*` imports `prompts`, never the reverse. `prompts.py` must not import from `publicator.webui`.
- **Never hardcode the filename grammar** (no `sha1`, no `[0-9a-f]{40}`, no `_`-splitting) anywhere in `src/`. It comes from `config["prompts"]`.
- **`sha1sum <file>` is NOT the git blob OID.** Git hashes `"blob <len>\0" + content`. Always digest blob *contents*.
- **`find_candidates` in `images.py` must not be modified.**
- **Test command:** `nix develop -c pytest -q` (full suite must stay green after every task).
- **Commit style:** Conventional Commits (`feat:`, `fix:`, `test:`, `docs:`, `refactor:`, `build:`).
- **Baseline numbers** measured 2026-08-21 over `../Art`, for comparison in Task 9: 2485 hash-named files, 421 distinct versions referenced, 58 resolvable, 363 lost.

---

### Task 1: Config — the `[prompts]` section

**Files:**
- Modify: `src/publicator/config.py`
- Test: `tests/test_config.py`

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `load_config(cwd)` gains a `"prompts"` key whose value is either `None` (section absent) or a dict `{"repo": str, "pattern": re.Pattern, "version_hash": str}`. Later tasks read exactly these three keys.

- [ ] **Step 1: Write the failing tests**

Add to `tests/test_config.py` — and **update the existing exact-dict test**, which will otherwise break:

```python
import re

def test_load_config_defaults_when_file_missing(tmp_path):
    assert load_config(tmp_path) == {
        "tiers": [], "galleries": [], "publicable": [], "tags": None,
        "schedule": {}, "prompts": None}


PROMPTS_TOML = (
    '[prompts]\n'
    'repo = "huggingface_prompts"\n'
    "filename = '^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_[0-9a-f-]{36}\\.[^.]+$'\n"
    'version_hash = "sha1"\n')


def test_load_config_compiles_the_prompt_grammar(tmp_path):
    (tmp_path / "publicator.toml").write_text(PROMPTS_TOML)
    p = load_config(tmp_path)["prompts"]
    assert p["repo"] == "huggingface_prompts"
    assert p["version_hash"] == "sha1"
    m = p["pattern"].match(
        "hot_warrior.json_0e2d420dcabd86c88cacdf55224f6e3d0e52b615"
        "_bb6ba911-8d16-4d5a-82c4-a46b844863ed.webp")
    assert m and m.group("lineage") == "hot_warrior.json"
    assert m.group("version") == "0e2d420dcabd86c88cacdf55224f6e3d0e52b615"


def test_load_config_accepts_grammar_without_lineage_group(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        "filename = '^prompt-(?P<version>[0-9a-f]{64})\\.png$'\n"
        'version_hash = "sha256"\n')
    p = load_config(tmp_path)["prompts"]
    assert "lineage" not in p["pattern"].groupindex


def test_load_config_rejects_grammar_without_version_group(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        "filename = '^(?P<lineage>.+)\\.png$'\n"
        'version_hash = "sha1"\n')
    with pytest.raises(ValueError, match="version"):
        load_config(tmp_path)


def test_load_config_rejects_unknown_version_hash(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        "filename = '^(?P<version>.+)\\.png$'\n"
        'version_hash = "crc32-of-my-dreams"\n')
    with pytest.raises(ValueError, match="version_hash"):
        load_config(tmp_path)


def test_load_config_rejects_uncompilable_pattern(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\nrepo = "p"\n'
        "filename = '^(?P<version>[unterminated'\n"
        'version_hash = "sha1"\n')
    with pytest.raises(ValueError, match="filename"):
        load_config(tmp_path)


def test_load_config_rejects_missing_repo(tmp_path):
    (tmp_path / "publicator.toml").write_text(
        '[prompts]\n'
        "filename = '^(?P<version>.+)\\.png$'\n"
        'version_hash = "sha1"\n')
    with pytest.raises(ValueError, match="repo"):
        load_config(tmp_path)
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop -c pytest tests/test_config.py -q`
Expected: FAIL — `KeyError: 'prompts'` / dict mismatch on the defaults test.

- [ ] **Step 3: Implement**

In `src/publicator/config.py`, add imports `hashlib` and `re` at the top, then this function above `load_config`:

```python
def _load_prompts(cfg: dict) -> dict | None:
    """[prompts]: the ART repo declares its own image-filename grammar, so
    publicator stays agnostic of it (ADR 0003). We depend on ROLES — a required
    `version` capture (which exact prompt text) and an optional `lineage` one
    (which logical prompt file) — never on the shape that carries them.
    Absent section -> feature off. Validated here, at the trust boundary: a bad
    pattern found while rendering a gallery card is far harder to diagnose."""
    p = cfg.get("prompts")
    if not p:
        return None
    if not p.get("repo"):
        raise ValueError("publicator.toml [prompts]: missing 'repo'")
    if not p.get("filename"):
        raise ValueError("publicator.toml [prompts]: missing 'filename'")
    try:
        pattern = re.compile(p["filename"])
    except re.error as e:
        raise ValueError(f"publicator.toml [prompts].filename: {e}") from e
    if "version" not in pattern.groupindex:
        raise ValueError("publicator.toml [prompts].filename: "
                         "missing required (?P<version>...) capture group")
    algo = p.get("version_hash")
    if algo not in hashlib.algorithms_available:
        raise ValueError(f"publicator.toml [prompts].version_hash: "
                         f"unknown digest {algo!r}")
    return {"repo": p["repo"], "pattern": pattern, "version_hash": algo}
```

Then add one line to the dict `load_config` returns, after `"schedule"`:

```python
        "prompts": _load_prompts(cfg),
```

Also extend `load_config`'s docstring with: `prompts (image-filename grammar + prompt repo, see ADR 0003)`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `nix develop -c pytest tests/test_config.py -q`
Expected: PASS (all tests in the file).

- [ ] **Step 5: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS — nothing else reads the config dict exhaustively.

- [ ] **Step 6: Commit**

```bash
git add src/publicator/config.py tests/test_config.py
git commit -m "feat(config): declare the image-filename grammar in publicator.toml

The art repo owns its own naming convention; publicator depends only on
the version/lineage roles and the digest algorithm. Validated at load."
```

---

### Task 2: Filename parsing and the resolution types

**Files:**
- Create: `src/publicator/prompts.py`
- Test: `tests/test_prompts.py`

**Interfaces:**
- Consumes: `config["prompts"]["pattern"]` from Task 1.
- Produces:
  - `ImageIdentity(version: str, lineage: str | None)`
  - `PromptVersion(version: str, text: str, paths: tuple[str, ...], committed: int)`
  - `Lineage(path: str, versions: tuple[PromptVersion, ...])`
  - `Exact(version: PromptVersion)`, `Nearest(candidates: tuple[Lineage, ...])`, `Unknown(reason: str)`
  - `PromptMatch = Exact | Nearest | Unknown`
  - `parse_identity(filename: str, pattern: re.Pattern) -> ImageIdentity | None`
  - `rank_paths(paths: Iterable[str], near: str) -> list[str]`

- [ ] **Step 1: Write the failing test**

Create `tests/test_prompts.py`:

```python
import re

from publicator.prompts import (
    Exact, ImageIdentity, Lineage, Nearest, PromptVersion, Unknown,
    parse_identity, rank_paths,
)

ART = re.compile(
    r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})"
    r"_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[^.]+$")
NOLIN = re.compile(r"^prompt-(?P<version>[0-9a-f]{64})\.png$")

SHA = "0e2d420dcabd86c88cacdf55224f6e3d0e52b615"
UUID = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"


def test_parse_identity_splits_lineage_and_version():
    got = parse_identity(f"hot_warrior.json_{SHA}_{UUID}.webp", ART)
    assert got == ImageIdentity(version=SHA, lineage="hot_warrior.json")


def test_parse_identity_keeps_underscores_inside_the_lineage():
    got = parse_identity(f"topless_mommy_tentacles_{SHA}_{UUID}.webp", ART)
    assert got.lineage == "topless_mommy_tentacles"
    assert got.version == SHA


def test_parse_identity_returns_none_for_a_plain_filename():
    assert parse_identity("VirtualDesktop.Android-20260403-004503-0.mp4", ART) is None


def test_parse_identity_without_a_lineage_group_yields_none_lineage():
    got = parse_identity(f"prompt-{'a' * 64}.png", NOLIN)
    assert got == ImageIdentity(version="a" * 64, lineage=None)


def test_nearest_has_no_text_attribute():
    """The anti-conflation device of ADR 0002: a near-miss cannot be rendered
    as the real prompt because it has no field to render."""
    near = Nearest(candidates=(Lineage(path="a/b", versions=()),))
    assert not hasattr(near, "text")
    assert not hasattr(near, "version")


def test_exact_exposes_the_prompt_text():
    v = PromptVersion(version=SHA, text="a prompt", paths=("a/b",), committed=7)
    assert Exact(version=v).version.text == "a prompt"


def test_unknown_carries_a_reason():
    assert Unknown(reason="nope").reason == "nope"


def test_rank_paths_prefers_the_directory_matching_the_image():
    paths = ["Heartsync__adult/mommy_tentacles",
             "Heartsync__NSFW_Uncensored_image/mommy_tentacles"]
    got = rank_paths(paths, "picked/huggingface/Heartsync__NSFW-Uncensored-image")
    assert got[0] == "Heartsync__NSFW_Uncensored_image/mommy_tentacles"


def test_rank_paths_never_discards():
    paths = ["a/x", "b/x", "c/x"]
    assert sorted(rank_paths(paths, "somewhere/else")) == sorted(paths)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_prompts.py -q`
Expected: FAIL with `ModuleNotFoundError: No module named 'publicator.prompts'`.

- [ ] **Step 3: Write minimal implementation**

Create `src/publicator/prompts.py`:

```python
"""Image -> generation prompt, resolved against the art repo's prompt git repo.

Read-only and pure-ish: it derives everything from the prompt repository and
stores nothing. `webui` imports this; this imports nothing from `webui`.

Three things keep this honest, and all three are load-bearing (ADRs 0001-0004):

* the filename grammar is CONFIG, not code — publicator knows the `version` and
  `lineage` roles, never the shape that carries them;
* resolution returns one of three distinct types, and `Nearest` deliberately has
  no `.text`, so a near-miss cannot be rendered as the real prompt;
* ranking ORDERS candidates and never discards one — silently picking a winner
  is the exact bug this module exists to prevent.
"""

import difflib
import os
import re
from dataclasses import dataclass


@dataclass(frozen=True)
class ImageIdentity:
    """What a filename claims. `lineage` is None when the grammar has no such
    group — a legal configuration, which then yields only Exact or Unknown."""
    version: str
    lineage: str | None


@dataclass(frozen=True)
class PromptVersion:
    """One exact prompt text, as archived. `paths` is plural because content
    addressing collapses two files that ever held identical text, and renames
    add historical paths. `committed` is 0 for blobs reachable only as
    unreferenced objects (recovered, but with no commit to date them)."""
    version: str
    text: str
    paths: tuple[str, ...]
    committed: int


@dataclass(frozen=True)
class Lineage:
    path: str
    versions: tuple[PromptVersion, ...]     # newest first


@dataclass(frozen=True)
class Exact:
    """The archive holds exactly this version. `.version.text` IS the prompt."""
    version: PromptVersion


@dataclass(frozen=True)
class Nearest:
    """The exact version was never archived. These are other versions of what
    looks like the same prompt file — deliberately NOT a prompt text."""
    candidates: tuple[Lineage, ...]


@dataclass(frozen=True)
class Unknown:
    reason: str


PromptMatch = Exact | Nearest | Unknown


def parse_identity(filename: str, pattern: re.Pattern) -> ImageIdentity | None:
    """Roles out of a filename, per the configured grammar. None when the name
    is not prompt-bearing at all, which is most files in the data dir."""
    m = pattern.match(filename)
    if not m:
        return None
    # .groupdict(), not .group(): the lineage group is optional in the contract
    # and .group() would raise IndexError for a grammar that omits it.
    return ImageIdentity(version=m.group("version"),
                         lineage=m.groupdict().get("lineage") or None)


def _norm(s: str) -> str:
    return re.sub(r"[^a-z0-9]", "", s.lower())


def _similarity(image_dir: str, prompt_path: str) -> float:
    """How much an image's own directory looks like the prompt's directory.
    Normalising away case and separators is what makes
    picked/huggingface/Heartsync__NSFW-Uncensored-image match the prompt dir
    Heartsync__NSFW_Uncensored_image exactly, and mrfakename_anima_v1 score
    high against anima_v1."""
    return difflib.SequenceMatcher(
        None, _norm(os.path.basename(image_dir)),
        _norm(os.path.dirname(prompt_path))).ratio()


def rank_paths(paths, near: str) -> list[str]:
    """Order candidate prompt paths by how well their directory matches the
    image's. ORDERS ONLY — every input path comes back out."""
    return sorted(paths, key=lambda p: (-_similarity(near, p), p))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `nix develop -c pytest tests/test_prompts.py -q`
Expected: PASS (9 tests).

- [ ] **Step 5: Commit**

```bash
git add src/publicator/prompts.py tests/test_prompts.py
git commit -m "feat(prompts): filename roles and the three-state match type

Nearest carries no .text on purpose: a near-miss must not be renderable
as the real prompt (ADR 0002)."
```

---

### Task 3: The archive index — digest → content, from git

**Files:**
- Modify: `src/publicator/prompts.py`
- Modify: `flake.nix` (add `pkgs.git` to the dev shell)
- Test: `tests/test_prompts.py`

**Interfaces:**
- Consumes: `PromptVersion` from Task 2.
- Produces: `PromptArchive(repo: str, pattern: re.Pattern, version_hash: str)` with
  - `.versions() -> dict[str, PromptVersion]` — digest → version, cache-refreshed
  - `.paths_for(basename: str) -> tuple[str, ...]` — every historical path with that basename
  - `.versions_at(path: str) -> tuple[PromptVersion, ...]` — newest first

**Why `pkgs.git`:** git is currently on PATH in the dev shell only because it leaks from `/etc/profiles/per-user/theta`. `writeShellApplication` pins PATH to `runtimeInputs`, so the packaged apps would have **no git at all**. Add it explicitly.

- [ ] **Step 1: Write the failing test**

Append to `tests/test_prompts.py`:

```python
import hashlib
import subprocess

from publicator.prompts import PromptArchive


def make_repo(tmp_path, name="prompts"):
    repo = tmp_path / name
    repo.mkdir()

    def run(*a):
        subprocess.run(["git", "-C", str(repo), *a], check=True, capture_output=True)

    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@example.com")
    run("config", "user.name", "t")
    return repo, run


def commit_file(repo, run, relpath, text):
    """Write, commit, and return the CONTENT digest — deliberately not the git
    blob OID, which hashes 'blob <len>\\0' + content and would never match."""
    p = repo / relpath
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)
    run("add", "--", relpath)
    run("commit", "-q", "-m", f"snapshot: {relpath}")
    return hashlib.sha1(text.encode()).hexdigest()


def archive(repo, pattern=ART, algo="sha1"):
    return PromptArchive(str(repo), pattern, algo)


def test_index_holds_a_committed_prompt_by_content_digest(tmp_path):
    repo, run = make_repo(tmp_path)
    digest = commit_file(repo, run, "anima_v1/hot.json", "a prompt\n")
    v = archive(repo).versions()[digest]
    assert v.text == "a prompt\n"
    assert v.paths == ("anima_v1/hot.json",)
    assert v.committed > 0


def test_index_keeps_superseded_versions(tmp_path):
    """The regression test for the whole feature: HEAD has moved on, and the
    old digest must still resolve to the OLD text."""
    repo, run = make_repo(tmp_path)
    old = commit_file(repo, run, "p/a", "version one\n")
    new = commit_file(repo, run, "p/a", "version two\n")
    versions = archive(repo).versions()
    assert versions[old].text == "version one\n"
    assert versions[new].text == "version two\n"


def test_index_uses_the_configured_digest_algorithm(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "hello\n")
    digest = hashlib.sha256(b"hello\n").hexdigest()
    assert archive(repo, NOLIN, "sha256").versions()[digest].text == "hello\n"


def test_paths_for_finds_a_basename_that_no_longer_exists_at_head(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/hot_mommy.json", "old\n")
    run("mv", "p/hot_mommy.json", "p/lab_hot_mommy.json")
    run("commit", "-q", "-m", "rename")
    a = archive(repo)
    assert a.paths_for("hot_mommy.json") == ("p/hot_mommy.json",)
    assert a.paths_for("lab_hot_mommy.json") == ("p/lab_hot_mommy.json",)


def test_versions_at_returns_newest_first(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "one\n")
    commit_file(repo, run, "p/a", "two\n")
    texts = [v.text for v in archive(repo).versions_at("p/a")]
    assert texts == ["two\n", "one\n"]


def test_index_refreshes_when_head_moves(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "one\n")
    a = archive(repo)
    assert len(a.versions()) == 1
    second = commit_file(repo, run, "p/a", "two\n")
    assert second in a.versions()


def test_missing_repo_yields_an_empty_index(tmp_path):
    assert archive(tmp_path / "nope").versions() == {}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_prompts.py -q`
Expected: FAIL with `ImportError: cannot import name 'PromptArchive'`.

- [ ] **Step 3: Write the implementation**

Append to `src/publicator/prompts.py` (add `import subprocess` and `import hashlib` to the imports at the top):

```python
class PromptArchive:
    """The prompt repository, indexed by content digest.

    Git history IS the archive (ADR 0001), so there is no second store to drift.
    The index is rebuilt when HEAD moves — a sound cache key precisely because
    every archive write is a commit."""

    def __init__(self, repo: str, pattern: re.Pattern, version_hash: str):
        self.repo = repo
        self.pattern = pattern
        self.version_hash = version_hash
        self._head: str | None = None
        self._versions: dict[str, PromptVersion] = {}
        self._by_basename: dict[str, list[str]] = {}
        self._by_path: dict[str, list[str]] = {}      # path -> [digest]

    # -- git ---------------------------------------------------------------

    def _git(self, *args: str) -> bytes:
        """stdout, or b"" for any git failure — a missing or broken prompt repo
        degrades the feature, it never breaks the gallery."""
        try:
            p = subprocess.run(["git", "-C", self.repo, *args],
                               capture_output=True, check=False)
        except (OSError, ValueError):
            return b""
        return p.stdout if p.returncode == 0 else b""

    def _blobs(self) -> tuple[dict[str, str], dict[str, bytes]]:
        """({blob oid: digest}, {digest: content}) for every object in the repo.

        --batch-all-objects reaches unreachable blobs too, so a prompt that was
        staged but never committed is still recovered for free. The stream is
        `<oid> <type> <size>\\n<payload>\\n`, so it must be walked by length -
        payloads are arbitrary bytes and may contain newlines."""
        out = self._git("cat-file", "--batch-all-objects", "--batch")
        by_oid: dict[str, str] = {}
        content: dict[str, bytes] = {}
        i = 0
        while i < len(out):
            j = out.find(b"\n", i)
            if j < 0:
                break
            header = out[i:j].split(b" ")
            if len(header) != 3:
                break
            oid, typ, size = header[0], header[1], int(header[2])
            body = out[j + 1:j + 1 + size]
            if typ == b"blob":
                digest = hashlib.new(self.version_hash, body).hexdigest()
                by_oid[oid.decode()] = digest
                content[digest] = body
            i = j + 1 + size + 1          # payload plus its trailing newline
        return by_oid, content

    def _history(self, by_oid: dict[str, str]):
        """(digest -> {path}, digest -> earliest commit time) from one log pass.

        Raw lines look like `:100644 100644 <old> <new> M\\tpath`.

        --no-renames is REQUIRED, not cosmetic. diff.renames defaults to true
        since git 2.9, and a detected rename emits `R100\\tp/a\\tp/b` — TWO
        tab-separated paths, which a single partition() mangles into the bogus
        path "p/a\\tp/b". Forcing delete+add gives one path per line and is
        what we want anyway: both the old and the new path stay discoverable,
        so a basename that no longer exists at HEAD still resolves."""
        text = self._git("-c", "core.quotePath=false", "log", "--all", "--raw",
                         "--no-abbrev", "--no-renames",
                         "--format=%ct").decode("utf-8", "replace")
        paths: dict[str, set[str]] = {}
        times: dict[str, int] = {}
        ct = 0
        for line in text.splitlines():
            if not line:
                continue
            if not line.startswith(":"):
                ct = int(line) if line.isdigit() else ct
                continue
            meta, _, path = line.partition("\t")
            fields = meta.split()
            if len(fields) < 4 or not path:
                continue
            digest = by_oid.get(fields[3])
            if digest is None:                        # deletion: new oid is all zeros
                continue
            paths.setdefault(digest, set()).add(path)
            times[digest] = min(times.get(digest, ct), ct)
        return paths, times

    def _refresh(self) -> None:
        head = self._git("rev-parse", "HEAD").decode().strip()
        if head == self._head and self._versions:
            return
        by_oid, content = self._blobs()
        paths, times = self._history(by_oid)
        self._versions = {
            digest: PromptVersion(
                version=digest,
                text=body.decode("utf-8", "replace"),
                paths=tuple(sorted(paths.get(digest, ()))),
                committed=times.get(digest, 0))
            for digest, body in content.items()}
        self._by_basename, self._by_path = {}, {}
        for digest, v in self._versions.items():
            for p in v.paths:
                self._by_path.setdefault(p, []).append(digest)
                base = os.path.basename(p)
                if p not in self._by_basename.setdefault(base, []):
                    self._by_basename[base].append(p)
        self._head = head

    # -- queries -----------------------------------------------------------

    def versions(self) -> dict[str, PromptVersion]:
        self._refresh()
        return self._versions

    def paths_for(self, basename: str) -> tuple[str, ...]:
        self._refresh()
        return tuple(self._by_basename.get(basename, ()))

    def versions_at(self, path: str) -> tuple[PromptVersion, ...]:
        self._refresh()
        return tuple(sorted((self._versions[d] for d in self._by_path.get(path, ())),
                            key=lambda v: (-v.committed, v.version)))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `nix develop -c pytest tests/test_prompts.py -q`
Expected: PASS (16 tests).

- [ ] **Step 5: Add git to the dev shell**

In `flake.nix`, find the dev shell line:

```nix
        packages = [ devPython pkgs.imagemagick pkgs.playwright-driver.browsers claude ];
```

Change it to:

```nix
        packages = [ devPython pkgs.git pkgs.imagemagick pkgs.playwright-driver.browsers claude ];
```

- [ ] **Step 6: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/publicator/prompts.py tests/test_prompts.py flake.nix
git commit -m "feat(prompts): index the prompt repo by content digest

git history is the archive (ADR 0001). --batch-all-objects also recovers
blobs that were staged but never committed. Cached on HEAD, which is a
sound key because every archive write is a commit."
```

---

### Task 4: Resolution — Exact / Nearest / Unknown

**Files:**
- Modify: `src/publicator/prompts.py`
- Test: `tests/test_prompts.py`

**Interfaces:**
- Consumes: `PromptArchive` (Task 3), `parse_identity` / `rank_paths` (Task 2).
- Produces:
  - `PromptArchive.resolve(identity: ImageIdentity, near: str = "") -> PromptMatch`
  - `PromptArchive.resolve_path(path: str) -> PromptMatch`
  - `from_config(data_dir: str, config: dict) -> PromptArchive | None`

- [ ] **Step 1: Write the failing test**

Append to `tests/test_prompts.py`:

```python
from publicator.prompts import from_config


def test_resolve_returns_exact_for_an_archived_version(tmp_path):
    repo, run = make_repo(tmp_path)
    digest = commit_file(repo, run, "p/hot.json", "the real prompt\n")
    got = archive(repo).resolve(ImageIdentity(version=digest, lineage="hot.json"))
    assert isinstance(got, Exact)
    assert got.version.text == "the real prompt\n"


def test_resolve_returns_exact_for_a_superseded_version(tmp_path):
    """HEAD has drifted; the image's own version must still win. This is the
    404-of-426 silently-wrong case from the audit, frozen into a test."""
    repo, run = make_repo(tmp_path)
    old = commit_file(repo, run, "p/hot.json", "version one\n")
    commit_file(repo, run, "p/hot.json", "version two\n")
    got = archive(repo).resolve(ImageIdentity(version=old, lineage="hot.json"))
    assert isinstance(got, Exact)
    assert got.version.text == "version one\n"


def test_resolve_returns_nearest_when_the_version_was_never_archived(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/hot.json", "some version\n")
    got = archive(repo).resolve(ImageIdentity(version="f" * 40, lineage="hot.json"))
    assert isinstance(got, Nearest)
    assert [c.path for c in got.candidates] == ["p/hot.json"]
    assert got.candidates[0].versions[0].text == "some version\n"


def test_nearest_keeps_every_ambiguous_lineage_ranked_not_filtered(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "Heartsync__adult/mommy_tentacles", "adult\n")
    commit_file(repo, run, "Heartsync__NSFW_Uncensored_image/mommy_tentacles", "image\n")
    got = archive(repo).resolve(
        ImageIdentity(version="f" * 40, lineage="mommy_tentacles"),
        near="picked/huggingface/Heartsync__NSFW-Uncensored-image")
    assert isinstance(got, Nearest)
    assert len(got.candidates) == 2, "ranking must never discard a candidate"
    assert got.candidates[0].path == "Heartsync__NSFW_Uncensored_image/mommy_tentacles"


def test_resolve_returns_unknown_when_the_grammar_has_no_lineage(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "x\n")
    got = archive(repo, NOLIN, "sha256").resolve(
        ImageIdentity(version="f" * 64, lineage=None))
    assert isinstance(got, Unknown)


def test_resolve_returns_unknown_for_an_unrecognised_lineage(tmp_path):
    repo, run = make_repo(tmp_path)
    commit_file(repo, run, "p/a", "x\n")
    got = archive(repo).resolve(ImageIdentity(version="f" * 40, lineage="never_seen"))
    assert isinstance(got, Unknown)


def test_resolve_path_parses_then_resolves(tmp_path):
    repo, run = make_repo(tmp_path)
    digest = commit_file(repo, run, "p/hot.json", "text\n")
    got = archive(repo).resolve_path(f"picked/hug/hot.json_{digest}_{UUID}.webp")
    assert isinstance(got, Exact)


def test_resolve_path_returns_unknown_for_a_non_prompt_filename(tmp_path):
    repo, _ = make_repo(tmp_path)
    assert isinstance(archive(repo).resolve_path("picked/tpl/clip.mp4"), Unknown)


def test_from_config_returns_none_without_a_prompts_section(tmp_path):
    assert from_config(str(tmp_path), {"prompts": None}) is None


def test_from_config_returns_none_when_the_repo_is_absent(tmp_path):
    cfg = {"prompts": {"repo": "nope", "pattern": ART, "version_hash": "sha1"}}
    assert from_config(str(tmp_path), cfg) is None


def test_from_config_builds_an_archive_for_a_present_repo(tmp_path):
    repo, run = make_repo(tmp_path, "hf")
    commit_file(repo, run, "p/a", "x\n")
    cfg = {"prompts": {"repo": "hf", "pattern": ART, "version_hash": "sha1"}}
    assert isinstance(from_config(str(tmp_path), cfg), PromptArchive)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_prompts.py -q`
Expected: FAIL with `AttributeError: 'PromptArchive' object has no attribute 'resolve'`.

- [ ] **Step 3: Write the implementation**

Add these two methods inside `PromptArchive`, after `versions_at`:

```python
    def resolve(self, identity: ImageIdentity, near: str = "") -> PromptMatch:
        """One of three answers, never a nullable string (ADR 0002). `near` is
        the image's own directory and only ever ORDERS Nearest candidates."""
        exact = self.versions().get(identity.version)
        if exact is not None:
            return Exact(version=exact)
        if not identity.lineage:
            return Unknown(reason="prompt not archived; "
                                  "the configured grammar carries no lineage")
        candidates = tuple(
            Lineage(path=p, versions=vs)
            for p in rank_paths(self.paths_for(identity.lineage), near)
            if (vs := self.versions_at(p)))
        if not candidates:
            return Unknown(
                reason=f"prompt not archived; no known versions of "
                       f"{identity.lineage!r}")
        return Nearest(candidates=candidates)

    def resolve_path(self, path: str) -> PromptMatch:
        """Resolve an art file by its path. Non-prompt-bearing names — most of
        the data dir — come back Unknown, which renders as nothing."""
        identity = parse_identity(os.path.basename(path), self.pattern)
        if identity is None:
            return Unknown(reason="filename does not match the configured grammar")
        return self.resolve(identity, near=os.path.dirname(path))
```

And this module-level function at the end of the file:

```python
def from_config(data_dir: str, config: dict) -> PromptArchive | None:
    """The archive for this data dir, or None when the feature is off — no
    [prompts] section, or a configured repo that is not on disk. Callers treat
    None as 'every lookup is Unknown'."""
    cfg = config.get("prompts")
    if not cfg:
        return None
    repo = os.path.join(data_dir, cfg["repo"])
    if not os.path.isdir(repo):
        return None
    return PromptArchive(repo, cfg["pattern"], cfg["version_hash"])
```

- [ ] **Step 4: Run test to verify it passes**

Run: `nix develop -c pytest tests/test_prompts.py -q`
Expected: PASS (27 tests).

- [ ] **Step 5: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/publicator/prompts.py tests/test_prompts.py
git commit -m "feat(prompts): resolve an image to Exact, Nearest or Unknown

A superseded version still resolves to its own text - the failure that
made the naive basename->HEAD lookup wrong on 404 of 426 pairs."
```

---

### Task 5: The renderer

**Files:**
- Create: `src/publicator/webui/prompt_view.py`
- Test: `tests/test_prompt_view.py`

**Interfaces:**
- Consumes: `Exact`, `Nearest`, `Unknown`, `rank_paths` from Tasks 2–4.
- Produces: `render_prompt(match: PromptMatch, near: str = "") -> str` — an HTML fragment, `""` for `Unknown`.

- [ ] **Step 1: Write the failing test**

Create `tests/test_prompt_view.py`:

```python
from publicator.prompts import Exact, Lineage, Nearest, PromptVersion, Unknown
from publicator.webui.prompt_view import render_prompt


def version(text="a prompt", paths=("p/a",), committed=1_700_000_000, digest="ab" * 20):
    return PromptVersion(version=digest, text=text, paths=paths, committed=committed)


def test_exact_renders_the_prompt_text():
    out = render_prompt(Exact(version=version(text="the real prompt")))
    assert "the real prompt" in out
    assert "NOT ARCHIVED" not in out


def test_exact_shows_every_path_not_just_the_first():
    out = render_prompt(Exact(version=version(paths=("p/a", "q/a"))))
    assert "p/a" in out and "q/a" in out


def test_exact_escapes_html_in_the_prompt():
    out = render_prompt(Exact(version=version(text="<script>alert(1)</script>")))
    assert "<script>" not in out
    assert "&lt;script&gt;" in out


def test_nearest_is_labelled_as_not_archived():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="p/a", versions=(version(text="an older one"),)),)))
    assert "NOT ARCHIVED" in out
    assert "an older one" in out, "near-misses are still shown, just labelled"
    # the text only ever appears inside the warning block
    assert out.index("NOT ARCHIVED") < out.index("an older one")


def test_nearest_lists_every_candidate_lineage():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="dir_one/a", versions=(version(),)),
        Lineage(path="dir_two/a", versions=(version(), version(digest="cd" * 20))))))
    assert "dir_one/a" in out and "dir_two/a" in out
    assert "1 known version" in out and "2 known versions" in out


def test_unknown_renders_nothing():
    assert render_prompt(Unknown(reason="whatever")) == ""


def test_undated_version_does_not_crash():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="p/a", versions=(version(committed=0),)),)))
    assert "unknown date" in out
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_prompt_view.py -q`
Expected: FAIL with `ModuleNotFoundError: No module named 'publicator.webui.prompt_view'`.

- [ ] **Step 3: Write the implementation**

Create `src/publicator/webui/prompt_view.py`:

```python
"""The prompt block on a gallery card: an image's generation prompt, or an
explicitly labelled near-miss.

Pure, like calendar_view: it is handed a resolved match and returns HTML.

The visual split is the point. An Exact reads as the prompt; a Nearest reads as
a warning that happens to contain other people's prompts. The type makes the
mistake impossible in code (Nearest has no .text); this module makes it hard to
make by eye."""

import html
from datetime import datetime

from publicator.prompts import Exact, Nearest, PromptMatch, rank_paths


def _stamp(committed: int) -> str:
    """Blobs recovered from unreferenced objects have no commit to date them."""
    if not committed:
        return "unknown date"
    return datetime.fromtimestamp(committed).strftime("%Y-%m-%d")


def _body(text: str) -> str:
    """Raw <pre>, for plain-text and JSON prompts alike — the JSON ones are
    already pretty-printed on disk, so parsing them buys nothing."""
    return f"<pre>{html.escape(text)}</pre>"


def render_prompt(match: PromptMatch, near: str = "") -> str:
    if isinstance(match, Exact):
        v = match.version
        paths = " · ".join(html.escape(p) for p in rank_paths(v.paths, near))
        return (f'<details class="prompt exact"><summary>prompt · '
                f'{paths or "path unknown"} · <code>{html.escape(v.version[:7])}</code>'
                f'</summary>{_body(v.text)}</details>')
    if isinstance(match, Nearest):
        items = []
        for lin in match.candidates:
            n = len(lin.versions)
            versions = "".join(
                f'<details class="pv"><summary>{_stamp(v.committed)} · '
                f'<code>{html.escape(v.version[:7])}</code></summary>'
                f"{_body(v.text)}</details>" for v in lin.versions)
            items.append(
                f'<li><span class="lin">{html.escape(lin.path)}</span> '
                f'<span class="n">{n} known version{"" if n == 1 else "s"}</span>'
                f"{versions}</li>")
        return ('<details class="prompt nearest">'
                "<summary>prompt NOT ARCHIVED ⚠</summary>"
                '<p class="warn">This exact prompt was never committed and is '
                "unrecoverable. Other versions of the same prompt file — "
                "<strong>NOT</strong> what made this image:</p>"
                f'<ul>{"".join(items)}</ul></details>')
    return ""
```

- [ ] **Step 4: Run test to verify it passes**

Run: `nix develop -c pytest tests/test_prompt_view.py -q`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add src/publicator/webui/prompt_view.py tests/test_prompt_view.py
git commit -m "feat(webui): render prompts, with near-misses visibly labelled"
```

---

### Task 6: Wire the prompt block into the gallery

**Files:**
- Modify: `src/publicator/webui/page.py` (`render_page` signature; both card loops; CSS in `PAGE_TEMPLATE`)
- Modify: `src/publicator/webui/server.py` (`GalleryHandler.archive`, `_build_page`, `serve`)
- Modify: `flake.nix` (`pkgs.git` in `publish-next` runtime inputs)
- Test: `tests/test_page.py`

**Interfaces:**
- Consumes: `from_config`, `PromptArchive.resolve_path` (Task 4); `render_prompt` (Task 5).
- Produces: `render_page(..., prompt_html: dict[str, str] | None = None)` — art path → HTML fragment. Keyword-only with a default, so existing callers and tests keep working.

- [ ] **Step 1: Write the failing test**

Append to `tests/test_page.py`:

```python
def test_render_page_injects_the_prompt_block_into_a_candidate_card():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={"picked/a.webp": "aa.webp"}, candidates=["picked/a.webp"],
        pending=[], existing_ts=[], timeline=[], schedules=[],
        config={}, ai_model="m", openrouter_model="",
        prompt_html={"picked/a.webp": '<details class="prompt exact">X</details>'})
    assert '<details class="prompt exact">X</details>' in page


def test_render_page_without_prompt_html_is_unchanged():
    from publicator.webui.page import render_page
    kwargs = dict(
        thumb_map={"picked/a.webp": "aa.webp"}, candidates=["picked/a.webp"],
        pending=[], existing_ts=[], timeline=[], schedules=[],
        config={}, ai_model="m", openrouter_model="")
    assert render_page(**kwargs) == render_page(**kwargs, prompt_html={})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_page.py -q`
Expected: FAIL with `TypeError: render_page() got an unexpected keyword argument 'prompt_html'`.

- [ ] **Step 3: Modify `render_page`**

In `src/publicator/webui/page.py`, change the signature:

```python
def render_page(*, thumb_map, candidates, pending, existing_ts, timeline,
                schedules, config, ai_model, openrouter_model,
                prompt_html=None) -> str:
```

Add to the docstring: ``prompt_html`` is {art path: prompt HTML block} — pre-rendered by the server so this module stays unaware of prompts.

Immediately after the `tg = tier_gallery_fields(config)` line, add:

```python
    prompt_html = prompt_html or {}
```

In the **pending** card f-string, insert the prompt block between the `</div>` that closes `actions` and the `{card_form_html(...)}` line:

```python
  </div>
{prompt_html.get(e["path"], "")}
{card_form_html(cid, js_pathp, tg, "Save changes", preset_opts)}
</div>""")
```

In the **candidate** card f-string, the same, keyed on `orig_path`:

```python
  </div>
{prompt_html.get(orig_path, "")}
{card_form_html(cid, js_path, tg, "Save to queue", preset_opts)}
</div>""")
```

- [ ] **Step 4: Add the CSS**

In `PAGE_TEMPLATE`, inside the existing `<style>` block, append:

```css
.prompt { margin: .4rem 0; font-size: .85rem; text-align: left; }
.prompt > summary { cursor: pointer; padding: .2rem .4rem; border-radius: 3px; }
.prompt.exact > summary { background: #eef4ee; color: #2c4a2c; }
.prompt.nearest > summary { background: #fff3cd; color: #7a5b00; font-weight: 600; }
.prompt.nearest { border-left: 3px solid #e0a800; padding-left: .4rem; }
.prompt .warn { color: #7a5b00; margin: .3rem 0; }
.prompt pre { white-space: pre-wrap; word-break: break-word; background: #f7f7f7;
              padding: .4rem; border-radius: 3px; max-height: 20rem; overflow: auto; }
.prompt ul { list-style: none; padding-left: 0; }
.prompt .lin { font-family: monospace; }
.prompt .n { color: #666; font-size: .8rem; }
```

- [ ] **Step 5: Wire the server**

In `src/publicator/webui/server.py`, add imports:

```python
from publicator.prompts import from_config as prompt_archive_from_config
from publicator.webui.prompt_view import render_prompt
```

Add a class attribute to `GalleryHandler`, after `schedules`:

```python
    archive = None                       # PromptArchive | None; None = feature off
```

Add this method just above `_build_page`:

```python
    def _prompt_html(self) -> dict[str, str]:
        """{art path: prompt block} for everything the page can show. Built here
        rather than in page.py so the page stays unaware of prompts."""
        if self.archive is None:
            return {}
        return {p: block for p in self.thumb_map
                if (block := render_prompt(self.archive.resolve_path(p),
                                           near=os.path.dirname(p)))}
```

Pass it in `_build_page`, adding one argument to the `render_page(...)` call:

```python
            openrouter_model=self.openrouter_model,
            prompt_html=self._prompt_html())
```

In `serve()`, after the `GalleryHandler.schedules = ...` line:

```python
    GalleryHandler.archive = prompt_archive_from_config(data_dir, config)
```

- [ ] **Step 6: Add git to the publish-next app**

In `flake.nix`, change:

```nix
        publish-next = app "publish-next" "publish_next" [ pkgs.imagemagick browsers claude ] true;
```

to:

```nix
        publish-next = app "publish-next" "publish_next" [ pkgs.git pkgs.imagemagick browsers claude ] true;
```

`writeShellApplication` pins PATH to `runtimeInputs`, so without this the packaged app has no `git` and every lookup silently returns `Unknown`.

- [ ] **Step 7: Run the tests**

Run: `nix develop -c pytest tests/test_page.py tests/test_server.py -q`
Expected: PASS.

- [ ] **Step 8: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/publicator/webui/page.py src/publicator/webui/server.py tests/test_page.py flake.nix
git commit -m "feat(webui): show each card's generation prompt

The server resolves and renders; page.py only splices, so it stays
unaware of prompts. pkgs.git added to publish-next: writeShellApplication
pins PATH to runtimeInputs, so git would otherwise be absent."
```

---

### Task 7: Search by prompt text

**Files:**
- Modify: `src/publicator/webui/server.py`
- Modify: `src/publicator/webui/page.py` (search box + result banner)
- Modify: `src/publicator/apps/publish_next.py` (pass the publicable dirs through)
- Test: `tests/test_server.py`

**Interfaces:**
- Consumes: `PromptArchive.resolve_path` (Task 4).
- Produces:
  - `GalleryHandler.publicable_dirs: list[str]`
  - `GalleryHandler.search(needle: str, lineage: str) -> tuple[list[str], list[str], int]` — `(exact hits, lineage-hint hits, skipped count)`. Task 8 uses the middle element; this task leaves it empty.
  - `render_page(..., query: str = "", skipped: int = 0)`

- [ ] **Step 1: Write the failing test**

Append to `tests/test_server.py`:

```python
def test_search_matches_exact_prompt_text_only(tmp_path, monkeypatch):
    """Matching a Nearest's text would return images whose prompt merely
    resembles the query - the conflation the whole feature exists to prevent."""
    import subprocess
    from publicator.prompts import PromptArchive
    from publicator.webui.server import GalleryHandler
    import re, hashlib

    repo = tmp_path / "hf"; repo.mkdir()
    run = lambda *a: subprocess.run(["git", "-C", str(repo), *a], check=True,
                                    capture_output=True)
    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("tentacles everywhere\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")
    digest = hashlib.sha1(b"tentacles everywhere\n").hexdigest()

    pattern = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                         r"[0-9a-f-]{36}\.[^.]+$")
    uuid = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"
    picked = tmp_path / "picked"; picked.mkdir()
    hit = picked / f"a_{digest}_{uuid}.webp"
    miss = picked / f"a_{'f' * 40}_{uuid}.webp"      # never archived -> Nearest
    hit.write_bytes(b"1"); miss.write_bytes(b"2")

    # monkeypatch, not plain assignment: these are CLASS attributes and would
    # otherwise leak into every other test in this file.
    monkeypatch.setattr(GalleryHandler, "archive",
                        PromptArchive(str(repo), pattern, "sha1"))
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [str(picked)])
    monkeypatch.setattr(GalleryHandler, "json_path",
                        str(tmp_path / "publications.json"))

    exact, _maybe, skipped = GalleryHandler.search(GalleryHandler, "tentacles", "")
    assert exact == [str(hit)]
    assert skipped == 1, "the unarchived one is reported, not silently dropped"


def test_search_is_inert_when_the_feature_is_off(monkeypatch):
    from publicator.webui.server import GalleryHandler
    monkeypatch.setattr(GalleryHandler, "archive", None)
    monkeypatch.setattr(GalleryHandler, "publicable_dirs", [])
    assert GalleryHandler.search(GalleryHandler, "nothing", "") == ([], [], 0)
```

And append to `tests/test_page.py`, so a forgotten `__PROMPTSEARCH__` placeholder
cannot pass silently — `str.replace` on a missing placeholder is a no-op:

```python
def test_render_page_renders_the_search_box_and_skip_banner():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={}, candidates=[], pending=[], existing_ts=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="",
        query="tentacles", skipped=3)
    assert "__PROMPTSEARCH__" not in page, "placeholder left in PAGE_TEMPLATE"
    assert 'name="prompt"' in page and 'value="tentacles"' in page
    assert "3 candidates not searched" in page
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_server.py -q`
Expected: FAIL with `AttributeError: type object 'GalleryHandler' has no attribute 'search'`.

- [ ] **Step 3: Implement the search**

In `src/publicator/webui/server.py`, extend the imports from `publicator.images`:

```python
from publicator.images import (
    THUMB_CACHE,
    collect_images,
    ensure_thumb,
    guess_mime,
    index_by_basename,
    load_publicated_hashes,
    compute_sha512,
    thumb_name,
)
```

and from `publicator.prompts`:

```python
from publicator.prompts import Exact, Nearest
from publicator.prompts import from_config as prompt_archive_from_config
```

Add a module constant near `log = logging.getLogger(...)`:

```python
# ponytail: a filtered view walks every publicable image; this caps how many
# survivors get sha512-hashed for the already-published check. Raise it if a
# lineage ever legitimately has more art than this.
SEARCH_LIMIT = 200
```

Add class attributes to `GalleryHandler`, after `archive`:

```python
    publicable_dirs: list[str] = []      # walked in full when a filter is active
```

Add this method after `_prompt_html`:

```python
    def search(self, needle: str, lineage: str) -> tuple[list[str], list[str], int]:
        """(exact hits, lineage-hint hits, count skipped as unarchived).

        `find_candidates` shuffles and caps, so filtering its sample would
        return near-nothing — a filtered view walks the publicable dirs itself.
        Order matters: resolve and filter first (regex + dict lookups, cheap),
        cap, and only then sha512-hash the survivors for the already-published
        check, which is the expensive step."""
        if self.archive is None:
            return [], [], 0
        exact, maybe, skipped = [], [], 0
        for d in self.publicable_dirs:
            for path in collect_images(d):
                match = self.archive.resolve_path(path)
                if isinstance(match, Nearest):
                    skipped += 1
                    if lineage and any(c.path == lineage for c in match.candidates):
                        maybe.append(path)
                    continue
                if not isinstance(match, Exact):
                    continue
                if needle and needle.lower() not in match.version.text.lower():
                    continue
                if lineage and lineage not in match.version.paths:
                    continue
                exact.append(path)
        published = load_publicated_hashes(self.json_path)

        def unpublished(paths):
            out = []
            for p in paths[:SEARCH_LIMIT]:
                try:
                    if compute_sha512(p) not in published:
                        out.append(p)
                except OSError as e:
                    log.debug("hashing %s: %s", p, e)
            return out

        return unpublished(exact), unpublished(maybe), skipped

    def _register_thumbs(self, paths: list[str]) -> None:
        """Extend the /thumbs allow-list with search results. The allow-list is
        'names this page rendered', so it must grow when a filtered page
        renders art the initial sample never included."""
        for p in paths:
            if p not in self.__class__.thumb_map:
                name = thumb_name(p)
                self.__class__.thumb_map[p] = name
                self.__class__.thumb_src[name] = p
```

- [ ] **Step 4: Route the query string**

Replace the first branch of `do_GET`. The current code is:

```python
    def do_GET(self):
        if self.path in ("/", "/index.html"):
            page = self._build_page().encode("utf-8")
```

Replace with:

```python
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path in ("/", "/index.html"):
            q = urllib.parse.parse_qs(parsed.query)
            page = self._build_page(
                needle=q.get("prompt", [""])[0],
                lineage=q.get("lineage", [""])[0]).encode("utf-8")
```

The remaining `elif` branches keep using `self.path` unchanged — they carry their own query strings and are unaffected.

Then change `_build_page` to:

```python
    def _build_page(self, needle: str = "", lineage: str = "") -> str:
        candidates, skipped = self.candidate_paths, 0
        if needle or lineage:
            candidates, _maybe, skipped = self.search(needle, lineage)
            self._register_thumbs(candidates)
        return render_page(
            thumb_map=self.thumb_map, candidates=candidates,
            pending=self.pending, existing_ts=self.existing_ts,
            timeline=self.timeline, schedules=self.schedules,
            config=self.config, ai_model=self.ai_model,
            openrouter_model=self.openrouter_model,
            prompt_html=self._prompt_html(candidates),
            query=needle, skipped=skipped)
```

`_prompt_html` now takes the paths to render for, so a filtered page does not resolve the whole default sample. Change its signature and body:

```python
    def _prompt_html(self, paths) -> dict[str, str]:
        """{art path: prompt block} for the cards this page will show."""
        if self.archive is None:
            return {}
        wanted = list(paths) + [e["path"] for e in self.pending]
        return {p: block for p in wanted
                if (block := render_prompt(self.archive.resolve_path(p),
                                           near=os.path.dirname(p)))}
```

- [ ] **Step 5: Add the search box and banner to the page**

In `src/publicator/webui/page.py`, extend the signature:

```python
def render_page(*, thumb_map, candidates, pending, existing_ts, timeline,
                schedules, config, ai_model, openrouter_model,
                prompt_html=None, query="", skipped=0) -> str:
```

Before the final `return page`, add:

```python
    # Server-side search: a plain GET form, no JS — the same reason scheduling
    # is server-side. Unarchived prompts are excluded from matching and the
    # count is stated, so the 86% gap stays visible instead of quietly
    # shrinking the result set.
    banner = (f'<p class="skipped">{skipped} candidate'
              f'{"" if skipped == 1 else "s"} not searched — prompt not archived.</p>'
              if skipped else "")
    search = (f'<form class="promptsearch" method="get" action="/">'
              f'<input type="search" name="prompt" placeholder="search prompt text"'
              f' value="{html.escape(query, quote=True)}">'
              f'<button type="submit">Search</button>'
              f'<a href="/">clear</a></form>{banner}')
    page = page.replace("__PROMPTSEARCH__", search)
```

Add the placeholder to `PAGE_TEMPLATE` immediately before `__CARDS__` (inside the gallery tab's container), and this CSS to the `<style>` block:

```css
.promptsearch { margin: .5rem 0; display: flex; gap: .4rem; align-items: center; }
.promptsearch input { flex: 1; max-width: 30rem; padding: .3rem; }
.skipped { color: #7a5b00; font-size: .85rem; margin: .2rem 0 .6rem; }
```

- [ ] **Step 6: Pass the publicable dirs through**

In `src/publicator/apps/publish_next.py`, change the `serve` call to pass the dirs it already computed:

```python
    result = serve(str(data_dir), candidates, pending, args, config, publicable_dirs)
```

In `src/publicator/webui/server.py`, change `serve`'s signature and set the attribute:

```python
def serve(data_dir: str, candidate_paths: list[str], pending: list[dict],
          args, config: dict, publicable_dirs: list[str] | None = None) -> dict | None:
```

and after `GalleryHandler.archive = ...`:

```python
    GalleryHandler.publicable_dirs = publicable_dirs or []
```

- [ ] **Step 7: Run the tests**

Run: `nix develop -c pytest tests/test_server.py tests/test_page.py -q`
Expected: PASS.

- [ ] **Step 8: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/publicator/webui/server.py src/publicator/webui/page.py \
        src/publicator/apps/publish_next.py tests/test_server.py
git commit -m "feat(webui): search the gallery by prompt text

Matches archived prompts only - matching a near-miss would return images
whose prompt merely resembles the query. The excluded count is stated
rather than silently shrinking the result set."
```

---

### Task 8: Group by prompt lineage

**Files:**
- Modify: `src/publicator/webui/server.py`
- Modify: `src/publicator/webui/page.py`
- Modify: `src/publicator/webui/prompt_view.py` (make lineages clickable)
- Test: `tests/test_server.py`, `tests/test_prompt_view.py`

**Interfaces:**
- Consumes: `GalleryHandler.search` (Task 7, whose middle return value this task finally uses).
- Produces: `render_page(..., maybe_candidates: list[str] | None = None)`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/test_prompt_view.py`:

```python
def test_nearest_lineages_link_to_their_group():
    out = render_prompt(Nearest(candidates=(
        Lineage(path="anima_v1/hot.json", versions=(version(),)),)))
    assert 'href="/?lineage=anima_v1%2Fhot.json"' in out


def test_exact_paths_link_to_their_group():
    out = render_prompt(Exact(version=version(paths=("anima_v1/hot.json",))))
    assert 'href="/?lineage=anima_v1%2Fhot.json"' in out
```

Append to `tests/test_page.py`:

```python
def test_render_page_separates_exact_and_hinted_lineage_results():
    from publicator.webui.page import render_page
    page = render_page(
        thumb_map={"a.webp": "a", "b.webp": "b"}, candidates=["a.webp"],
        maybe_candidates=["b.webp"], pending=[], existing_ts=[], timeline=[],
        schedules=[], config={}, ai_model="m", openrouter_model="")
    assert "possibly from this prompt" in page
    # the two groups must not be merged into one grid
    assert page.index("a.webp") < page.index("possibly from this prompt")
    assert page.index("possibly from this prompt") < page.index("b.webp")
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop -c pytest tests/test_prompt_view.py tests/test_page.py -q`
Expected: FAIL — no `href="/?lineage=..."`, and `TypeError` for `maybe_candidates`.

- [ ] **Step 3: Make lineages clickable**

In `src/publicator/webui/prompt_view.py`, add `import urllib.parse` and this helper above `render_prompt`:

```python
def _lineage_link(path: str) -> str:
    """A lineage label that jumps to every image from that prompt file."""
    href = "/?lineage=" + urllib.parse.quote(path, safe="")
    return (f'<a class="lin" href="{html.escape(href, quote=True)}">'
            f"{html.escape(path)}</a>")
```

In the `Exact` branch, replace the `paths = " · ".join(...)` line with:

```python
        paths = " · ".join(_lineage_link(p) for p in rank_paths(v.paths, near))
```

In the `Nearest` branch, replace `f'<li><span class="lin">{html.escape(lin.path)}</span> '` with:

```python
                f"<li>{_lineage_link(lin.path)} "
```

- [ ] **Step 4: Render the two groups**

In `src/publicator/webui/page.py`, extend the signature with `maybe_candidates=None`:

```python
def render_page(*, thumb_map, candidates, pending, existing_ts, timeline,
                schedules, config, ai_model, openrouter_model,
                prompt_html=None, query="", skipped=0,
                maybe_candidates=None) -> str:
```

Refactor the candidate-card loop into a local helper so both groups share it. Replace the whole `for idx, orig_path in enumerate(candidates):` block with:

```python
    def candidate_card(idx, orig_path):
        filename = os.path.basename(orig_path)
        safe_name = html.escape(filename, quote=True)
        safe_rel = html.escape(thumb_map.get(orig_path, ""), quote=True)
        js_path = html.escape(json.dumps(orig_path), quote=True)
        view_href = html.escape("/original?path=" + urllib.parse.quote(orig_path),
                                quote=True)
        cid = f"card_{idx}"
        return f"""<div class="card" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_name}">
  <div class="name">{safe_name}</div>
  <div class="actions">
    <a class="btn-view" href="{view_href}" target="_blank" rel="noopener">View</a>
    <button class="btn-add" onclick="openForm('{cid}')">Add</button>
    <button class="btn-del" onclick="delCard('{cid}', {js_path})">Delete</button>
  </div>
{prompt_html.get(orig_path, "")}
{card_form_html(cid, js_path, tg, "Save to queue", preset_opts)}
</div>"""

    for idx, orig_path in enumerate(candidates):
        cards.append(candidate_card(idx, orig_path))
    # Two lists, never merged: images whose digest IS a version of this lineage,
    # and images that only share its basename because their own prompt was never
    # archived. Merging them would re-conflate exactly what the type separates.
    if maybe_candidates:
        cards.append('<div class="groupsplit">possibly from this prompt — '
                     "their own prompt was never archived, so this is a "
                     "filename hint only</div>")
        for idx, orig_path in enumerate(maybe_candidates, start=len(candidates)):
            cards.append(candidate_card(idx, orig_path))
```

Add this CSS to the `<style>` block:

```css
.groupsplit { grid-column: 1 / -1; margin: 1rem 0 .3rem; padding: .4rem;
              background: #fff3cd; color: #7a5b00; border-left: 3px solid #e0a800;
              font-size: .9rem; }
```

- [ ] **Step 5: Wire it in the server**

In `src/publicator/webui/server.py`, update `_build_page`:

```python
    def _build_page(self, needle: str = "", lineage: str = "") -> str:
        candidates, maybe, skipped = self.candidate_paths, [], 0
        if needle or lineage:
            candidates, maybe, skipped = self.search(needle, lineage)
            self._register_thumbs(candidates + maybe)
        return render_page(
            thumb_map=self.thumb_map, candidates=candidates,
            maybe_candidates=maybe, pending=self.pending,
            existing_ts=self.existing_ts, timeline=self.timeline,
            schedules=self.schedules, config=self.config,
            ai_model=self.ai_model, openrouter_model=self.openrouter_model,
            prompt_html=self._prompt_html(candidates + maybe),
            query=needle, skipped=skipped)
```

- [ ] **Step 6: Run the tests**

Run: `nix develop -c pytest tests/test_prompt_view.py tests/test_page.py tests/test_server.py -q`
Expected: PASS.

- [ ] **Step 7: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/publicator/webui/ tests/test_page.py tests/test_prompt_view.py tests/test_server.py
git commit -m "feat(webui): group the gallery by prompt lineage

Exact members and basename-hint members render as two separate lists -
merging them would re-conflate what the match type separates."
```

---

### Task 9: The `prompt-audit` app

**Files:**
- Create: `src/publicator/apps/prompt_audit.py`
- Modify: `flake.nix`
- Test: `tests/test_prompt_audit.py`

**Interfaces:**
- Consumes: `from_config`, `resolve_path` (Task 4); `collect_images` from `images.py`.
- Produces: `audit(data_dir: str, config: dict) -> dict` with keys `parsed`, `files`, `exact`, `exact_versions`, `nearest`, `nearest_versions`, `unknown`.

- [ ] **Step 1: Write the failing test**

Create `tests/test_prompt_audit.py`:

```python
import hashlib
import re
import subprocess

from publicator.apps.prompt_audit import audit, format_report

ART = re.compile(r"^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_"
                 r"[0-9a-f-]{36}\.[^.]+$")
UUID = "bb6ba911-8d16-4d5a-82c4-a46b844863ed"


def setup_dir(tmp_path):
    repo = tmp_path / "hf"; repo.mkdir()

    def run(*a):
        subprocess.run(["git", "-C", str(repo), *a], check=True, capture_output=True)

    run("init", "-q", "-b", "main")
    run("config", "user.email", "t@e"); run("config", "user.name", "t")
    (repo / "a").write_text("prompt text\n")
    run("add", "--", "a"); run("commit", "-q", "-m", "s")
    digest = hashlib.sha1(b"prompt text\n").hexdigest()

    picked = tmp_path / "picked"; picked.mkdir()
    (picked / f"a_{digest}_{UUID}.webp").write_bytes(b"1")     # exact
    (picked / f"a_{'f' * 40}_{UUID}.webp").write_bytes(b"2")   # nearest
    (picked / f"zz_{'e' * 40}_{UUID}.webp").write_bytes(b"3")  # unknown lineage
    (picked / "holiday.jpg").write_bytes(b"4")                 # not prompt-bearing
    cfg = {"publicable": ["picked"],
           "prompts": {"repo": "hf", "pattern": ART, "version_hash": "sha1"}}
    return cfg


def test_audit_counts_each_resolution_state(tmp_path):
    cfg = setup_dir(tmp_path)
    got = audit(str(tmp_path), cfg)
    assert got["files"] == 4
    assert got["parsed"] == 3
    assert got["exact"] == 1 and got["exact_versions"] == 1
    assert got["nearest"] == 1 and got["nearest_versions"] == 1
    assert got["unknown"] == 1


def test_report_leads_with_the_parse_rate(tmp_path):
    """`parsed` dropping to 0 is how grammar drift between identify_image.sh
    and publicator.toml is caught (ADR 0003)."""
    lines = format_report(audit(str(tmp_path), setup_dir(tmp_path))).splitlines()
    assert lines[0].startswith("parsed")
    assert "3 of 4" in lines[0]


def test_audit_without_a_prompts_section_parses_nothing(tmp_path):
    (tmp_path / "picked").mkdir()
    got = audit(str(tmp_path), {"publicable": ["picked"], "prompts": None})
    assert got == {"files": 0, "parsed": 0, "exact": 0, "exact_versions": 0,
                   "nearest": 0, "nearest_versions": 0, "unknown": 0}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c pytest tests/test_prompt_audit.py -q`
Expected: FAIL with `ModuleNotFoundError: No module named 'publicator.apps.prompt_audit'`.

- [ ] **Step 3: Write the implementation**

Create `src/publicator/apps/prompt_audit.py`:

```python
#!/usr/bin/env python3
"""CLI: how much of the data dir's art can be traced back to its prompt.

Two of these numbers earn their place (ADRs 0003, 0004):

* `parsed` drops to 0 if the filename grammar in publicator.toml drifts from
  the one identify_image.sh actually produces — they live in two repos and
  nothing can prevent that, so it is made loud instead.
* `nearest` is the health metric for archiving. Flat is healthy. Rising means
  archive writes are silently failing at generation time. It can also fall,
  when a previously-failed prompt is committed later and its digest resolves
  retroactively.
"""
import argparse
import os
import sys
from pathlib import Path

from publicator.config import load_config
from publicator.images import collect_images
from publicator.prompts import Exact, Nearest, from_config


def audit(data_dir: str, config: dict) -> dict:
    archive = from_config(data_dir, config)
    counts = {"files": 0, "parsed": 0, "exact": 0, "nearest": 0, "unknown": 0}
    exact_versions, nearest_versions = set(), set()
    for d in config.get("publicable", []):
        for path in collect_images(os.path.join(data_dir, d)):
            counts["files"] += 1
            if archive is None:
                continue
            identity = archive.parse(path)
            if identity is None:
                continue
            counts["parsed"] += 1
            match = archive.resolve(identity, near=os.path.dirname(path))
            if isinstance(match, Exact):
                counts["exact"] += 1
                exact_versions.add(identity.version)
            elif isinstance(match, Nearest):
                counts["nearest"] += 1
                nearest_versions.add(identity.version)
            else:
                counts["unknown"] += 1
    counts["exact_versions"] = len(exact_versions)
    counts["nearest_versions"] = len(nearest_versions)
    return counts


def format_report(c: dict) -> str:
    return (
        f"parsed   : {c['parsed']} of {c['files']} files\n"
        f"exact    : {c['exact']} images / {c['exact_versions']} versions\n"
        f"nearest  : {c['nearest']} images / {c['nearest_versions']} versions\n"
        f"unknown  : {c['unknown']}")


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Report how much art resolves to its generation prompt.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir; default: CWD")
    args = ap.parse_args()
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    print(format_report(audit(str(data_dir), load_config(data_dir))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

The audit needs the identity as well as the match (to count *distinct versions*), so add this small method to `PromptArchive` in `src/publicator/prompts.py`, right above `resolve_path`:

```python
    def parse(self, path: str) -> ImageIdentity | None:
        """The roles this art file's name claims, or None if it is not
        prompt-bearing. `resolve_path` is parse + resolve; the audit needs the
        two halves separately, to count distinct versions."""
        return parse_identity(os.path.basename(path), self.pattern)
```

and simplify `resolve_path` to use it:

```python
    def resolve_path(self, path: str) -> PromptMatch:
        """Resolve an art file by its path. Non-prompt-bearing names — most of
        the data dir — come back Unknown, which renders as nothing."""
        identity = self.parse(path)
        if identity is None:
            return Unknown(reason="filename does not match the configured grammar")
        return self.resolve(identity, near=os.path.dirname(path))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `nix develop -c pytest tests/test_prompt_audit.py -q`
Expected: PASS (3 tests).

- [ ] **Step 5: Add the flake app**

In `flake.nix`, after the `echo-first = ...` line, add:

```nix
        # Prompt-mapping coverage. `parsed` drops to 0 on grammar drift;
        # `nearest` rising means archive writes are failing (ADRs 0003, 0004).
        prompt-audit = app "prompt-audit" "prompt_audit" [ pkgs.git ] false;
```

- [ ] **Step 6: Run the full suite**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 7: Verify against the real data**

Run: `cd ../Art && nix run <this>#prompt-audit`
Expected: `parsed` non-zero. Compare to the 2026-08-21 baseline in Global Constraints. Note: this requires Task 11's `[prompts]` section to exist in `../Art/publicator.toml` — if that is not done yet, all four numbers are zero, which is correct behaviour, not a failure.

- [ ] **Step 8: Commit**

```bash
git add src/publicator/apps/prompt_audit.py src/publicator/prompts.py \
        tests/test_prompt_audit.py flake.nix
git commit -m "feat(apps): prompt-audit reports prompt-mapping coverage

parsed catches grammar drift between the two repos; nearest is the health
metric for whether archive writes are still landing."
```

---

### Task 10: Write side — archive at mint time (repo `huggingface_prompts`)

**Files (in the `huggingface_prompts` submodule, a SEPARATE git repo — `git@github.com:M4jor-Tom/huggingface_prompts.git`):**
- Modify: `identify_image.sh`
- Create: `test_identify_image.sh`
- Create: `README.md` (pointer back to the ADRs, which live in the other repo)

**Interfaces:**
- Consumes: nothing from earlier tasks — this is the other repo.
- Produces: the archive invariant the read side depends on. `identify_image.sh` still prints exactly one line on stdout and still exits 0.

**Working directory:** `../Art/huggingface_prompts` relative to the publicator repo. Commit here separately, then bump the submodule pointer in `../Art`.

- [ ] **Step 1: Write the failing test**

Create `../Art/huggingface_prompts/test_identify_image.sh`:

```sh
#!/bin/sh
# The property that protects the IMAGE, not the link (ADR 0004): whatever goes
# wrong with git, identify_image.sh must exit 0 and print exactly one line.
# rename_image.sh captures that line as a filename, so a warning leaking to
# stdout would be spliced into the image's name - a worse bug than a dangling
# prompt reference.
set -u
script="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/identify_image.sh"
fails=0

check() {
	name="$1"; dir="$2"
	out="$(cd "$dir" && "$script" p 2>/dev/null)"; rc=$?
	lines=$(printf '%s\n' "$out" | wc -l)
	if [ "$rc" -ne 0 ]; then
		echo "FAIL $name: exit $rc, expected 0"; fails=$((fails + 1))
	elif [ "$lines" -ne 1 ]; then
		echo "FAIL $name: $lines stdout lines, expected 1"; fails=$((fails + 1))
	else
		echo "ok   $name"
	fi
}

new_repo() {
	d="$(mktemp -d)"
	git -C "$d" init -q -b main
	git -C "$d" config user.email t@example.com
	git -C "$d" config user.name t
	printf 'a prompt\n' > "$d/p"
	echo "$d"
}

# 1. healthy repo: archives and prints
d="$(new_repo)"; check "healthy" "$d"
if ! git -C "$d" cat-file -e HEAD:p 2>/dev/null; then
	echo "FAIL healthy: prompt was not committed"; fails=$((fails + 1))
else
	echo "ok   healthy commits the prompt"
fi

# 2. locked index: git add fails, must not be fatal
d="$(new_repo)"; touch "$d/.git/index.lock"; check "locked index" "$d"

# 3. gitignored prompt: permanently unarchivable, must not be fatal
d="$(new_repo)"; echo p > "$d/.gitignore"; check "gitignored" "$d"

# 4. not a git repo at all
d="$(mktemp -d)"; printf 'a prompt\n' > "$d/p"; check "not a repo" "$d"

[ "$fails" -eq 0 ] || { echo "$fails failure(s)"; exit 1; }
echo "all checks passed"
```

Make it executable: `chmod +x ../Art/huggingface_prompts/test_identify_image.sh`

- [ ] **Step 2: Run it to verify it fails**

Run: `cd ../Art/huggingface_prompts && ./test_identify_image.sh`
Expected: the four `check` cases pass trivially (the current script always exits 0), but **`FAIL healthy: prompt was not committed`** — the archive does not happen yet. That single failure is the point.

- [ ] **Step 3: Implement**

Rewrite `../Art/huggingface_prompts/identify_image.sh`:

```sh
#!/bin/sh

if [ $# -ne 1 ]; then
	echo "USAGE: identify_image <prompt-path>"
	exit 0
fi

# Archive the prompt in the same operation that mints the reference to it
# (ADR 0001). Git history IS the archive; without this the digest below is a
# dangling pointer, which is how 86% of past prompts were lost.
#
# Every failure warns and continues (ADR 0004): priority is image > prompt >
# link between them, and rename_image.sh moves a fixed ~/Downloads/image.webp,
# so a non-zero exit here strands the image at a path the next generation
# overwrites. Warnings go to stderr - stdout is captured as the filename.
warn() { echo "$*" >&2; }

# git -C resolves pathspecs against -C, not the caller's cwd, so both the
# target and the repo root must be absolute before they are combined.
target="$(CDPATH= cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
if repo="$(git -C "$(dirname -- "$target")" rev-parse --show-toplevel 2>/dev/null)"; then
	if git -C "$repo" check-ignore -q -- "$target"; then
		warn "WARNING: $1 is gitignored - this image's prompt can never be archived."
	elif ! git -C "$repo" add -- "$target" 2>/dev/null; then
		warn "WARNING: could not stage $1 - the link will dangle until this"
		warn "         content is committed. The digest resolves retroactively."
	elif ! git -C "$repo" diff --cached --quiet -- "$target" \
	     && ! git -C "$repo" commit -q -m "snapshot: ${target#"$repo"/}" 2>/dev/null; then
		warn "WARNING: could not commit $1 - the link will dangle until this"
		warn "         content is committed. The digest resolves retroactively."
	fi
else
	warn "WARNING: $1 is not inside a git repository - prompt will not be archived."
fi

echo "$(basename "$1")_$(sha1sum "$1" | cut -d ' ' -f 1)_$(uuidgen)"
```

Note the `exit 0` added to the usage branch: the original fell through and printed nothing, which is fine, but being explicit keeps the "always exit 0" property honest.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd ../Art/huggingface_prompts && ./test_identify_image.sh`
Expected: `all checks passed`.

- [ ] **Step 5: Verify the digest is findable**

```sh
script="$PWD/identify_image.sh"          # run from ../Art/huggingface_prompts
cd "$(mktemp -d)" && git init -q -b main . && git config user.email t@e && git config user.name t
printf 'hello prompt\n' > p
id=$("$script" p)
digest=$(echo "$id" | sed 's/^p_//; s/_[0-9a-f-]\{36\}$//')
git cat-file --batch-all-objects --batch-check='%(objectname) %(objecttype)' \
  | awk '$2=="blob"{print $1}' \
  | while read -r o; do git cat-file blob "$o" | sha1sum; done | grep "$digest"
```

Expected: the digest is printed — the reference resolves.

- [ ] **Step 6: Add a pointer README**

Create `../Art/huggingface_prompts/README.md`:

```markdown
# huggingface_prompts

Generation prompts, one file per prompt. **This repository is the prompt
archive**: `identify_image.sh` commits a prompt before minting the digest that
names it, so an image filename always references content git holds.

Do not "simplify" the git calls out of `identify_image.sh`, and never make it
exit non-zero — `rename_image.sh` moves a fixed `~/Downloads/image.webp`, so a
non-zero exit strands the image at a path the next generation overwrites.
`./test_identify_image.sh` guards both properties.

The reasoning lives with the consumer, in the `publicator.py` repo:
`docs/adr/0001-archive-prompts-at-identity-mint-time.md` and
`docs/adr/0004-fail-open-when-prompt-archiving-fails.md`.
```

- [ ] **Step 7: Commit in the submodule, then bump the pointer**

```bash
cd ../Art/huggingface_prompts
git add identify_image.sh test_identify_image.sh README.md
git commit -m "feat: archive the prompt when minting its identity

Git history is the archive: without this the digest in an image filename
is a dangling pointer, which lost 86% of past prompts. Every failure
warns and continues - a lost link beats a lost image."
git push
cd ..
git add huggingface_prompts
git commit -m "chore: bump huggingface_prompts (archive prompts at mint time)"
```

---

### Task 11: Documentation and the data-repo config

**Files:**
- Modify: `CLAUDE.md` (publicator repo)
- Modify: `docs/ROADMAP.md` (publicator repo)
- Modify: `../Art/publicator.toml` (data repo — commit separately)

**Interfaces:**
- Consumes: the config contract from Task 1.
- Produces: nothing code-facing. This is what makes the feature discoverable.

- [ ] **Step 1: Add the `[prompts]` section to the data repo**

Append to `../Art/publicator.toml`:

```toml
# Image -> generation prompt. This repo owns the grammar; publicator only knows
# the `version` (which exact prompt text) and `lineage` (which prompt file)
# roles. Keep in sync with huggingface_prompts/identify_image.sh — `nix run
# <publicator>#prompt-audit` reports `parsed: 0 of N` if they ever drift.
[prompts]
repo = "huggingface_prompts"
filename = '^(?P<lineage>.+)_(?P<version>[0-9a-f]{40})_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[^.]+$'
version_hash = "sha1"
```

- [ ] **Step 2: Verify it against the real data**

Run: `cd ../Art && nix run <this>#prompt-audit`
Expected: roughly the 2026-08-21 baseline — `parsed` around 2485 of ~11111, `exact` around 885 images / 58 versions, `nearest` around 1600 / 363. Exact numbers will differ as art is added or published; **`parsed` must not be 0**.

- [ ] **Step 3: Commit the data repo**

```bash
cd ../Art
git add publicator.toml
git commit -m "feat(publicator): declare the image filename grammar"
```

- [ ] **Step 4: Document in CLAUDE.md**

In the publicator repo's `CLAUDE.md`, add this subsection to **Architecture**, after the **Thumbnails** paragraph:

```markdown
**Prompt mapping.** A card can show the generation prompt that produced its
image. The art repo's `huggingface_prompts` submodule *is* the archive:
`identify_image.sh` commits a prompt before minting the digest that names it,
so a filename references content git holds (it did not, historically — 86% of
prompt versions were never committed and are unrecoverable).
`publicator/prompts.py` digests every blob in that repo, including unreachable
ones, and resolves a filename to one of three types — `Exact`, `Nearest`,
`Unknown`. **`Nearest` deliberately has no `.text`**: a near-miss must not be
renderable as the real prompt. Never resolve a prompt by looking its basename
up at `HEAD`; measured over the real data that is silently wrong on 404 of 426
pairs, because prompts evolve after the image is made.

The **filename grammar is data, not code** — `publicator.toml`'s `[prompts]`
declares a `re` pattern with `version` (required) and `lineage` (optional)
capture groups plus `version_hash`. Nothing in `src/` may hardcode `sha1` or
the filename shape. `nix run <this>#prompt-audit` reports `parsed: N of M`,
which drops to 0 if that grammar drifts from what `identify_image.sh` produces,
and `nearest`, which rising means archive writes are failing. Both apps that
shell out to git (`publish-next`, `prompt-audit`) must list `pkgs.git` in their
flake `runtimeInputs` — `writeShellApplication` pins PATH to those.
Full design: `docs/superpowers/specs/2026-08-21-prompt-mapping-design.md`,
decisions in `docs/adr/0001`–`0004`.
```

Also add to the **Gotchas** list:

```markdown
- Prompt lookup is content-addressed, never `basename`-at-`HEAD`; and the
  filename grammar lives in `publicator.toml`, not in the code.
```

- [ ] **Step 5: Update the roadmap**

In `docs/ROADMAP.md`, change the bare `- Prompt mapping` line at line 14 to:

```markdown
- ~~Prompt mapping~~ — done; see `docs/adr/0001`–`0004`
```

- [ ] **Step 6: Run the full suite one last time**

Run: `nix develop -c pytest -q`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: document prompt mapping in CLAUDE.md"
```

---

## Verification checklist

After all tasks:

```bash
nix develop -c pytest -q                        # full suite green
nix run <this>#check-steps                      # unrelated drift guard still passes
cd ../Art && nix run <this>#prompt-audit        # parsed non-zero
cd ../Art/huggingface_prompts && ./test_identify_image.sh
rg -n 'sha1|\[0-9a-f\]\{40\}' src/publicator/   # must be empty: no leaked grammar
```

Then open the gallery (`cd ../Art && nix run <this>#publish-next`) and confirm by eye:

1. A card whose prompt is archived shows a green `prompt · <path> · <digest>` block that expands to the prompt text.
2. A card whose prompt is lost shows an amber **prompt NOT ARCHIVED ⚠** block, and the versions inside it are visibly framed as *not* this image's prompt.
3. Searching a phrase from a known prompt returns that art, and the banner states how many candidates were skipped as unarchived.
4. Clicking a lineage label loads `/?lineage=…` with two clearly separated groups.

---

## As executed (2026-08-22)

Delivered on `feature/prompt-mapping` in 19 commits, plus 3 in
`huggingface_prompts`. 148 tests pass. Divergences from the plan above, all
arising from task reviews:

- **Task 1's test fixtures were over-escaped** (4 backslashes in Python source
  yields 2 inside a TOML *literal* string, which the regex reads as an escaped
  backslash). Corrected in place above; the shipped tests use the corrected form.
- **`_stamp` gained a `tz` parameter.** Rendering a commit date in ambient local
  time was inconsistent with `calendar_view.py` and `scheduling.py`, which both
  take `tz` explicitly, and with this module's own "pure, like calendar_view"
  docstring.
- **`PromptArchive` gained a `threading.Lock`.** Task 6 shares one instance
  across `ThreadingHTTPServer` threads, and `_refresh` rebuilds its lookup maps
  in place; a concurrent reader could see them empty (a `Nearest` silently
  becoming `Unknown`), and two concurrent refreshes could corrupt the index for
  the life of the process, since `_head` is set last.
- **`HEAD_PROBE_TTL` was added.** `_refresh` probed `git rev-parse HEAD` on every
  query, which a 3436-image search turns into thousands of serialized subprocess
  spawns.
- **The search form is gated on the archive being present.** Without the gate, a
  data dir with no `[prompts]` section rendered a search box that emptied the
  gallery.
- **`__CARDS__` is now the last placeholder replacement**, so prompt text
  containing a literal `__PROMPTSEARCH__` cannot splice the search form into its
  own `<pre>`.
- **The CSS in §6 assumed a light page.** It is dark (`body` `#1a1a1a`, `.card`
  `#2a2a2a`); several colours were re-measured against their real backdrops, and
  lineage links needed two rules because they render on both a pale summary and
  the dark card.

Not done here, deliberately: pushing either repo, and bumping `../Art`'s
submodule pointer. Those belong together and are the repo owner's to run.
