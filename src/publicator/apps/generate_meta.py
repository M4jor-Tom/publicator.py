#!/usr/bin/env python3
"""CLI: AI title/description for image files, without the gallery UI.

Same generator the `/ai` endpoint calls — this is just an entrypoint onto
`llm_meta.generate_metadata`, so the two can't drift. Needs no data dir, no
publications.json and no browser; `--openrouter` needs $OPENROUTER_KEY.
"""
import argparse
import json
import sys

from publicator import setup_logging
from publicator.llm_meta import DEFAULT_MODEL, OPENROUTER_MODEL, generate_metadata


def main() -> int:
    ap = argparse.ArgumentParser(description="Generate an AI title + description for an image.")
    ap.add_argument("images", nargs="+", metavar="IMAGE")
    which = ap.add_mutually_exclusive_group()  # --openrouter must not silently win over --model
    which.add_argument("--model", default=DEFAULT_MODEL, help=f"llm model id (default: {DEFAULT_MODEL})")
    which.add_argument("--openrouter", action="store_true",
                       help=f"shorthand for --model {OPENROUTER_MODEL} (needs $OPENROUTER_KEY)")
    ap.add_argument("--timeout", type=int, default=300, help="seconds per image (default: 300)")
    ap.add_argument("--json", action="store_true", dest="as_json",
                    help="emit a JSON array instead of text")
    ap.add_argument("-v", "--verbose", action="store_true", help="debug logging (the llm calls)")
    args = ap.parse_args()

    setup_logging(args.verbose)
    model = OPENROUTER_MODEL if args.openrouter else args.model

    out, failed = [], 0
    for image in args.images:
        try:
            title, description = generate_metadata(image, model, timeout=args.timeout)
        except RuntimeError as e:
            # One bad image shouldn't sink a batch; report it and keep going.
            print(f"{image}: {e}", file=sys.stderr)
            failed += 1
            continue
        out.append({"path": image, "title": title, "description": description})
        if not args.as_json:
            if len(args.images) > 1:
                print(f"--- {image}")
            print(f"Title: {title}\n\n{description}\n")

    if args.as_json:
        print(json.dumps(out, indent=2, ensure_ascii=False))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
