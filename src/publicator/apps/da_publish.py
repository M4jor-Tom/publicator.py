#!/usr/bin/env python3
"""CLI: publish publications.json entries to DeviantArt via Playwright —
the first state=unpublished one by default, one by --uuid, or --all of them."""
import argparse
import sys

from publicator import setup_logging
from publicator import deviantart
from publicator.deviantart import check_steps, configure, load_pending_entries, publish_batch


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Publish publications.json entries to DeviantArt via Playwright.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir (publications.json + .deviantart-session); default: CWD")
    ap.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    which = ap.add_mutually_exclusive_group()
    which.add_argument("--uuid", default=None,
                       help="entry to publish; default: first state=unpublished")
    which.add_argument("--all", action="store_true",
                       help="publish every state=unpublished entry in one browser session")
    ap.add_argument("--headless", action="store_true",
                    help="run Firefox headless for unattended use "
                         "(headed by default: headless is a bot-detection signal)")
    ap.add_argument("--check-steps", action="store_true",
                    help="verify STEPS mirror the skill's steps, then exit")
    ap.add_argument("-v", "--verbose", action="store_true", help="debug logging (steps, session, llm)")
    a = ap.parse_args()

    setup_logging(a.verbose)

    if a.check_steps:
        return 0 if check_steps() else 1

    configure(a.data_dir)
    json_path = a.json or str(deviantart.DATA_DIR / "publications.json")
    entries = load_pending_entries(json_path)
    if a.uuid:
        entries = [e for e in entries if e["uuid"] == a.uuid]
        if not entries:
            print(f"no state=unpublished entry with uuid {a.uuid}", file=sys.stderr)
            return 1
    if not entries:
        print("nothing to publish (no state=unpublished entry)")
        return 0
    if not a.all:
        entries = entries[:1]

    for entry in entries:
        print(f"publishing: {entry['title']}  [{entry['uuid']}]")
    published, failed, err = publish_batch(
        entries, [e["uuid"] for e in entries], json_path, headless=a.headless)
    print(f"published={published} failed={failed}")
    if err:
        print(f"error: {err}", file=sys.stderr)
    return 0 if published and not failed else 1


if __name__ == "__main__":
    sys.exit(main())
