#!/usr/bin/env python3
"""Print path/title/schedule for the first state=unpublished publication.

Replaces echo_first_unstaged_publication_data.sh: source of truth is
publications.json state field, not git diff --staged.
Also exposes helpers imported by publish_next.py.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

# Data (publications.json + images) lives in the publication database dir: the
# --data-dir passed to each app, defaulting to CWD. Only code assets (schema,
# tags/) travel with this package. set_data_dir() retargets the helpers below,
# so publish_next.py can point them at its own --data-dir.
DATA_DIR = Path.cwd()


def set_data_dir(path: str | None = None) -> Path:
    """Point the helpers at the publication database dir: the given path, else CWD."""
    global DATA_DIR
    DATA_DIR = Path(path).resolve() if path else Path.cwd()
    return DATA_DIR


def pubs_file() -> Path:
    return DATA_DIR / "publications.json"


def load_pubs() -> list[dict]:
    return json.loads(pubs_file().read_text())


def first_unpublished() -> dict:
    for p in load_pubs():
        for a in p.get("apparitions", []):
            if a.get("state") == "unpublished":
                return p
    raise SystemExit("no unpublished publication")


def deviantart_apparition(pub: dict) -> dict:
    for a in pub.get("apparitions", []):
        if a.get("platformName") == "deviantart":
            return a
    raise SystemExit(f"no deviantart apparition on {pub['uuid']}")


def find_art_path(basename: str) -> Path:
    """Absolute path to the non-webp art file, searched under the data dir."""
    stem = basename.rsplit(".", 1)[0]
    for p in DATA_DIR.rglob(f"*{stem}*"):
        if p.suffix != ".webp":
            return p.resolve()
    raise SystemExit(f"no non-webp file for {basename}")


def format_schedule(ts: int) -> str:
    # Match `date --date @TS` — parse_schedule in publish_next.py expects it.
    return subprocess.check_output(["date", "--date", f"@{ts}"]).decode().strip()


def main() -> int:
    ap = argparse.ArgumentParser(description="Print the first unpublished publication's data.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir (publications.json + images); default: CWD")
    set_data_dir(ap.parse_args().data_dir)
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
