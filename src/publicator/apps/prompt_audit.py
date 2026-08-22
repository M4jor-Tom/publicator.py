#!/usr/bin/env python3
"""CLI: how much of the data dir's art can be traced back to its prompt.

Two of these numbers earn their place (ADRs 0003, 0004):

* `parsed` drops to 0 if the filename grammar in publicator.toml drifts from
  the one identify_image.sh actually produces — they live in two repos and
  nothing can prevent that, so it is made loud instead.
* `nearest` is the health metric for archiving. Flat is healthy. Rising means
  archive writes are silently failing at generation time. It can also fall,
  when a previously-failed prompt is committed later and its digest resolves
  retroactively.
"""
import argparse
import os
import sys
from pathlib import Path

from publicator.config import load_config
from publicator.images import collect_images
from publicator.prompts import Exact, Nearest, from_config


def audit(data_dir: str, config: dict) -> dict:
    archive = from_config(data_dir, config)
    counts = {"files": 0, "parsed": 0, "exact": 0, "nearest": 0, "unknown": 0}
    exact_versions, nearest_versions = set(), set()
    for d in config.get("publicable", []):
        for path in collect_images(os.path.join(data_dir, d)):
            counts["files"] += 1
            if archive is None:
                continue
            identity = archive.parse(path)
            if identity is None:
                continue
            counts["parsed"] += 1
            match = archive.resolve(identity, near=os.path.dirname(path))
            if isinstance(match, Exact):
                counts["exact"] += 1
                exact_versions.add(identity.version)
            elif isinstance(match, Nearest):
                counts["nearest"] += 1
                nearest_versions.add(identity.version)
            else:
                counts["unknown"] += 1
    counts["exact_versions"] = len(exact_versions)
    counts["nearest_versions"] = len(nearest_versions)
    return counts


def format_report(c: dict) -> str:
    return (
        f"parsed   : {c['parsed']} of {c['files']} files\n"
        f"exact    : {c['exact']} images / {c['exact_versions']} versions\n"
        f"nearest  : {c['nearest']} images / {c['nearest_versions']} versions\n"
        f"unknown  : {c['unknown']}")


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Report how much art resolves to its generation prompt.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir; default: CWD")
    args = ap.parse_args()
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    print(format_report(audit(str(data_dir), load_config(data_dir))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
