"""Candidate discovery (hash-dedup against publications.json) and thumbnails."""

import hashlib
import json
import os
import random
import shutil
import subprocess
import sys

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tiff", ".mp4"}
MIME = {
    ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png",
    ".webp": "image/webp", ".gif": "image/gif", ".mp4": "video/mp4",
}


def guess_mime(path: str) -> str:
    return MIME.get(os.path.splitext(path)[1].lower(), "application/octet-stream")


def compute_sha512(filepath: str, chunk_size: int = 1024 * 1024) -> str:
    h = hashlib.sha512()
    with open(filepath, "rb") as f:
        while chunk := f.read(chunk_size):
            h.update(chunk)
    return h.hexdigest()


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


def generate_thumbnails(image_paths: list[str], thumb_dir: str) -> dict[str, str]:
    os.makedirs(thumb_dir, exist_ok=True)
    path_to_thumb = {}
    for i, path in enumerate(image_paths):
        ext = os.path.splitext(path)[1].lower()
        thumb_path = os.path.join(thumb_dir, f"thumb_{i:04d}{ext}")
        try:
            subprocess.run(
                ["convert", path, "-resize", "300x300>", thumb_path],
                capture_output=True, check=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            shutil.copy2(path, thumb_path)
        path_to_thumb[path] = thumb_path
    return path_to_thumb
