"""Automated DeviantArt publication.

Reads the first state=unpublished publication from publications.json, opens
Playwright Firefox against a scratch copy of the `privacy` profile, runs the
publish-deviantart skill end-to-end, and flips the publication's state to
`published_or_scheduled` on success. Exits 0 on success, non-zero on any failure
— caller can then fall back to the manual steps in
docs/superpowers/skills/publish-deviantart/steps.md.
"""
import re
import shutil
import sys
from datetime import datetime
from pathlib import Path

from playwright.sync_api import sync_playwright

from echo_first_unpublished_publication_data import (
    deviantart_apparition,
    find_art_path,
    first_unpublished,
    format_schedule,
    mark_published_or_scheduled,
)

PKG = Path(__file__).resolve().parent  # code assets (tags) travel with the package
PROFILE_SRC = Path.home() / ".mozilla/firefox/privacy"
PROFILE_DST = Path("/tmp/pw-privacy")
TAGS_FILE = PKG / "tags/da.txt"


def prepare_profile() -> None:
    if PROFILE_DST.exists():
        shutil.rmtree(PROFILE_DST)
    shutil.copytree(PROFILE_SRC, PROFILE_DST, symlinks=True)
    for name in ("lock", ".parentlock", "parent.lock", "compatibility.ini"):
        (PROFILE_DST / name).unlink(missing_ok=True)


def read_pub() -> dict:
    pub = first_unpublished()
    app = deviantart_apparition(pub)
    ts = app.get("apparitionTimestampIfDifferentThanSubmission")
    if ts is None:
        raise RuntimeError(f"no schedule timestamp on {pub['uuid']}")
    art = find_art_path(pub["files"][0]["basename"])
    if art.suffix == ".webp":
        raise RuntimeError(f"refusing webp for DA: {art}")
    return {
        "uuid": pub["uuid"],
        "art": str(art),
        "title": app["urlElsePublicationName"],
        "schedule": format_schedule(ts),
    }


def set_checkbox(page, name: str, want: bool = True) -> None:
    """DA renders checkboxes as hidden <input>; toggle via JS until state matches.
    The mature click doesn't always stick on first attempt (racy DOM), so verify."""
    for _ in range(3):
        checked = page.evaluate(
            "n => document.querySelector(`input[name=\"${n}\"]`).checked", name
        )
        if checked == want:
            return
        page.evaluate(
            "n => document.querySelector(`input[name=\"${n}\"]`).click()", name
        )
        page.wait_for_timeout(300)
    raise RuntimeError(f"checkbox {name} would not stay {want}")


def type_tags(page) -> None:
    tags = [t.strip() for t in TAGS_FILE.read_text().splitlines() if t.strip()]
    tag_input = page.locator('input[aria-errormessage$="-error"]').last
    tag_input.click()
    for t in tags:
        tag_input.type(t, delay=10)
        page.keyboard.press("Enter")
        page.wait_for_timeout(120)


def open_schedule_menu(page) -> None:
    caret = page.locator('button[aria-haspopup="menu"]').last
    caret.scroll_into_view_if_needed()
    caret.click()
    page.wait_for_timeout(800)
    page.locator('[role="menuitem"][label="Schedule"]').first.click(force=True)


# schedule string from `date`: "Tue Jul 28 08:00:00 PM CEST 2026"
_MONTHS = {m: i + 1 for i, m in enumerate(
    ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"])}


def parse_schedule(s: str) -> tuple[int, int, int, int]:
    """Return (year, month, day, hour24) from `date`-style string."""
    m = re.match(r"\w+\s+(\w+)\s+(\d+)\s+(\d+):\d+:\d+\s+(AM|PM)\s+\w+\s+(\d+)", s.strip())
    if not m:
        raise ValueError(f"unparseable schedule: {s!r}")
    mon, day, hr, ampm, year = m.group(1), int(m.group(2)), int(m.group(3)), m.group(4), int(m.group(5))
    if ampm == "PM" and hr != 12: hr += 12
    if ampm == "AM" and hr == 12: hr = 0
    return year, _MONTHS[mon], day, hr


def pick_schedule(page, schedule: str) -> None:
    year, month, day, hour = parse_schedule(schedule)
    open_schedule_menu(page)
    # date picker
    page.locator('#schedule-draft-date-picker [role="button"]').click()
    page.wait_for_timeout(500)
    # Advance months until the target day button shows. Calendar opens on
    # current month; only forward navigation needed (scheduling is future-only).
    day_label = f'{datetime(year, month, day):%B} {day}'
    # ponytail: 24 hops = 2y ceiling. Bump if you ever schedule further out.
    for _ in range(24):
        if page.locator(f'button[aria-label*="{day_label}"]').count():
            break
        page.locator('button[aria-label="Go to the Next Month"]').click()
        page.wait_for_timeout(200)
    page.locator(f'button[aria-label*="{day_label}"]').first.click()
    # time picker is a native <select>, 24h values 0..23
    page.select_option('#schedule-draft-time-picker', value=str(hour))
    page.wait_for_timeout(300)
    page.get_by_text("Confirm Schedule", exact=True).click()
    page.wait_for_timeout(1500)


def run(pub: dict) -> None:
    with sync_playwright() as p:
        ctx = p.firefox.launch_persistent_context(str(PROFILE_DST), headless=False)
        page = ctx.pages[0] if ctx.pages else ctx.new_page()

        # 1-2 open submit modal
        page.goto("https://www.deviantart.com", wait_until="domcontentloaded", timeout=45000)
        page.get_by_role("link", name="Submit").first.click()
        page.wait_for_load_state("domcontentloaded", timeout=30000)

        # 3 upload art
        with page.expect_file_chooser(timeout=15000) as fc:
            page.get_by_text("Upload Your Art", exact=True).click()
        fc.value.set_files(pub["art"])

        # 4 title
        page.get_by_label("Title", exact=False).first.fill(pub["title"])

        # 5 mature + AI checkboxes (verify — mature is racy on first click)
        set_checkbox(page, "matureContent", True)
        set_checkbox(page, "isAiGenerated", True)

        # 6 tags — typed rather than pasted; clipboard round-trip via xclip proved flaky
        type_tags(page)

        # 7 schedule
        pick_schedule(page, pub["schedule"])

        # Re-verify mature — the Confirm Schedule roundtrip sometimes drops it
        set_checkbox(page, "matureContent", True)

        # 8 final submit — the green "Schedule" button at bottom-right
        page.get_by_text("Schedule", exact=True).last.click()
        page.wait_for_timeout(5000)
        ctx.close()


def publish_one() -> int:
    prepare_profile()
    pub = read_pub()
    try:
        run(pub)
    except Exception as e:
        print(f"AUTOMATION FAILED: {e}", file=sys.stderr)
        return 1
    mark_published_or_scheduled(pub["uuid"])
    print("published:", pub["title"])
    return 0


def main() -> int:
    if "--loop" in sys.argv[1:]:
        while True:
            try:
                first_unpublished()
            except SystemExit:
                print("no more unpublished; done")
                return 0
            rc = publish_one()
            if rc != 0:
                return rc
    return publish_one()


if __name__ == "__main__":
    sys.exit(main())
