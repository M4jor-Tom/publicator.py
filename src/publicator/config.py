import hashlib
import json
import re
import tomllib
from pathlib import Path

from jsonschema import validate as _js_validate

SCHEMA = Path(__file__).resolve().parent / "publicationsSchema.json"
_SCHEMA = json.loads(SCHEMA.read_text())  # static bundled asset; load once, not per call


def _load_prompts(cfg: dict) -> dict | None:
    """[prompts]: the ART repo declares its own image-filename grammar, so
    publicator stays agnostic of it (ADR 0003). We depend on ROLES — a required
    `version` capture (which exact prompt text) and an optional `lineage` one
    (which logical prompt file) — never on the shape that carries them.
    Absent section -> feature off. Validated here, at the trust boundary: a bad
    pattern found while rendering a gallery card is far harder to diagnose."""
    p = cfg.get("prompts")
    if not p:
        return None
    if not p.get("repo"):
        raise ValueError("publicator.toml [prompts]: missing 'repo'")
    if not p.get("filename"):
        raise ValueError("publicator.toml [prompts]: missing 'filename'")
    try:
        pattern = re.compile(p["filename"])
    except re.error as e:
        raise ValueError(f"publicator.toml [prompts].filename: {e}") from e
    if "version" not in pattern.groupindex:
        raise ValueError("publicator.toml [prompts].filename: "
                         "missing required (?P<version>...) capture group")
    algo = p.get("version_hash")
    if algo not in hashlib.algorithms_available:
        raise ValueError(f"publicator.toml [prompts].version_hash: "
                         f"unknown digest {algo!r}")
    return {"repo": p["repo"], "pattern": pattern, "version_hash": algo}


def load_config(cwd="."):
    """publicator.toml config (read from <cwd>). Missing file -> empty defaults.
    Returns tiers/galleries (DA allow-lists), publicable (candidate dirs),
    tags (DA tag-list path), schedule (cadence for the gallery client),
    prompts (image-filename grammar + prompt repo, see ADR 0003)."""
    f = Path(cwd) / "publicator.toml"
    cfg = tomllib.loads(f.read_text()) if f.exists() else {}
    da = cfg.get("deviantart", {})
    return {
        "tiers": list(da.get("tiers", [])),
        "galleries": list(da.get("galleries", [])),
        "publicable": list(cfg.get("publicable", [])),
        "tags": cfg.get("tags"),
        "schedule": dict(cfg.get("schedule", {})),
        "prompts": _load_prompts(cfg),
    }


def validate_publications(data, config):
    """JSON-Schema validate, then enforce DA apparition tier/gallery values
    against config (the schema already forbids these fields on non-DA)."""
    _js_validate(instance=data, schema=_SCHEMA)
    tiers, galleries = set(config.get("tiers", [])), set(config.get("galleries", []))
    for p in data:
        for a in p.get("apparitions", []):
            if a.get("platformName") != "deviantart":
                continue
            if "tier" in a and a["tier"] not in tiers:
                raise ValueError(f"{p.get('uuid')}: tier {a['tier']!r} not in publicator.toml")
            for g in a.get("galleries", []):
                if g not in galleries:
                    raise ValueError(f"{p.get('uuid')}: gallery {g!r} not in publicator.toml")
