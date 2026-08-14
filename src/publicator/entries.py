"""The publications.json entry model: locate entries, their art, their schedule.

Source of truth for "unpublished" is the apparition `state` field, not git.
The CLI that prints the first unpublished entry lives in apps/echo_first.py.
"""
import json
import subprocess
from pathlib import Path

# Data (publications.json + images) lives in the publication database dir: the
# --data-dir passed to each app, defaulting to CWD. Only code assets (schema,
# tags/) travel with this package. set_data_dir() retargets the helpers below,
# so publish_next.py can point them at its own --data-dir.
DATA_DIR = Path.cwd()

# publications.json apparition states. They live here, with the entry model, so
# scheduling.py can filter on them without importing the Playwright module.
STATE_UNPUBLISHED = "unpublished"
STATE_PUBLISHED = "published_or_scheduled"


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
            if a.get("state") == STATE_UNPUBLISHED:
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
