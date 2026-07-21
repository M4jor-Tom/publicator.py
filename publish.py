import json
from jsonschema import validate
import time
import argparse
import os
import hashlib
import uuid
import sys
from pathlib import Path

parser = argparse.ArgumentParser(description="Add a publication with multiple files")
parser.add_argument("paths", nargs="+", type=str, help="Paths to files to include")
args = parser.parse_args()

paths: list[str] = args.paths

schema_file: str = str(Path(__file__).resolve().parent / "publicationsSchema.json")
data_file: str = "publications.json"  # CWD-relative: the Art data dir

with open(schema_file, "r", encoding="utf-8") as s, open(data_file, "r", encoding="utf-8") as o:
    validate(instance=json.load(o), schema=json.load(s))

print("Valid before insertion")

with open(data_file, "r", encoding="utf-8") as f:
    data = json.load(f)

# Build checksum index from existing publications
existing_checksums: dict[str, list[dict]] = {}

for pub in data:
    for f in pub.get("files", []):
        checksum = f.get("fileSha512sum")
        if checksum:
            existing_checksums.setdefault(checksum, []).append({
                "uuid": pub.get("uuid"),
                "basename": f.get("basename")
            })

files = []
duplicates = []

for path in paths:
    if not os.path.isfile(path):
        raise FileNotFoundError(f"Not a file: {path}")

    sha512 = hashlib.sha512()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            sha512.update(chunk)

    checksum = sha512.hexdigest()

    if checksum in existing_checksums:
        duplicates.append({
            "path": path,
            "checksum": checksum,
            "existing": existing_checksums[checksum]
        })

    files.append({
        "basename": os.path.basename(path),
        "fileSha512sum": checksum
    })

if duplicates:
    print("\n⚠️  Duplicate content detected:\n")
    for d in duplicates:
        print(f"- {d['path']}")
        for e in d["existing"]:
            print(f"    already in publication {e['uuid']} as {e['basename']}")
    print("\nAborting insertion.")
    sys.exit(1)

new_item = {
    "uuid": str(uuid.uuid4()),
    "submissionTimestamp": int(time.time()),
    "description": "",
    "files": files,
    "apparitions": []
}

print("Reading existing JSON.")
with open(data_file, "r", encoding="utf-8") as f:
    data = json.load(f)

print("Adding new publication.")
data.append(new_item)

print("Writing back to file.")
with open(data_file, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=4)

with open(schema_file, "r", encoding="utf-8") as s, open(data_file, "r", encoding="utf-8") as o:
    validate(instance=json.load(o), schema=json.load(s))

print("Valid after insertion")
