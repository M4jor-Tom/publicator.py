"""Pick and scheduling state transitions for persisted publications."""

import os
import time
import uuid

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


def save_publication_metadata(json_path: str, uuid_: str, *, title: str | None = None,
                              description: str | None = None,
                              config: dict | None = None) -> None:
    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_SCHEDULED:
            raise ValueError("metadata edits require SCHEDULED")
        if title is not None:
            pub["title"] = title
        if description is not None:
            pub["description"] = description

    store.mutate_publications(json_path, mutation, config)


def _generated_metadata_is_complete(pub: dict) -> bool:
    return all(isinstance(pub.get(key), str) and pub[key].strip()
               and TODO_GENERATE not in pub[key] for key in ("title", "description"))


def save_generated_metadata(json_path: str, uuid_: str, title: str, description: str,
                            *, config: dict | None = None) -> None:
    """Save the provider's complete pair, keeping failures retryable."""
    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_SCHEDULED:
            raise ValueError("metadata generation requires SCHEDULED")
        pub["title"] = title
        pub["description"] = description
        if _generated_metadata_is_complete(pub):
            pub["state"] = PUBLICATION_STATE_DESCRIBED

    store.mutate_publications(json_path, mutation, config)


def merge_generated_metadata(json_path: str, uuid_: str, title: str, description: str,
                             *, config: dict | None = None) -> None:
    """Replace only exact markers; advance only after both fields are complete."""
    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_SCHEDULED:
            raise ValueError("metadata generation requires SCHEDULED")
        for field, value in (("title", title), ("description", description)):
            if pub.get(field) == TODO_GENERATE:
                pub[field] = value
        if _generated_metadata_is_complete(pub):
            pub["state"] = PUBLICATION_STATE_DESCRIBED

    store.mutate_publications(json_path, mutation, config)


def approve_publication_metadata(json_path: str, uuid_: str, config: dict | None = None) -> None:
    def mutation(data):
        pub = store._find(data, uuid_)
        if pub.get("state") != PUBLICATION_STATE_SCHEDULED:
            raise ValueError("metadata approval requires SCHEDULED")
        if any(not pub.get(key, "").strip() or TODO_GENERATE in pub[key]
               for key in ("title", "description")):
            raise ValueError("title and description required")
        pub["state"] = PUBLICATION_STATE_DESCRIBED

    store.mutate_publications(json_path, mutation, config)


def return_to_metadata(json_path: str, uuid_: str, config: dict | None = None) -> None:
    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if pub.get("state") != PUBLICATION_STATE_DESCRIBED:
            raise ValueError("return to metadata requires DESCRIBED")
        pub["state"] = PUBLICATION_STATE_SCHEDULED

    store.mutate_publications(json_path, mutation, config)


def return_to_scheduling(json_path: str, uuid_: str, config: dict | None = None) -> None:
    cfg = store._config(config)

    def mutation(data):
        pub = store._find(data, uuid_)
        if has_delivery_success(pub):
            raise ValueError("delivered publication cannot be edited")
        if (pub.get("apparitionTimestampIfDifferentThanSubmission") is not None
                and pub.get("scheduleProfile") not in {
                    p["name"] for p in schedule_profiles(cfg.get("schedule", {}))}):
            pub["scheduleIsManual"] = True
        pub["state"] = PUBLICATION_STATE_PICKED

    store.mutate_publications(json_path, mutation, cfg)


def repack_publications(json_path: str, config: dict | None = None) -> None:
    cfg = store._config(config)
    store.mutate_publications(
        json_path, lambda data: repack_publication_dates(data, cfg.get("schedule", {})), config)


def select_publication_platforms(json_path: str, *, selected: bool,
                                 registered_platforms: list[str], uuid_: str | None = None,
                                 platform: str | None = None, config: dict | None = None) -> None:
    """Select eligible platform targets in one validated mutation."""
    if type(selected) is not bool:
        raise ValueError("selected must be boolean")
    if platform is not None and platform not in registered_platforms:
        raise ValueError("unregistered platform")
    if (uuid_ is None) != (platform is None):
        raise ValueError("uuid and platform required together")
    cfg = store._config(config)

    def mutation(data):
        pubs = [store._find(data, uuid_)] if uuid_ is not None else data
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
                        if profile:
                            if profile["default_tiers"]:
                                app["tiers"] = list(profile["default_tiers"])
                            if profile["default_price"] is not None:
                                app["priceIfNotFree"] = profile["default_price"]
                    apps.append(app)
                    pub["state"] = PUBLICATION_STATE_DESCRIBED
                elif not selected and app is not None:
                    apps.remove(app)
            if pub.get("apparitions") and all(
                    a["state"] == APPARITION_STATE_PUBLISHED for a in pub["apparitions"]):
                pub["state"] = PUBLICATION_STATE_APPEARED

    store.mutate_publications(json_path, mutation, cfg)


def save_platform_options(json_path: str, uuid_: str, platform: str, fields: dict,
                          config: dict | None = None) -> None:
    """Edit existing undelivered DeviantArt options; never create a selection."""
    if platform != "deviantart" or set(fields) - {"price", "tiers", "galleries"}:
        raise ValueError("unsupported platform options")

    def mutation(data):
        pub = store._find(data, uuid_)
        if pub.get("state") != PUBLICATION_STATE_DESCRIBED:
            raise ValueError("options require DESCRIBED")
        app = next((a for a in pub.get("apparitions", [])
                    if a.get("platformName") == platform), None)
        if app is None or app.get("state") != "unpublished":
            raise ValueError("undelivered selection required")
        if "price" in fields:
            price = fields["price"]
            if price not in (None, "") and (
                    type(price) not in (int, float) or not 0 <= price < float("inf")):
                raise ValueError("price must be a finite nonnegative number")
            store.set_or_pop(app, "priceIfNotFree", price)
        for key in ("tiers", "galleries"):
            if key in fields:
                value = fields[key]
                if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
                    raise ValueError(f"{key} must be a list of names")
                store.set_or_pop(app, key, value)

    store.mutate_publications(json_path, mutation, config)


def mark_state(json_path: str, target_uuid: str, state: str, *,
               platform: str = "deviantart", config: dict | None = None) -> None:
    """Persist one delivery result and own publication completion state."""
    cfg = store._config(config)

    def mutation(data):
        for pub in data:
            if pub.get("uuid") != target_uuid:
                continue
            for app in pub.get("apparitions", []):
                if app.get("platformName") == platform:
                    app["state"] = state
                    break
            if state == APPARITION_STATE_PUBLISHED and pub.get("apparitions"):
                pub["state"] = (PUBLICATION_STATE_APPEARED if all(
                    app.get("state") == APPARITION_STATE_PUBLISHED
                    for app in pub["apparitions"]) else PUBLICATION_STATE_DESCRIBED)
            break

    store.mutate_publications(json_path, mutation, cfg)
