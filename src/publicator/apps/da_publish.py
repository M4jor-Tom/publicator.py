#!/usr/bin/env python3
"""CLI: publish ONE publications.json entry to DeviantArt via Playwright."""
import argparse
import sys

from publicator import setup_logging
from publicator import deviantart
from publicator.deviantart import check_steps, configure, load_pending_entries, publish_batch


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Publish ONE publications.json entry to DeviantArt via Playwright.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir (publications.json + .deviantart-session); default: CWD")
    ap.add_argument("--json", default=None, help="default: <data-dir>/publications.json")
    ap.add_argument("--uuid", default=None,
                    help="entry to publish; default: first state=unpublished")
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

    entry = entries[0]
    print(f"publishing: {entry['title']}  [{entry['uuid']}]")
    published, failed, err = publish_batch([entry], [entry["uuid"]], json_path)
    print(f"published={published} failed={failed}")
    if err:
        print(f"error: {err}", file=sys.stderr)
    return 0 if published and not failed else 1


if __name__ == "__main__":
    sys.exit(main())
