"""Candidate discovery (hash-dedup against publications.json) and thumbnails."""

import hashlib
import json
import mimetypes
import os
import random
import shutil
import subprocess
import sys

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tiff", ".mp4"}
THUMB_CACHE = ".thumbs"     # under the data dir: one thumbnail cache for the whole UI


def guess_mime(path: str) -> str:
    return mimetypes.guess_type(path)[0] or "application/octet-stream"


def compute_sha512(filepath: str) -> str:
    with open(filepath, "rb") as f:
        return hashlib.file_digest(f, "sha512").hexdigest()


def load_publicated_hashes(json_path: str) -> set[str]:
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        return set()
    publicated = set()
    for entry in data:
        for file_obj in entry.get("files", []) + entry.get("previewFilesIfNotFree", []):
            sha = file_obj.get("sha512sum") or file_obj.get("fileSha512sum")
            if sha:
                publicated.add(sha.lower())
    return publicated


def collect_images(directory: str) -> list[str]:
    images = []
    for root, _, files in os.walk(directory):
        for filename in files:
            if os.path.splitext(filename)[1].lower() in IMAGE_EXTENSIONS:
                images.append(os.path.join(root, filename))
    return images


def find_candidates(directories: list[str], json_path: str, limit: int) -> list[str]:
    publicated = load_publicated_hashes(json_path)
    images = []
    for d in directories:
        images.extend(collect_images(d))   # os.walk on a missing dir yields nothing
    random.shuffle(images)
    candidates = []
    for path in images:
        if len(candidates) >= limit:
            break
        try:
            if compute_sha512(path) not in publicated:
                candidates.append(path)
        except OSError as e:
            print(f"Error hashing {path}: {e}", file=sys.stderr)
    return candidates


def thumb_name(path: str, sha: str | None = None) -> str:
    """Cache filename for this art's thumbnail: content-addressed, so the same
    image keeps one thumbnail across runs and publications.json's stored sha512
    saves re-hashing. The extension is kept so the served mime type stays right."""
    return (sha or compute_sha512(path))[:32] + os.path.splitext(path)[1].lower()


def index_by_basename(root: str) -> dict[str, str]:
    """{basename: path} for everything under the data dir — one walk, so the
    published back-catalogue resolves to files without a search per entry.
    Hidden dirs are skipped: the thumbnail cache and the browser profiles."""
    index = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not d.startswith(".")]
        for f in filenames:
            index.setdefault(f, os.path.join(dirpath, f))
    return index


def ensure_thumb(path: str, cache_dir: str, name: str | None = None) -> str:
    """Path to this art's cached thumbnail, generated on a miss. One cache for
    the whole UI (gallery cards + calendar), living in the data dir — thumbnails
    used to be built per run into /tmp and thrown away. `name` short-circuits the
    hashing when the caller already knows the cache name."""
    thumb = os.path.join(cache_dir, name or thumb_name(path))
    if not os.path.exists(thumb):
        os.makedirs(cache_dir, exist_ok=True)
        try:
            subprocess.run(
                ["convert", path, "-resize", "300x300>", thumb],
                capture_output=True, check=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            shutil.copy2(path, thumb)
    return thumb


def thumb_for_ai(path: str, cache_dir: str, name: str | None = None) -> str:
    """What to hand a vision model: the thumbnail, not the full-res original — same
    visual info for a fraction of the tokens/latency. Every AI caller comes here."""
    try:
        return ensure_thumb(path, cache_dir, name)
    except OSError:
        return path      # unwritable cache: costlier, still works
