#!/usr/bin/env python3
"""CLI: the unified publish workflow (gallery + AI metadata + Firefox batch)."""
import argparse
import logging
import sys
import tempfile
from pathlib import Path

from publicator import setup_logging
from publicator.config import load_config
from publicator.deviantart import configure, load_pending_entries
from publicator.images import find_candidates, generate_thumbnails
from publicator.llm_meta import DEFAULT_MODEL
from publicator.publish_next import serve

log = logging.getLogger("publicator.gallery")


def main() -> int:
    parser = argparse.ArgumentParser(description="Unified publish workflow (gallery + AI + Firefox DA).")
    parser.add_argument("-n", type=int, default=10)
    parser.add_argument("--data-dir", default=None,
                        help="publication database dir (publications.json + images + browser session); default: CWD")
    parser.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    parser.add_argument("--ai-model", default=DEFAULT_MODEL)
    parser.add_argument("--openrouter-model",
                        # ponytail: free :free ids churn on OpenRouter; this is the current
                        # free model with both vision and structured_outputs. Override via flag.
                        default="openrouter/google/gemma-4-26b-a4b-it:free",
                        help="free vision model for the OpenRouter option; needs $OPENROUTER_KEY")
    parser.add_argument("--ai-timeout", type=int, default=300,
                        help="seconds to wait for an AI title/description (default: 300)")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="debug logging (HTTP requests, AI/llm calls, publish steps)")
    args = parser.parse_args()

    setup_logging(args.verbose)

    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    config = load_config(data_dir)
    configure(args.data_dir, config)  # points deviantart + entry helpers at the db dir
    args.json = args.json or str(data_dir / "publications.json")
    publicable_dirs = [str(data_dir / d) for d in config["publicable"]]

    print("Finding unpublished images...")
    candidates = find_candidates(publicable_dirs, args.json, args.n)
    pending = load_pending_entries(args.json)
    log.debug("found %d candidate(s), %d pending", len(candidates), len(pending))
    if not candidates and not pending:
        print("Nothing to publish (no new picks, no pending queue).")
        return 0

    msg = f"{len(candidates)} new pick(s)"
    if pending:
        msg += f", {len(pending)} already queued"
    print(f"Found {msg}. Generating thumbnails...")
    with tempfile.TemporaryDirectory(prefix="publish-next-") as thumb_dir:
        thumb_map = generate_thumbnails(candidates + [e["path"] for e in pending], thumb_dir)
        log.debug("generated %d thumbnail(s) in %s", len(thumb_map), thumb_dir)
        result = serve(thumb_dir, thumb_map, candidates, pending, args, config)

    if result is None:
        print("No publish action taken.")
        return 0
    print(f"Done. published={result.get('published',0)} failed={result.get('failed',0)}")
    if result.get("error"):
        print(f"error: {result['error']}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
