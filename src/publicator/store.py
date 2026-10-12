"""publications.json writes: pick, phase transitions, patch, and atomic replace."""

import contextlib
import fcntl
import json
import os
import tempfile
import threading
import time
import uuid
from pathlib import Path

from publicator.config import load_config, validate_publications
from publicator.entries import (
    APPARITION_STATE_PUBLISHED,
    APPARITION_STATE_UNPUBLISHED,
    PUBLICATION_STATE_APPEARED,
    PUBLICATION_STATE_DESCRIBED,
    PUBLICATION_STATE_PICKED,
    PUBLICATION_STATE_SCHEDULED,
    has_delivery_success,
)
from publicator.images import compute_sha512
from publicator.llm_meta import TODO_GENERATE
from publicator.scheduling import repack_publication_dates, resolve_ts, schedule_profiles, zone

_THREAD_LOCK = threading.RLock()


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


@contextlib.contextmanager
def _mutation_lock(json_path: str):
    lock_path = f"{json_path}.lock"
    Path(lock_path).parent.mkdir(parents=True, exist_ok=True)
    with _THREAD_LOCK:
        with open(lock_path, "a", encoding="utf-8") as f:
            fcntl.flock(f.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(f.fileno(), fcntl.LOCK_UN)


def _read_publications(json_path: str) -> list[dict]:
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            return json.load(f)
    except FileNotFoundError:
        return []


def _config(config: dict | None) -> dict:
    return config if config is not None else load_config(Path.cwd())


def _write_publications(json_path: str, data: list[dict], config: dict | None) -> None:
    validate_publications(data, _config(config))
    atomic_write_json(json_path, data)


def _find(data: list[dict], uuid_: str) -> dict:
    for pub in data:
        if pub.get("uuid") == uuid_:
            return pub
    raise KeyError(uuid_)


def _default_profile(config: dict | None) -> str:
    raw = (_config(config).get("schedule") or {}).get("profiles")
    return str(raw[0].get("name", "default")) if raw else "default"


def _sha_matches(pub: dict, sha: str) -> bool:
    return any(f.get("sha512sum") == sha or f.get("fileSha512sum") == sha
               for f in pub.get("files", []))


def pick_publication(path: str, json_path: str, config: dict | None = None) -> str:
    """Persist a PICKED publication once, deduplicated by content hash."""
    sha = compute_sha512(path)
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        for pub in data:
            if _sha_matches(pub, sha):
                return pub["uuid"]
        uuid_ = str(uuid.uuid4())
        data.append({
            "uuid": uuid_,
            "state": PUBLICATION_STATE_PICKED,
            "title": "",
            "submissionTimestamp": int(time.time()),
            "description": "",
            "files": [{
                "basename": os.path.basename(path),
                "sha512sum": sha,
            }],
            "apparitions": [],
        })
        _write_publications(json_path, data, config)
        return uuid_


def edit_publication_date(json_path: str, uuid_: str, fields: dict, config: dict | None = None) -> None:
    """Pin a date, or reallocate automatically on profile-only/unpin edits."""
    cfg = _config(config)
    manual = bool(fields.get("schedule") or fields.get("scheduleTs"))
    if not manual and "scheduleProfile" not in fields and fields.get("scheduleIsManual") is not False:
        raise ValueError("date, profile, or unpin required")
    ts = resolve_ts(fields, zone(cfg.get("schedule", {}))) if manual else None
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_PICKED:
            raise ValueError("date edits require PICKED")
        profile = str(fields.get("scheduleProfile") or pub.get("scheduleProfile") or _default_profile(cfg))
        profile_names = {p["name"] for p in schedule_profiles(cfg.get("schedule", {}))}
        if "scheduleProfile" in fields and profile not in profile_names:
            raise ValueError("unknown schedule profile")
        if manual:
            pub["apparitionTimestampIfDifferentThanSubmission"] = ts
        if "scheduleProfile" in fields or not manual:
            pub["scheduleProfile"] = profile if profile in profile_names else _default_profile(cfg)
        pub["scheduleIsManual"] = manual
        repack_publication_dates(data, cfg.get("schedule", {}))
        _write_publications(json_path, data, cfg)


def approve_publication_date(json_path: str, uuid_: str, config: dict | None = None) -> None:
    cfg = _config(config)
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_PICKED:
            raise ValueError("date approval requires PICKED")
        if pub.get("apparitionTimestampIfDifferentThanSubmission") is None:
            raise ValueError("date required")
        pub["state"] = PUBLICATION_STATE_SCHEDULED
        repack_publication_dates(data, cfg.get("schedule", {}))
        _write_publications(json_path, data, cfg)


def save_publication_metadata(json_path: str, uuid_: str, *, title: str | None = None,
                              description: str | None = None,
                              config: dict | None = None) -> None:
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_SCHEDULED:
            raise ValueError("metadata edits require SCHEDULED")
        if title is not None:
            pub["title"] = title
        if description is not None:
            pub["description"] = description
        _write_publications(json_path, data, config)


def merge_generated_metadata(json_path: str, uuid_: str, *, title: str, description: str,
                            config: dict | None = None) -> None:
    """Merge provider results into still-marked fields without changing approval state."""
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        for field, value in (("title", title), ("description", description)):
            if pub.get(field) == TODO_GENERATE:
                pub[field] = value
        _write_publications(json_path, data, config)


def approve_publication_metadata(json_path: str, uuid_: str, config: dict | None = None) -> None:
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if pub.get("state") != PUBLICATION_STATE_SCHEDULED:
            raise ValueError("metadata approval requires SCHEDULED")
        if any(not pub.get(key, "").strip() or TODO_GENERATE in pub[key]
               for key in ("title", "description")):
            raise ValueError("title and description required")
        pub["state"] = PUBLICATION_STATE_DESCRIBED
        _write_publications(json_path, data, config)


def return_to_metadata(json_path: str, uuid_: str, config: dict | None = None) -> None:
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_DESCRIBED:
            raise ValueError("return to metadata requires DESCRIBED")
        pub["state"] = PUBLICATION_STATE_SCHEDULED
        _write_publications(json_path, data, config)


def return_to_scheduling(json_path: str, uuid_: str, config: dict | None = None) -> None:
    cfg = _config(config)
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if (pub.get("apparitionTimestampIfDifferentThanSubmission") is not None
                and pub.get("scheduleProfile") not in {
                    p["name"] for p in schedule_profiles(cfg.get("schedule", {}))}):
            pub["scheduleIsManual"] = True
        pub["state"] = PUBLICATION_STATE_PICKED
        _write_publications(json_path, data, cfg)


def load_publications(json_path: str) -> list[dict]:
    return _read_publications(json_path)


def mark_state(json_path: str, target_uuid: str, state: str, *,
               platform: str = "deviantart", config: dict | None = None) -> None:
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        for p in data:
            if p.get("uuid") != target_uuid:
                continue
            for app in p.get("apparitions", []):
                if app.get("platformName") == platform:
                    app["state"] = state
                    break
            if state == APPARITION_STATE_PUBLISHED and p.get("apparitions"):
                p["state"] = (PUBLICATION_STATE_APPEARED
                              if all(a.get("state") == APPARITION_STATE_PUBLISHED
                                     for a in p.get("apparitions", []))
                              else PUBLICATION_STATE_DESCRIBED)
            break
        _write_publications(json_path, data,
                            config if config is not None else load_config(Path(json_path).parent))


def set_or_pop(d, k, v):
    if v in (None, [], ""):
        d.pop(k, None)
    else:
        d[k] = v


def repack_publications(json_path: str, config: dict | None = None) -> None:
    """Persist automatic PICKED suggestions, keeping approved/manual dates frozen."""
    cfg = _config(config)
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        repack_publication_dates(data, cfg.get("schedule", {}))
        _write_publications(json_path, data, cfg)


def select_publication_platforms(json_path: str, *, selected: bool,
                                 registered_platforms: list[str], uuid_: str | None = None,
                                 platform: str | None = None, config: dict | None = None) -> None:
    """Select one cell or all eligible cells in one validated mutation."""
    if type(selected) is not bool:
        raise ValueError("selected must be boolean")
    if platform is not None and platform not in registered_platforms:
        raise ValueError("unregistered platform")
    if (uuid_ is None) != (platform is None):
        raise ValueError("uuid and platform required together")
    cfg = _config(config)
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pubs = [_find(data, uuid_)] if uuid_ is not None else data
        for pub in pubs:
            if pub.get("state") not in (PUBLICATION_STATE_DESCRIBED, PUBLICATION_STATE_APPEARED):
                if uuid_ is not None:
                    raise ValueError("platform selection requires DESCRIBED or APPEARED")
                continue
            for name in [platform] if platform else registered_platforms:
                apps = pub.setdefault("apparitions", [])
                app = next((a for a in apps if a.get("platformName") == name), None)
                if app and app.get("state") == APPARITION_STATE_PUBLISHED:
                    if uuid_ is not None:
                        raise ValueError("successful selection is locked")
                    continue
                if selected and app is None:
                    if any(not pub.get(k, "").strip() or TODO_GENERATE in pub[k]
                           for k in ("title", "description")):
                        raise ValueError("approved metadata required")
                    if name == "deviantart" and len(pub["title"]) > 50:
                        raise ValueError("DeviantArt title must be at most 50 characters")
                    app = {"platformName": name, "state": APPARITION_STATE_UNPUBLISHED}
                    if name == "deviantart":
                        profile = next((p for p in schedule_profiles(cfg.get("schedule", {}))
                                        if p["name"] == pub.get("scheduleProfile")), None)
                        if profile and profile["default_tiers"]:
                            app["tiers"] = list(profile["default_tiers"])
                    apps.append(app)
                    pub["state"] = PUBLICATION_STATE_DESCRIBED
                elif not selected and app is not None:
                    apps.remove(app)
            if pub.get("apparitions") and all(a["state"] == APPARITION_STATE_PUBLISHED for a in pub["apparitions"]):
                pub["state"] = PUBLICATION_STATE_APPEARED
        _write_publications(json_path, data, cfg)


def save_platform_options(json_path: str, uuid_: str, platform: str, fields: dict,
                          config: dict | None = None) -> None:
    """Edit existing undelivered DeviantArt options; never create a selection."""
    if platform != "deviantart" or set(fields) - {"price", "tiers", "galleries"}:
        raise ValueError("unsupported platform options")
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        pub = _find(data, uuid_)
        if pub.get("state") != PUBLICATION_STATE_DESCRIBED:
            raise ValueError("options require DESCRIBED")
        app = next((a for a in pub.get("apparitions", []) if a.get("platformName") == platform), None)
        if app is None or app.get("state") != APPARITION_STATE_UNPUBLISHED:
            raise ValueError("undelivered selection required")
        if "price" in fields:
            price = fields["price"]
            if price not in (None, "") and (type(price) not in (int, float) or not 0 <= price < float("inf")):
                raise ValueError("price must be a finite nonnegative number")
            set_or_pop(app, "priceIfNotFree", price)
        for key in ("tiers", "galleries"):
            if key in fields:
                value = fields[key]
                if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
                    raise ValueError(f"{key} must be a list of names")
                set_or_pop(app, key, value)
        _write_publications(json_path, data, config)
