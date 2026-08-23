#!/usr/bin/env python3
"""CLI: AI title/description for image files, without the gallery UI.

An entrypoint onto `llm_meta.generate_metadata` (via the same `thumb_for_ai`
downscale) and nothing else, so it and the gallery's `/ai` endpoint can't drift.
No data dir, no publications.json, no browser; `--openrouter` needs $OPENROUTER_KEY.
"""
import argparse
import json
import sys
import tempfile

from publicator import setup_logging
from publicator.images import thumb_for_ai
from publicator.llm_meta import DEFAULT_MODEL, DEFAULT_TIMEOUT, OPENROUTER_MODEL, generate_metadata


def main() -> int:
    ap = argparse.ArgumentParser(description="Generate an AI title + description for an image.")
    ap.add_argument("images", nargs="+", metavar="IMAGE")
    which = ap.add_mutually_exclusive_group()
    which.add_argument("--model", default=DEFAULT_MODEL, help=f"llm model id (default: {DEFAULT_MODEL})")
    which.add_argument("--openrouter", dest="model", action="store_const", const=OPENROUTER_MODEL,
                       help=f"shorthand for --model {OPENROUTER_MODEL} (needs $OPENROUTER_KEY)")
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT,
                    help=f"seconds per image (default: {DEFAULT_TIMEOUT})")
    ap.add_argument("--json", action="store_true", dest="as_json",
                    help="emit a JSON array instead of text")
    ap.add_argument("-v", "--verbose", action="store_true", help="debug logging (the llm calls)")
    args = ap.parse_args()

    setup_logging(args.verbose)

    done = []
    # ponytail: throwaway thumb cache — the gallery keeps a content-addressed one
    # in the data dir, but this app is defined by not having a data dir.
    with tempfile.TemporaryDirectory() as cache:
        for image in args.images:
            try:
                title, description = generate_metadata(
                    thumb_for_ai(image, cache), args.model, timeout=args.timeout)
            except RuntimeError as e:
                print(f"{image}: {e}", file=sys.stderr)  # one bad image shouldn't sink a batch
                continue
            done.append({"path": image, "title": title, "description": description})
            if not args.as_json:
                # Printed in the loop, not after it: a batch is minutes per image,
                # and JSON is the only mode that has to buffer for a valid array.
                head = f"--- {image}\n" if len(args.images) > 1 else ""
                print(f"{head}Title: {title}\n\n{description}\n", flush=True)

    if args.as_json:
        print(json.dumps(done, indent=2, ensure_ascii=False))
    return 1 if len(done) < len(args.images) else 0


if __name__ == "__main__":
    sys.exit(main())
