import argparse
import json
import tomllib
from pathlib import Path

from jsonschema import validate as _js_validate

SCHEMA = Path(__file__).resolve().parent / "publicationsSchema.json"
_SCHEMA = json.loads(SCHEMA.read_text())  # static bundled asset; load once, not per call


def load_config(cwd="."):
    """DeviantArt tier/gallery allow-lists from <cwd>/config.toml [deviantart].
    Missing file -> empty lists."""
    f = Path(cwd) / "config.toml"
    if not f.exists():
        return {"tiers": [], "galleries": []}
    da = tomllib.loads(f.read_text()).get("deviantart", {})
    return {"tiers": list(da.get("tiers", [])), "galleries": list(da.get("galleries", []))}


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
                raise ValueError(f"{p.get('uuid')}: tier {a['tier']!r} not in config.toml")
            for g in a.get("galleries", []):
                if g not in galleries:
                    raise ValueError(f"{p.get('uuid')}: gallery {g!r} not in config.toml")


def main():
    ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
    ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
    args = ap.parse_args()
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    data = json.loads((data_dir / "publications.json").read_text())
    validate_publications(data, load_config(Path.cwd()))
    print("Valid")


if __name__ == "__main__":
    main()
