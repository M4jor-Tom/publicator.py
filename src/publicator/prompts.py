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
