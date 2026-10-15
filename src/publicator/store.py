"""publications.json writes: pick, phase transitions, patch, and atomic replace."""

import contextlib
import fcntl
import json
import os
import tempfile
import threading
from pathlib import Path

from publicator.config import load_config, validate_publications
from publicator.entries import PUBLICATION_STATE_PICKED, has_delivery_success
from publicator.llm_meta import TODO_GENERATE

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


def pick_publication(path: str, json_path: str, config: dict | None = None) -> str:
    from publicator.workflow import pick_publication as operation
    return operation(path, json_path, config)


def edit_publication_date(json_path: str, uuid_: str, fields: dict, config: dict | None = None) -> None:
    from publicator.workflow import edit_publication_date as operation
    return operation(json_path, uuid_, fields, config)


def approve_publication_date(json_path: str, uuid_: str, config: dict | None = None) -> None:
    from publicator.workflow import approve_publication_date as operation
    return operation(json_path, uuid_, config)


def save_publication_metadata(json_path: str, uuid_: str, *, title: str | None = None,
                              description: str | None = None,
                              config: dict | None = None) -> None:
    from publicator.workflow import save_publication_metadata as operation
    return operation(json_path, uuid_, title=title, description=description, config=config)


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
    from publicator.workflow import approve_publication_metadata as operation
    return operation(json_path, uuid_, config)


def return_to_metadata(json_path: str, uuid_: str, config: dict | None = None) -> None:
    from publicator.workflow import return_to_metadata as operation
    return operation(json_path, uuid_, config)


def return_to_scheduling(json_path: str, uuid_: str, config: dict | None = None) -> None:
    from publicator.workflow import return_to_scheduling as operation
    return operation(json_path, uuid_, config)


def mutate_publications(json_path: str, mutation, config: dict | None = None):
    """Apply one validated atomic mutation to the latest persisted publications."""
    with _mutation_lock(json_path):
        data = _read_publications(json_path)
        result = mutation(data)
        _write_publications(json_path, data, config)
        return result


def load_publications(json_path: str) -> list[dict]:
    return _read_publications(json_path)


def mark_state(json_path: str, target_uuid: str, state: str, *,
               platform: str = "deviantart", config: dict | None = None) -> None:
    from publicator.workflow import mark_state as operation
    return operation(json_path, target_uuid, state, platform=platform, config=config)


def set_or_pop(d, k, v):
    if v in (None, [], ""):
        d.pop(k, None)
    else:
        d[k] = v


def repack_publications(json_path: str, config: dict | None = None) -> None:
    from publicator.workflow import repack_publications as operation
    return operation(json_path, config)


def select_publication_platforms(json_path: str, *, selected: bool,
                                 registered_platforms: list[str], uuid_: str | None = None,
                                 platform: str | None = None, config: dict | None = None) -> None:
    from publicator.workflow import select_publication_platforms as operation
    return operation(json_path, selected=selected, registered_platforms=registered_platforms,
                     uuid_=uuid_, platform=platform, config=config)


def save_platform_options(json_path: str, uuid_: str, platform: str, fields: dict,
                          config: dict | None = None) -> None:
    from publicator.workflow import save_platform_options as operation
    return operation(json_path, uuid_, platform, fields, config)
