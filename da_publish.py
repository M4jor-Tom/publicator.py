#!/usr/bin/env python3
"""DeviantArt submission — Playwright logic only, extracted from publish_next.py.

Publishes ONE publications.json entry through the DeviantArt web submit flow and
flips its state to published_or_scheduled. `publish_next.py` (the gallery UI)
imports `publish_batch`/`load_pending_entries`/`configure` from here; this module
never imports back, so there is no cycle.

The ordered `STEPS` registry below mirrors, 1:1, the numbered list in the
`publish-deviantart` skill (SKILL.md). `--check-steps` asserts they stay in sync,
so a step added to the skill but not implemented here fails loudly.

DeviantArt is behind PerimeterX, which blocks every Playwright browser at the
LOGIN page. Login is therefore done out-of-band in a real Firefox (`#login`);
here we copy that pre-signed-in profile and only VERIFY the session — the submit
pages themselves are not bot-walled.
"""

import argparse
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime
from pathlib import Path

log = logging.getLogger("publicator.da")


def setup_logging(verbose: bool) -> None:
    """App-wide logging. verbose -> DEBUG (llm calls, HTTP, each publish step),
    else INFO. Shared by both entrypoints; basicConfig is a no-op after the first
    call, so whichever main() runs first wins (they use the same settings)."""
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        datefmt="%H:%M:%S",
    )

from echo_first_unpublished_publication_data import (
    deviantart_apparition,
    find_art_path,
    format_schedule,
    set_data_dir,
)
from validate import load_config

PKG = Path(__file__).resolve().parent  # code assets (SKILL.md) travel with the package
TAGS_FILE = None                       # resolved by configure() from publicator.toml [tags], under DATA_DIR
SKILL_MD = PKG / ".claude/skills/publish-deviantart/SKILL.md"

STATE_UNPUBLISHED = "unpublished"
STATE_PUBLISHED = "published_or_scheduled"

# Runtime state (publications.json, images, browser session) lives in the
# publication database dir: --data-dir, defaulting to CWD. configure() retargets.
DATA_DIR = Path.cwd()
LOGIN_DIR = DATA_DIR / ".deviantart-login"      # human sign-in profile (real Firefox, via #login)
SESSION_DIR = DATA_DIR / ".deviantart-session"  # Playwright working copy of the login profile

# schedule string from `date`: "Tue Jul 28 08:00:00 PM CEST 2026"
_MONTHS = {m: i + 1 for i, m in enumerate(
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"])}


def configure(data_dir) -> Path:
    """Point this module (and the echo helpers) at a publication database dir.
    Returns the resolved DATA_DIR."""
    global DATA_DIR, LOGIN_DIR, SESSION_DIR, TAGS_FILE
    DATA_DIR = set_data_dir(data_dir)  # also points the echo helpers (find_art_path) at it
    LOGIN_DIR = DATA_DIR / ".deviantart-login"
    SESSION_DIR = DATA_DIR / ".deviantart-session"
    tags = load_config(Path.cwd()).get("tags")
    TAGS_FILE = DATA_DIR / tags if tags else None
    return DATA_DIR


# ---------------------------------------------------------------------------
# publications.json state write
# ---------------------------------------------------------------------------

def _atomic_write_json(path: str, data) -> None:
    d = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(prefix=".pub-", dir=d)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=4)
        os.replace(tmp, path)
    except Exception:
        try: os.unlink(tmp)
        except OSError: pass
        raise


def mark_state(json_path: str, target_uuid: str, state: str) -> None:
    with open(json_path, "r", encoding="utf-8") as f:
        data = json.load(f)
    for p in data:
        if p.get("uuid") == target_uuid:
            deviantart_apparition(p)["state"] = state
            break
    _atomic_write_json(json_path, data)


# ---------------------------------------------------------------------------
# Entry resolution: publications.json -> {uuid, path, title, scheduleTs}
# ---------------------------------------------------------------------------

def _pending_art(basename: str) -> Path | None:
    """Best on-disk file for a stored basename (for upload): prefer a non-webp
    match, else any match (webp is converted at publish time)."""
    stem = basename.rsplit(".", 1)[0]
    matches = [p for p in DATA_DIR.rglob(f"*{stem}*") if p.is_file()]
    if not matches:
        return None
    non_webp = [p for p in matches if p.suffix.lower() != ".webp"]
    return (non_webp[0] if non_webp else matches[0]).resolve()


def load_pending_entries(json_path: str) -> list[dict]:
    """Existing state=unpublished DA entries -> queue dicts
    {uuid, path, title, scheduleTs}. Skips entries whose art can't be found."""
    try:
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return []
    out = []
    for p in data:
        try:
            app = deviantart_apparition(p)
            if app.get("state") != STATE_UNPUBLISHED:
                continue
            art = _pending_art(p["files"][0]["basename"])
        except (SystemExit, KeyError, IndexError) as e:
            print(f"skip pending {p.get('uuid')}: {e}", file=sys.stderr)
            continue
        if art is None:
            print(f"skip pending {p.get('uuid')}: art file not found", file=sys.stderr)
            continue
        out.append({
            "uuid": p["uuid"],
            "path": str(art),
            "title": app["urlElsePublicationName"],
            "description": p.get("description", ""),
            "scheduleTs": app.get("apparitionTimestampIfDifferentThanSubmission"),
            "price": app.get("priceIfNotFree"),
            "tier": app.get("tier"),
            "galleries": app.get("galleries", []),
        })
    return out


def _resolve_art(path: str) -> str:
    """Path to a DA-uploadable (non-webp) file. DA rejects webp, so for a webp
    input prefer an existing non-webp sibling, else convert it to a temp png."""
    p = Path(path)
    if p.suffix.lower() != ".webp":
        return str(p.resolve())
    try:
        return str(find_art_path(p.name))
    except SystemExit:
        # ponytail: no sibling on disk — convert once to /tmp; imagemagick is a dep.
        png = Path(tempfile.gettempdir()) / f"{p.stem}.png"
        subprocess.run(["convert", str(p), str(png)], check=True)
        return str(png)


# ---------------------------------------------------------------------------
# DeviantArt submit steps (mirror SKILL.md — keep STEPS in sync, see check_steps)
# ---------------------------------------------------------------------------

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


def _tag_input(page):
    return page.locator('input[aria-errormessage$="-error"]').last


def open_schedule_menu(page) -> None:
    caret = page.locator('button[aria-haspopup="menu"]').last
    caret.scroll_into_view_if_needed()
    caret.click()
    page.wait_for_timeout(800)
    page.locator('[role="menuitem"][label="Schedule"]').first.click(force=True)


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


# --- individual steps: each takes (page, entry); order/text mirror SKILL.md ---

def _step_goto(page, e):
    page.goto("https://www.deviantart.com", wait_until="domcontentloaded", timeout=45000)


def _step_submit(page, e):
    page.get_by_role("link", name="Submit").first.click()
    page.wait_for_load_state("domcontentloaded", timeout=30000)


def _step_upload(page, e):
    with page.expect_file_chooser(timeout=15000) as fc:
        page.get_by_text("Upload Your Art", exact=True).click()
    fc.value.set_files(_resolve_art(e["path"]))


def _step_title(page, e):
    page.get_by_label("Title", exact=False).first.fill(e["title"])


DESCRIPTION_SELECTOR = '[aria-label="Description"]'


def _step_description(page, e):
    page.locator(DESCRIPTION_SELECTOR).first.fill(e.get("description", ""))


def _step_checkboxes(page, e):
    set_checkbox(page, "matureContent", True)
    set_checkbox(page, "isAiGenerated", True)


def _step_clear_tags(page, e):
    """Step 6: remove any tags already in the Tags field so only tags/da.txt
    ends up on the deviation. DA caps a deviation at 30 tags, so a single
    leftover tag overflows the field once add_tags types its 30.

    A freshly-uploaded deviation usually starts empty (the greyed "Suggested
    tags" below the field are click-to-add hints, NOT filled tags), but a
    restored draft can arrive pre-tagged. Each committed tag renders as a chip
    `span[role="button"][data-tag]` whose click removes it (verified against the
    live submit form); click them until none remain.

    ponytail: 60 = DA's 30-tag cap x2 headroom. `data-tag` marks entered chips
    apart from the field's "Copy Tags" <button>; if DA reworks that markup this
    is the one place to update.
    """
    _tag_input(page).click()
    chips = page.locator('[role="combobox"] span[role="button"][data-tag]')
    for _ in range(60):
        if chips.count() == 0:
            break
        try:
            chips.first.click()
        except Exception:
            pass  # chip list re-rendered mid-iteration; loop re-locates and retries
        page.wait_for_timeout(80)


def _step_add_tags(page, e):
    """Step 7: type each tag from tags/da.txt into the Tags field."""
    if TAGS_FILE is None:
        raise RuntimeError("no 'tags' path configured in publicator.toml")
    tags = [t.strip() for t in TAGS_FILE.read_text().splitlines() if t.strip()]
    tag_input = _tag_input(page)
    tag_input.click()
    for t in tags:
        tag_input.type(t, delay=10)
        page.keyboard.press("Enter")
        page.wait_for_timeout(120)


PREMIUM_CHECKBOX = "isPremiumDownload"
PREMIUM_PRICE_SELECTOR = 'input[name="premiumDownloadPrice"]'


def _step_premium(page, e):
    price = e.get("price")
    if price is None:
        return
    set_checkbox(page, PREMIUM_CHECKBOX, True)
    page.wait_for_timeout(300)
    page.locator(PREMIUM_PRICE_SELECTOR).fill(f"{float(price):g}")


TIER_SELECTOR = 'select[name="premiumFolderTier"]'
GALLERY_ADD_BUTTON = 'button[aria-label="Add to Gallery"]'


def _step_tier(page, e):
    tier = e.get("tier")
    if not tier:
        return
    page.select_option(TIER_SELECTOR, label=tier)


def _step_galleries(page, e):
    for g in e.get("galleries") or []:
        page.locator(GALLERY_ADD_BUTTON).first.click()
        page.wait_for_timeout(300)
        page.get_by_text(g, exact=True).first.click()
        page.wait_for_timeout(200)


def _step_schedule(page, e):
    pick_schedule(page, format_schedule(int(e["scheduleTs"])))
    set_checkbox(page, "matureContent", True)  # re-assert: racy, can drop after the schedule dialog
    page.get_by_text("Schedule", exact=True).last.click()
    page.wait_for_timeout(5000)


# Ordered to mirror the SKILL.md "### Steps" list. Titles here are the skill
# lines (backticks stripped); check_steps() asserts this stays 1:1 with the
# skill, so a step added there but not implemented here is caught.
STEPS: list[tuple[str, object]] = [
    ("Connect to www.deviantart.com", _step_goto),
    ("Click on submit", _step_submit),
    ('Click on "upload your art" and pick file <pub.path>', _step_upload),
    ("Set as title <pub.title>", _step_title),
    ("Set as description <pub.description>", _step_description),
    ('Tick boxes "Mature" and "Created using AI tools"', _step_checkboxes),
    ('Drop all the pre-filled tags in the "Tags" field', _step_clear_tags),
    ('Copy the content of ../publicator.py/tags/da.txt into the "Tags" field', _step_add_tags),
    ('If the piece has a price, tick "Submit as Premium Download" and set the price', _step_premium),
    ("Set the subscription tier <pub.tier>", _step_tier),
    ("Add to galleries <pub.galleries>", _step_galleries),
    ("Schedule publication for the <pub.schedule>", _step_schedule),
]


def submit_entry(page, entry: dict) -> None:
    """Drive DA through every SKILL step for one entry."""
    for i, (title, fn) in enumerate(STEPS, 1):
        log.debug("step %d/%d: %s", i, len(STEPS), title)
        fn(page, entry)


# ---------------------------------------------------------------------------
# Skill <-> STEPS drift check
# ---------------------------------------------------------------------------

def _norm(s: str) -> str:
    """Compare step text without markdown noise: drop backticks, collapse space."""
    return re.sub(r"\s+", " ", s.replace("`", "")).strip()


def parse_skill_steps(md_path: Path) -> list[str]:
    """The numbered lines under the skill's '### Steps' heading (source of truth)."""
    steps, in_steps = [], False
    for line in md_path.read_text(encoding="utf-8").splitlines():
        if line.strip().lower().startswith("### steps"):
            in_steps = True
            continue
        if in_steps:
            if line.startswith("#"):  # next heading ends the section
                break
            m = re.match(r"\s*\d+\.\s+(.*\S)\s*$", line)
            if m:
                steps.append(m.group(1))
    return steps


def check_steps() -> bool:
    """True iff STEPS mirror the skill 1:1. Prints an aligned diff on drift."""
    if not SKILL_MD.exists():
        print(f"check-steps: {SKILL_MD} not found (dev-only check; skipping)")
        return True
    skill = [_norm(s) for s in parse_skill_steps(SKILL_MD)]
    ours = [_norm(t) for t, _ in STEPS]
    if skill == ours:
        print(f"check-steps OK: {len(ours)} steps in sync with the skill")
        return True
    print("check-steps DRIFT: SKILL.md and da_publish.STEPS differ "
          f"(skill has {len(skill)}, script implements {len(ours)}):", file=sys.stderr)
    for i in range(max(len(skill), len(ours))):
        s = skill[i] if i < len(skill) else "<none — remove from STEPS?>"
        o = ours[i] if i < len(ours) else "<none — IMPLEMENT THIS STEP in da_publish.STEPS>"
        mark = "  ok" if s == o else ">>DRIFT"
        print(f"{mark} step {i + 1}\n    skill:  {s}\n    script: {o}", file=sys.stderr)
    return False


# ---------------------------------------------------------------------------
# Session prep + batch publish
# ---------------------------------------------------------------------------

_LOGIN_HINT = ("no valid DeviantArt session — run `nix run <publicator.py>#login`, "
               "sign in, then publish again")


def _prepare_session_from_login() -> None:
    """Copy the human sign-in profile into a fresh Playwright working copy.

    Login can't happen under Playwright (PerimeterX blocks it), so it's done
    out-of-band in a real Firefox via #login. Playwright's Firefox is a
    different build, so we work on a COPY (never the original) and drop the
    lock/version files that would otherwise make it refuse the profile."""
    if not LOGIN_DIR.exists() or not any(LOGIN_DIR.iterdir()):
        raise RuntimeError(_LOGIN_HINT)
    if SESSION_DIR.exists():
        shutil.rmtree(SESSION_DIR)
    # ponytail: skip Firefox caches — cookies/storage carry the session, the
    # caches are hundreds of MB of dead weight to copy on every run.
    shutil.copytree(LOGIN_DIR, SESSION_DIR, symlinks=True,
                    ignore=shutil.ignore_patterns("cache2", "startupCache",
                                                   "thumbnails", "*.log"))
    for name in ("lock", ".parentlock", "parent.lock", "compatibility.ini"):
        (SESSION_DIR / name).unlink(missing_ok=True)


def _session_authed(ctx) -> bool:
    """True if the copied profile carries a live DA auth cookie. Replaces the
    old interactive login wait: PerimeterX blocks signing in under Playwright,
    so the session must already exist (established out-of-band via #login)."""
    now = time.time()
    for c in ctx.cookies():
        if c.get("name") in ("auth_secure", "userinfo") and "deviantart" in c.get("domain", ""):
            exp = c.get("expires", -1)
            if exp in (-1, None) or exp > now:
                return True
    return False


def publish_batch(entries: list[dict], uuids: list[str],
                  json_path: str) -> tuple[int, int, str | None]:
    """Drive Firefox through DA submission for each entry. Returns (ok, failed, err)."""
    from playwright.sync_api import sync_playwright

    log.debug("preparing Playwright session from %s", LOGIN_DIR)
    try:
        _prepare_session_from_login()
    except RuntimeError as e:
        return 0, len(entries), str(e)

    published = 0
    failed = 0
    err: str | None = None

    with sync_playwright() as p:
        # Headful Firefox on a copy of the pre-signed-in profile. Login is done
        # out-of-band via #login (PerimeterX blocks it under Playwright); here we
        # only verify the session carried over. Headful, not headless: headless
        # is itself a bot-detection signal.
        ctx = p.firefox.launch_persistent_context(str(SESSION_DIR), headless=False)
        page = ctx.pages[0] if ctx.pages else ctx.new_page()

        if not _session_authed(ctx):
            ctx.close()
            return 0, len(entries), _LOGIN_HINT
        log.debug("DA session authed; publishing %d entr%s",
                  len(entries), "y" if len(entries) == 1 else "ies")

        for entry, u in zip(entries, uuids):
            try:
                submit_entry(page, entry)
                mark_state(json_path, u, STATE_PUBLISHED)
                published += 1
                print(f"published: {entry['title']}", flush=True)
            except Exception as e:
                failed += 1
                err = f"{entry.get('title', entry.get('path'))}: {e}"
                print(f"AUTOMATION FAILED: {err}", file=sys.stderr, flush=True)
                break

        ctx.close()

    return published, failed, err


# ---------------------------------------------------------------------------
# CLI + self-check
# ---------------------------------------------------------------------------

def _selfcheck() -> None:
    assert parse_schedule("Tue Sep 8 08:00:00 PM CEST 2026") == (2026, 9, 8, 20)
    assert parse_schedule("Wed Jan 1 12:00:00 AM UTC 2025") == (2025, 1, 1, 0)
    assert parse_schedule("Wed Jan 1 12:00:00 PM UTC 2025") == (2025, 1, 1, 12)
    assert [fn for _, fn in STEPS], "STEPS must map to callables"
    assert check_steps(), "STEPS drifted from the skill"

    class _NoPage:
        def __getattr__(self, _n): raise AssertionError("premium step touched page for a free entry")
    _step_premium(_NoPage(), {"price": None})
    _step_tier(_NoPage(), {})        # no tier -> no-op
    _step_galleries(_NoPage(), {})   # no galleries -> no-op

    # TAGS_FILE is resolved from publicator.toml [tags], under DATA_DIR
    _cwd0 = os.getcwd()
    with tempfile.TemporaryDirectory() as _d:
        os.chdir(_d)
        try:
            (Path(_d) / "publicator.toml").write_text('tags = "tags/da.txt"\n')
            configure(_d)
            assert TAGS_FILE == Path(_d) / "tags/da.txt", TAGS_FILE
            (Path(_d) / "publicator.toml").write_text("")   # no [tags] key
            configure(_d)
            assert TAGS_FILE is None, TAGS_FILE
        finally:
            os.chdir(_cwd0)
    print("da_publish selfcheck OK")


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
    ap.add_argument("--selfcheck", action="store_true", help="run offline self-checks, then exit")
    ap.add_argument("-v", "--verbose", action="store_true", help="debug logging (steps, session, llm)")
    a = ap.parse_args()

    setup_logging(a.verbose)

    if a.selfcheck:
        _selfcheck()
        return 0
    if a.check_steps:
        return 0 if check_steps() else 1

    configure(a.data_dir)
    json_path = a.json or str(DATA_DIR / "publications.json")
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
