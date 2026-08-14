#!/usr/bin/env python3
"""CLI: validate publications.json against the bundled schema."""
import argparse
import json
import sys
from pathlib import Path

from publicator.config import load_config, validate_publications


def main() -> int:
    ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
    ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
    args = ap.parse_args()
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    data = json.loads((data_dir / "publications.json").read_text())
    validate_publications(data, load_config(data_dir))
    print("Valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())
