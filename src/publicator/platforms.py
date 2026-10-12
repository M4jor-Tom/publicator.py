"""Executable publication platforms.

Registration is deliberately tiny: historical data may name platforms that have
no publisher here, but only registered platforms can be dispatched.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from types import ModuleType

from publicator import deviantart
from publicator.config import load_config, validate_publications
from publicator.store import load_publications


@dataclass(frozen=True)
class Platform:
    name: str
    label: str
    module: ModuleType | object
    skill: Path


PLATFORMS: dict[str, Platform] = {
    "deviantart": Platform(
        name="deviantart",
        label="DeviantArt",
        module=deviantart,
        skill=deviantart.SKILL_MD,
    ),
}


def _check_platform(platform: Platform) -> None:
    missing = [name for name in ("configure", "load_pending_entries", "publish_batch")
               if not callable(getattr(platform.module, name, None))]
    if missing:
        raise ValueError(f"platform {platform.name!r} missing publisher functions: {', '.join(missing)}")
    if not platform.skill.exists():
        raise ValueError(f"platform {platform.name!r} missing skill: {platform.skill}")


def _selected_names(data: list[dict], uuids: set[str] | None, names: set[str] | None) -> list[str]:
    seen: list[str] = []
    for pub in data:
        if pub.get("state") != "DESCRIBED":
            continue
        if uuids is not None and pub.get("uuid") not in uuids:
            continue
        for app in pub.get("apparitions", []):
            name = app.get("platformName")
            if app.get("state") != "unpublished" or not name:
                continue
            if names is not None and name not in names:
                continue
            if name not in seen:
                seen.append(name)
    return seen


def publish_selected(json_path: str, data_dir: str | None = None, *,
                     platform_names: list[str] | None = None,
                     uuids: list[str] | None = None,
                     headless: bool = False,
                     config: dict | None = None) -> tuple[int, int, str | None]:
    """Publish stored DESCRIBED/unpublished targets through registered modules.

    Returns ``(published, failed, err)``. Successful targets are marked by their
    platform module and disappear from later module loads, so retries only see
    still-unpublished apparitions.
    """
    requested = set(platform_names) if platform_names is not None else None
    unknown = sorted((requested or set()) - set(PLATFORMS))
    if unknown:
        raise ValueError(f"unregistered platform: {', '.join(unknown)}")
    config = config if config is not None else load_config(data_dir or Path.cwd())
    data = load_publications(json_path)
    validate_publications(data, config)
    selected = _selected_names(data, set(uuids) if uuids is not None else None, requested)
    for name in selected:
        if name not in PLATFORMS:
            raise ValueError(f"unregistered platform: {name}")

    published = failed = 0
    err: str | None = None
    for name in selected:
        platform = PLATFORMS[name]
        _check_platform(platform)
        module = platform.module
        module.configure(data_dir, config)
        entries = module.load_pending_entries(json_path)
        if uuids is not None:
            allowed = set(uuids)
            entries = [e for e in entries if e.get("uuid") in allowed]
        if not entries:
            continue
        ok, bad, msg = module.publish_batch(
            entries, [e["uuid"] for e in entries], json_path, headless=headless)
        published += ok
        failed += bad
        err = err or msg
    return published, failed, err
