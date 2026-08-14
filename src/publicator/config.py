import json
import tomllib
from pathlib import Path

from jsonschema import validate as _js_validate

SCHEMA = Path(__file__).resolve().parent / "publicationsSchema.json"
_SCHEMA = json.loads(SCHEMA.read_text())  # static bundled asset; load once, not per call


def load_config(cwd="."):
    """publicator.toml config (read from <cwd>). Missing file -> empty defaults.
    Returns tiers/galleries (DA allow-lists), publicable (candidate dirs),
    tags (DA tag-list path), schedule (cadence for the gallery client)."""
    f = Path(cwd) / "publicator.toml"
    cfg = tomllib.loads(f.read_text()) if f.exists() else {}
    da = cfg.get("deviantart", {})
    return {
        "tiers": list(da.get("tiers", [])),
        "galleries": list(da.get("galleries", [])),
        "publicable": list(cfg.get("publicable", [])),
        "tags": cfg.get("tags"),
        "schedule": dict(cfg.get("schedule", {})),
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
