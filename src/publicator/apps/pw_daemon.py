"""Playwright daemon. Reads Python command chunks from FIFO, execs with `page`,
`context`, `p` in scope. Writes screenshot + log after each command.

Chromium persistent context in the publication database dir (--data-dir, default
CWD) `.deviantart-session/`, so it shares the logged-in DeviantArt session with
the publish-next app.

# ponytail: single-page, single-context daemon. Add tab tracking if the
# publication flow starts spawning multiple tabs we care about.
"""
import argparse
import io
import os
import sys
import traceback
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path

from playwright.sync_api import sync_playwright

FIFO = "/tmp/pw.cmd"
LOG = "/tmp/pw.log"
SHOT = "/tmp/pw.png"


def log(msg: str) -> None:
    with open(LOG, "a") as f:
        f.write(str(msg).rstrip() + "\n")


def main() -> None:
    _ap = argparse.ArgumentParser(description="Playwright daemon for manual DA publishing.")
    _ap.add_argument("--data-dir", default=None,
                     help="publication database dir holding .deviantart-session/; default: CWD")
    _dd = _ap.parse_args().data_dir
    SESSION = (Path(_dd).resolve() if _dd else Path.cwd()) / ".deviantart-session"  # shared with the publish-next app

    if not os.path.exists(FIFO):
        os.mkfifo(FIFO)
    open(LOG, "w").close()
    SESSION.mkdir(parents=True, exist_ok=True)

    with sync_playwright() as p:
        context = p.chromium.launch_persistent_context(str(SESSION), headless=False)
        page = context.pages[0] if context.pages else context.new_page()
        log(f"READY pid={os.getpid()}")
        seq = 0
        scope = {"p": p, "context": context, "page": page}
        while True:
            with open(FIFO, "r") as f:
                cmd = f.read()
            if not cmd.strip():
                continue
            seq += 1
            log(f"--- CMD {seq} ---")
            log(cmd)
            buf = io.StringIO()
            try:
                with redirect_stdout(buf), redirect_stderr(buf):
                    exec(cmd, scope)
                out = buf.getvalue()
                if out:
                    log(out)
                log(f"--- OK {seq} ---")
            except Exception:
                out = buf.getvalue()
                if out:
                    log(out)
                log(traceback.format_exc())
                log(f"--- ERR {seq} ---")
            # refresh page ref in case command re-bound it (new tab, etc.)
            page = scope.get("page", page)
            try:
                page.screenshot(path=SHOT, full_page=False)
            except Exception as e:
                log(f"screenshot err: {e}")


if __name__ == "__main__":
    sys.exit(main())
