#!/usr/bin/env python3
"""Print path/title/schedule for the first state=unpublished publication.

Replaces echo_first_unstaged_publication_data.sh: source of truth is
publications.json state field, not git diff --staged.
Also exposes helpers imported by publish_next.py.
"""
import json
import subprocess
import sys
from pathlib import Path

# Data lives in the caller's CWD (the app is invoked from the Art data dir).
# Only code assets travel with this package; data/images resolve against cwd.
PUBS_FILE = Path("publications.json")


def load_pubs() -> list[dict]:
    return json.loads(PUBS_FILE.read_text())


def first_unpublished() -> dict:
    for p in load_pubs():
        if p.get("state") == "unpublished":
            return p
    raise SystemExit("no unpublished publication")


def deviantart_apparition(pub: dict) -> dict:
    for a in pub.get("apparitions", []):
        if a.get("platformName") == "deviantart":
            return a
    raise SystemExit(f"no deviantart apparition on {pub['uuid']}")


def find_art_path(basename: str) -> Path:
    """Absolute path to the non-webp art file, searched under the data dir (cwd)."""
    stem = basename.rsplit(".", 1)[0]
    for p in Path.cwd().rglob(f"*{stem}*"):
        if p.suffix != ".webp":
            return p.resolve()
    raise SystemExit(f"no non-webp file for {basename}")


def format_schedule(ts: int) -> str:
    # Match `date --date @TS` — parse_schedule in publish_next.py expects it.
    return subprocess.check_output(["date", "--date", f"@{ts}"]).decode().strip()


def mark_published_or_scheduled(uuid: str) -> None:
    pubs = load_pubs()
    for p in pubs:
        if p.get("uuid") == uuid:
            p["state"] = "published_or_scheduled"
            PUBS_FILE.write_text(json.dumps(pubs, indent=4))
            return
    raise SystemExit(f"uuid not found: {uuid}")


def main() -> int:
    pub = first_unpublished()
    app = deviantart_apparition(pub)
    ts = app.get("apparitionTimestampIfDifferentThanSubmission")
    if ts is None:
        raise SystemExit(f"no apparitionTimestampIfDifferentThanSubmission on {pub['uuid']}")
    print(f"path: {find_art_path(pub['files'][0]['basename'])}")
    print(f"title: {app['urlElsePublicationName']}")
    print(f"schedule: {format_schedule(ts)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
