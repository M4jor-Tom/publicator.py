#!/usr/bin/env python3
"""Generate metadata for marked publications in a data directory."""
import argparse
import json
import sys
from pathlib import Path

from publicator import setup_logging
from publicator import entries
from publicator.config import load_config
from publicator.images import thumb_for_ai
from publicator.llm_meta import (
    DEFAULT_TIMEOUT, OPENROUTER_MODEL, TODO_GENERATE, VISION_MODELS,
    generate_metadata, local_model_status,
)
from publicator import workflow


def _load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def generate_pending(json_path, data_dir, *, source, model_option, timeout=DEFAULT_TIMEOUT, yes=False) -> tuple[int, int]:
    """Generate every publication containing an exact metadata marker."""
    entries.set_data_dir(data_dir)
    publications = _load(json_path)
    pending = [p for p in publications
               if p.get("title") == TODO_GENERATE or p.get("description") == TODO_GENERATE]
    if not pending:
        return 0, 0

    install_missing = False
    if source == "local":
        status = local_model_status(model_option)
        if not status["fit"]:
            print(f"Local model does not fit: {status['reason']}", file=sys.stderr)
            return 0, len(pending)
        if not status["available"]:
            print(f"Ollama is unavailable: {status['reason']}", file=sys.stderr)
            return 0, len(pending)
        if not status["installed"]:
            if not yes:
                if not sys.stdin.isatty():
                    print("Local model installation requires a TTY; pass -y to accept", file=sys.stderr)
                    return 0, len(pending)
                try:
                    answer = input(f"Install {model_option['ollama_name']}? [Y/n] ")
                except EOFError:
                    answer = "n"
                if answer.strip().lower() == "n":
                    print("Local model installation declined", file=sys.stderr)
                    return 0, len(pending)
            install_missing = True

    config = load_config(data_dir)
    cache_dir = str(Path(data_dir) / ".thumbs")
    succeeded = failed = 0
    for pub in pending:
        uuid = pub.get("uuid", "<unknown>")
        art_path = "<unresolved>"
        try:
            art_path = str(entries.find_art_path(pub["files"][0]["basename"]))
            image = thumb_for_ai(art_path, cache_dir)
            title, description = generate_metadata(
                image, model_option, source=source, timeout=timeout,
                install_missing=install_missing,
            )
            workflow.merge_generated_metadata(json_path, pub["uuid"], title=title,
                                              description=description, config=config)
            succeeded += 1
            install_missing = False
        except (Exception, SystemExit) as e:
            print(f"{uuid} ({art_path}): {e}", file=sys.stderr)
            failed += 1
    print(f"Metadata generation: {succeeded} succeeded, {failed} failed")
    return succeeded, failed


def parse_args(argv=None):
    ap = argparse.ArgumentParser(description="Generate metadata for marked publications.")
    ap.add_argument("--data-dir", default=".", help="publication database directory (default: CWD)")
    ap.add_argument("--source", required=True, choices=("local", "openrouter", "claude"))
    ap.add_argument("--model", help="provider model name (required for local)")
    ap.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT,
                    help=f"seconds per image (default: {DEFAULT_TIMEOUT})")
    ap.add_argument("-y", action="store_true", help="accept installation of a missing local model")
    ap.add_argument("-v", "--verbose", action="store_true", help="debug logging")
    args = ap.parse_args(argv)
    if args.source == "local" and not args.model:
        ap.error("--model is required for --source local")
    if args.source == "claude":
        if args.model:
            ap.error("--model is not used with --source claude")
        args.model_option = None
    else:
        name_field = "ollama_name" if args.source == "local" else "openrouter_name"
        default = None if args.source == "local" else OPENROUTER_MODEL
        requested = args.model or default
        args.model_option = next((m for m in VISION_MODELS if m[name_field] == requested), None)
        if args.model_option is None:
            ap.error(f"unsupported {args.source} model: {requested}")
    return args


def main_args(argv=None) -> int:
    args = parse_args(argv)
    setup_logging(args.verbose)
    try:
        _, failed = generate_pending(
            str(Path(args.data_dir) / "publications.json"), args.data_dir,
            source=args.source, model_option=args.model_option,
            timeout=args.timeout, yes=args.y,
        )
    except (OSError, ValueError) as e:
        print(f"generate-meta: {e}", file=sys.stderr)
        return 1
    return 1 if failed else 0


def main() -> int:
    return main_args()


if __name__ == "__main__":
    sys.exit(main())
