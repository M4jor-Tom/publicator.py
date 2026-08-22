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
import hashlib
import os
import re
import subprocess
import threading
import time
from dataclasses import dataclass

# ponytail: one `git rev-parse HEAD` per query call made a bulk scan spawn
# thousands of subprocesses. Probe at most every HEAD_PROBE_TTL seconds; a
# prompt committed mid-render shows up on the next reload. Drop the TTL if the
# archive ever needs to be read-your-writes.
HEAD_PROBE_TTL = 2.0


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
        self._probed: float | None = None
        self._versions: dict[str, PromptVersion] = {}
        self._by_basename: dict[str, list[str]] = {}
        self._by_path: dict[str, list[str]] = {}      # path -> [digest]
        # GalleryHandler holds one PromptArchive as a class attribute and
        # ThreadingHTTPServer serves each request on its own thread, so a
        # reload racing a slow render (or a second browser tab) can otherwise
        # read _by_path/_by_basename while they are still empty or mid-rebuild
        # — an empty read, not just a stale one: paths_for() answers () too
        # early, resolve() calls that Unknown, and a card silently shows no
        # prompt block where it should show a NOT ARCHIVED warning. Worse, two
        # concurrent refreshes can each wipe the other's half-built dicts and
        # both still set _head last — the head-unchanged early-return then
        # always succeeds, so the corrupted index persists for the life of the
        # process. A plain Lock is enough: _refresh only calls
        # _git/_blobs/_history, never a public query method, so there is no
        # re-entrancy.
        self._lock = threading.Lock()

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
        now = time.monotonic()
        if self._probed is not None and now - self._probed < HEAD_PROBE_TTL:
            return
        head = self._git("rev-parse", "HEAD").decode().strip()
        self._probed = now
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
        with self._lock:
            self._refresh()
            return self._versions

    def paths_for(self, basename: str) -> tuple[str, ...]:
        with self._lock:
            self._refresh()
            return tuple(self._by_basename.get(basename, ()))

    def versions_at(self, path: str) -> tuple[PromptVersion, ...]:
        with self._lock:
            self._refresh()
            return tuple(sorted((self._versions[d] for d in self._by_path.get(path, ())),
                                key=lambda v: (-v.committed, v.version)))

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

    def parse(self, path: str) -> ImageIdentity | None:
        """The roles this art file's name claims, or None if it is not
        prompt-bearing. `resolve_path` is parse + resolve; the audit needs the
        two halves separately, to count distinct versions."""
        return parse_identity(os.path.basename(path), self.pattern)

    def resolve_path(self, path: str) -> PromptMatch:
        """Resolve an art file by its path. Non-prompt-bearing names — most of
        the data dir — come back Unknown, which renders as nothing."""
        identity = self.parse(path)
        if identity is None:
            return Unknown(reason="filename does not match the configured grammar")
        return self.resolve(identity, near=os.path.dirname(path))


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
