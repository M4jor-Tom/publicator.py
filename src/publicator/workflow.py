"""Pick and scheduling state transitions for persisted publications."""

import os
import time
import uuid

from publicator.entries import (
    PUBLICATION_STATE_PICKED,
    PUBLICATION_STATE_SCHEDULED,
    has_delivery_success,
)
from publicator.images import compute_sha512
from publicator.scheduling import repack_publication_dates, resolve_ts, schedule_profiles, zone
from publicator import store


def _default_profile(config: dict) -> str:
    profiles = (config.get("schedule") or {}).get("profiles")
    return str(profiles[0].get("name", "default")) if profiles else "default"


def _sha_matches(pub: dict, sha: str) -> bool:
    return any(f.get("sha512sum") == sha or f.get("fileSha512sum") == sha
               for f in pub.get("files", []))


def pick_publication(path: str, json_path: str, config: dict | None = None) -> str:
    """Persist a PICKED publication once, deduplicated by content hash."""
    sha = compute_sha512(path)
    uuid_ = str(uuid.uuid4())
    publication = {
        "uuid": uuid_,
        "state": PUBLICATION_STATE_PICKED,
        "title": "",
        "submissionTimestamp": int(time.time()),
        "description": "",
        "files": [{"basename": os.path.basename(path), "sha512sum": sha}],
        "apparitions": [],
    }

    def mutation(data):
        for pub in data:
            if _sha_matches(pub, sha):
                return pub["uuid"]
        data.append(publication)
        return uuid_

    return store.mutate_publications(json_path, mutation, config)


def edit_publication_date(json_path: str, uuid_: str, fields: dict,
                           config: dict | None = None) -> None:
    cfg = store._config(config)
    schedule = cfg.get("schedule", {})
    manual = bool(fields.get("schedule") or fields.get("scheduleTs"))
    if not manual and "scheduleProfile" not in fields and fields.get("scheduleIsManual") is not False:
        raise ValueError("date, profile, or unpin required")
    ts = resolve_ts(fields, zone(schedule)) if manual else None
    profile_names = {profile["name"] for profile in schedule_profiles(schedule)}

    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_PICKED:
            raise ValueError("date edits require PICKED")
        profile = str(fields.get("scheduleProfile") or pub.get("scheduleProfile") or _default_profile(cfg))
        if "scheduleProfile" in fields and profile not in profile_names:
            raise ValueError("unknown schedule profile")
        if manual:
            pub["apparitionTimestampIfDifferentThanSubmission"] = ts
        if "scheduleProfile" in fields or not manual:
            pub["scheduleProfile"] = profile if profile in profile_names else _default_profile(cfg)
        pub["scheduleIsManual"] = manual
        repack_publication_dates(data, schedule)

    store.mutate_publications(json_path, mutation, config)


def approve_publication_date(json_path: str, uuid_: str, config: dict | None = None) -> None:
    cfg = store._config(config)
    schedule = cfg.get("schedule", {})

    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_PICKED:
            raise ValueError("date approval requires PICKED")
        if pub.get("apparitionTimestampIfDifferentThanSubmission") is None:
            raise ValueError("date required")
        pub["state"] = PUBLICATION_STATE_SCHEDULED
        repack_publication_dates(data, schedule)

    store.mutate_publications(json_path, mutation, config)


def repack_publications(json_path: str, config: dict | None = None) -> None:
    cfg = store._config(config)
    store.mutate_publications(
        json_path, lambda data: repack_publication_dates(data, cfg.get("schedule", {})), config)
