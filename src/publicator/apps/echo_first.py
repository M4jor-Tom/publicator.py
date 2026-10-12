#!/usr/bin/env python3
"""CLI: print path/title/schedule for the first state=unpublished publication."""
import argparse
import sys

from publicator.entries import (
    effective_schedule_ts,
    find_art_path,
    first_unpublished,
    format_schedule,
    set_data_dir,
)


def main() -> int:
    ap = argparse.ArgumentParser(description="Print the first unpublished publication's data.")
    ap.add_argument("--data-dir", default=None,
                    help="publication database dir (publications.json + images); default: CWD")
    set_data_dir(ap.parse_args().data_dir)
    pub = first_unpublished("deviantart")
    ts = effective_schedule_ts(pub)
    if ts is None:
        raise SystemExit(f"no apparitionTimestampIfDifferentThanSubmission on {pub['uuid']}")
    print(f"path: {find_art_path(pub['files'][0]['basename'])}")
    print(f"title: {pub['title']}")
    print(f"schedule: {format_schedule(ts)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
