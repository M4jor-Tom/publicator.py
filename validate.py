import json
from pathlib import Path
from jsonschema import validate

schema = Path(__file__).resolve().parent / "publicationsSchema.json"
with open(schema) as s, open("publications.json") as o:
    validate(instance=json.load(o), schema=json.load(s))

print("Valid")
