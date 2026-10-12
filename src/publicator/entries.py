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
# so each app/webui caller can point them at its own --data-dir.
DATA_DIR = Path.cwd()

# publications.json publication states: completed workflow phases.
PUBLICATION_STATE_PICKED = "PICKED"
PUBLICATION_STATE_SCHEDULED = "SCHEDULED"
PUBLICATION_STATE_DESCRIBED = "DESCRIBED"
PUBLICATION_STATE_APPEARED = "APPEARED"
PUBLICATION_STATES = {
    PUBLICATION_STATE_PICKED,
    PUBLICATION_STATE_SCHEDULED,
    PUBLICATION_STATE_DESCRIBED,
    PUBLICATION_STATE_APPEARED,
}

# publications.json apparition delivery states. Old aliases stay for still-migrating
# publisher code.
APPARITION_STATE_UNPUBLISHED = "unpublished"
APPARITION_STATE_PUBLISHED = "published_or_scheduled"
STATE_UNPUBLISHED = APPARITION_STATE_UNPUBLISHED
STATE_PUBLISHED = APPARITION_STATE_PUBLISHED


def has_delivery_success(pub: dict) -> bool:
    return any(a.get("state") == APPARITION_STATE_PUBLISHED for a in pub.get("apparitions", []))


def effective_schedule_ts(pub: dict) -> int | None:
    ts = pub.get("apparitionTimestampIfDifferentThanSubmission")
    return int(ts) if ts is not None else pub.get("submissionTimestamp")


def set_data_dir(path: str | None = None) -> Path:
    """Point the helpers at the publication database dir: the given path, else CWD."""
    global DATA_DIR
    DATA_DIR = Path(path).resolve() if path else Path.cwd()
    return DATA_DIR


def pubs_file() -> Path:
    return DATA_DIR / "publications.json"


def load_pubs() -> list[dict]:
    return json.loads(pubs_file().read_text())


def platform_apparition(pub: dict, platform: str) -> dict:
    for a in pub.get("apparitions", []):
        if a.get("platformName") == platform:
            return a
    raise SystemExit(f"no {platform} apparition on {pub['uuid']}")


def first_unpublished(platform: str | None = None) -> dict:
    for p in load_pubs():
        if p.get("state") != PUBLICATION_STATE_DESCRIBED:
            continue
        for a in p.get("apparitions", []):
            if platform is not None and a.get("platformName") != platform:
                continue
            if a.get("state") == STATE_UNPUBLISHED:
                return p
    raise SystemExit("no unpublished publication")


def deviantart_apparition(pub: dict) -> dict:
    return platform_apparition(pub, "deviantart")


def find_art_path(basename: str) -> Path:
    """Absolute path to the non-webp art file, searched under the data dir."""
    stem = basename.rsplit(".", 1)[0]
    for p in DATA_DIR.rglob(f"*{stem}*"):
        if p.suffix.lower() != ".webp":
            return p.resolve()
    raise SystemExit(f"no non-webp file for {basename}")


def format_schedule(ts: int) -> str:
    # Match `date --date @TS` — deviantart.parse_schedule expects it.
    return subprocess.check_output(["date", "--date", f"@{ts}"]).decode().strip()
