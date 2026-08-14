"""publications.json writes: append, patch, and atomic replace."""

import json
import os
import tempfile
import time
import uuid
from pathlib import Path
from zoneinfo import ZoneInfo

from publicator.config import load_config, validate_publications
from publicator.entries import STATE_UNPUBLISHED, deviantart_apparition
from publicator.images import compute_sha512
from publicator.scheduling import resolve_ts, zone


def atomic_write_json(path: str, data) -> None:
    d = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(prefix=".pub-", dir=d)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=4)
        os.replace(tmp, path)
    except Exception:
        try: os.unlink(tmp)
        except OSError: pass
        raise


def write_publications(entries: list[dict], json_path: str, config: dict | None = None) -> list[str]:
    """Append entries as state=unpublished, validate, atomic write. Returns their UUIDs in order.
    config: tier/gallery allow-list; defaults to a fresh load_config(cwd) for standalone callers."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        data = []

    tz = zone((config or {}).get("schedule", {}))
    new_uuids = []
    now = int(time.time())
    for i, e in enumerate(entries):
        path = e["path"]
        sha = compute_sha512(path)
        ts = resolve_ts(e, tz)
        apparition = {
            "platformName": "deviantart",
            "state": STATE_UNPUBLISHED,
            "urlElsePublicationName": e["title"],
            "apparitionTimestampIfDifferentThanSubmission": ts,
        }
        price = e.get("price")
        set_or_pop(apparition, "priceIfNotFree", float(price) if price not in (None, "") else None)
        set_or_pop(apparition, "tier", e.get("tier") or None)
        set_or_pop(apparition, "galleries", list(e["galleries"]) if e.get("galleries") else None)
        u = str(uuid.uuid4())
        new_uuids.append(u)
        data.append({
            "uuid": u,
            "submissionTimestamp": now,
            "description": e.get("description", ""),
            "files": [{
                "basename": os.path.basename(path),
                "sha512sum": sha,
            }],
            "apparitions": [apparition],
        })

    validate_publications(data, config if config is not None else load_config(Path.cwd()))
    atomic_write_json(json_path, data)
    return new_uuids


def mark_state(json_path: str, target_uuid: str, state: str) -> None:
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)
    for p in data:
        if p.get("uuid") == target_uuid:
            deviantart_apparition(p)["state"] = state
            break
    atomic_write_json(json_path, data)


def set_or_pop(d, k, v):
    if v in (None, [], ""):
        d.pop(k, None)
    else:
        d[k] = v


def apply_update(pubs, uuid_, fields, zone: ZoneInfo):
    """Patch the publication (and its DA apparition) with uuid_ in place.
    Cleared price/tier/galleries are removed. Raises KeyError if not found.
    `zone` resolves the naive 'schedule' string (see resolve_ts)."""
    for p in pubs:
        if p.get("uuid") == uuid_:
            break
    else:
        raise KeyError(uuid_)
    p["description"] = fields.get("description", p.get("description", ""))
    app = deviantart_apparition(p)
    app["urlElsePublicationName"] = fields["title"]
    app["apparitionTimestampIfDifferentThanSubmission"] = resolve_ts(fields, zone)
    price = fields.get("price")
    set_or_pop(app, "priceIfNotFree", float(price) if price not in (None, "") else None)
    set_or_pop(app, "tier", fields.get("tier") or None)
    set_or_pop(app, "galleries", list(fields["galleries"]) if fields.get("galleries") else None)
