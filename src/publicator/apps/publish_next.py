#!/usr/bin/env python3
"""CLI: the durable four-phase workflow and calendar."""
import argparse
import sys
from pathlib import Path

from publicator import setup_logging
from publicator.config import load_config
from publicator.images import Search, find_candidates
from publicator.llm_meta import DEFAULT_MODEL, OPENROUTER_MODEL
from publicator.webui.server import serve

def main() -> int:
    parser = argparse.ArgumentParser(description="Unified publish workflow (gallery + AI + Firefox DA).")
    parser.add_argument("-n", type=int, default=10)
    parser.add_argument("--data-dir", default=None,
                        help="publication database dir (publications.json + images + browser session); default: CWD")
    parser.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    parser.add_argument("--ai-model", default=DEFAULT_MODEL)
    parser.add_argument("--openrouter-model", default=OPENROUTER_MODEL,
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
    args.json = args.json or str(data_dir / "publications.json")
    publicable_dirs = [str(data_dir / d) for d in config["publicable"]]

    print("Finding unpublished images...")
    candidates = find_candidates(Search(args.n, tuple(publicable_dirs)), args.json)
    print(f"Found {len(candidates)} new pick(s); opening saved workflow and calendar.")
    serve(str(data_dir), candidates, args, config, publicable_dirs)
    return 0


if __name__ == "__main__":
    sys.exit(main())
