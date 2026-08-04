import argparse
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


def _selfcheck():
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        # missing file -> all-empty defaults
        assert load_config(d) == {
            "tiers": [], "galleries": [], "publicable": [], "tags": None, "schedule": {}
        }, load_config(d)
        (Path(d) / "publicator.toml").write_text(
            'publicable = ["picked"]\n'
            'tags = "tags/da.txt"\n'
            '[deviantart]\ntiers = ["T"]\ngalleries = ["G"]\n'
            '[schedule]\nfrequency = "weekly"\nday = "tuesday"\nhour = 20\nper_slot = 2\n'
        )
        c = load_config(d)
        assert c["tiers"] == ["T"] and c["galleries"] == ["G"], c
        assert c["publicable"] == ["picked"], c
        assert c["tags"] == "tags/da.txt", c
        assert c["schedule"] == {
            "frequency": "weekly", "day": "tuesday", "hour": 20, "per_slot": 2
        }, c
    print("validate selfcheck OK")


def main():
    ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
    ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
    ap.add_argument("--selfcheck", action="store_true", help="run offline self-checks, then exit")
    args = ap.parse_args()
    if args.selfcheck:
        _selfcheck(); return
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    data = json.loads((data_dir / "publications.json").read_text())
    validate_publications(data, load_config(data_dir))
    print("Valid")


if __name__ == "__main__":
    main()
