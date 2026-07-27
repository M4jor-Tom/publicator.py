import argparse
import json
from pathlib import Path
from jsonschema import validate

ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
dd = ap.parse_args().data_dir
data_dir = Path(dd).resolve() if dd else Path.cwd()

schema = Path(__file__).resolve().parent / "publicationsSchema.json"
with open(schema) as s, open(data_dir / "publications.json") as o:
    validate(instance=json.load(o), schema=json.load(s))

print("Valid")
