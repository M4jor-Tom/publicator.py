"""DeviantArt submission — Playwright logic only.

Publishes ONE publications.json entry through the DeviantArt web submit flow and
flips its state to published_or_scheduled. `webui.server` (the gallery UI) and
`apps.da_publish` (the single-entry CLI) import `publish_batch`/
`load_pending_entries`/`configure` from here; this module never imports back, so
there is no cycle.

The ordered `STEPS` registry below mirrors, 1:1, the numbered list in the
`publish-deviantart` skill (SKILL.md). `--check-steps` asserts they stay in sync,
so a step added to the skill but not implemented here fails loudly.

DeviantArt is behind PerimeterX, which blocks every Playwright browser at the
LOGIN page. Login is therefore done out-of-band in a real Firefox (`#login`);
here we copy that pre-signed-in profile and only VERIFY the session — the submit
pages themselves are not bot-walled.
"""

import json
import logging
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime
from pathlib import Path

from publicator.config import load_config
from publicator.entries import (
    STATE_PUBLISHED,
    STATE_UNPUBLISHED,
    deviantart_apparition,
    find_art_path,
    format_schedule,
    set_data_dir,
)
from publicator.store import atomic_write_json, mark_state

log = logging.getLogger("publicator.da")

TAGS_FILE = None  # resolved by configure() from publicator.toml [tags], under DATA_DIR

# The skill file is a repo asset, not a package one: src/publicator/deviantart.py
# -> parents[2] is the repo root. Absent (e.g. an installed copy) -> check_steps
# reports "not found" and skips, which is the documented dev-only behaviour.
REPO_ROOT = Path(__file__).resolve().parents[2]
SKILL_MD = REPO_ROOT / ".claude/skills/publish-deviantart/SKILL.md"

# Runtime state (publications.json, images, browser session) lives in the
# publication database dir: --data-dir, defaulting to CWD. configure() retargets.
DATA_DIR = Path.cwd()
LOGIN_DIR = DATA_DIR / ".deviantart-login"      # human sign-in profile (real Firefox, via #login)
SESSION_DIR = DATA_DIR / ".deviantart-session"  # Playwright working copy of the login profile

# schedule string from `date`: "Tue Jul 28 08:00:00 PM CEST 2026"
_MONTHS = {m: i + 1 for i, m in enumerate(
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"])}


def configure(data_dir, config=None) -> Path:
    """Point this module (and the echo helpers) at a publication database dir.
    `config` is a pre-loaded publicator.toml dict; None -> load it here (for
    standalone callers). Returns the resolved DATA_DIR."""
    global DATA_DIR, LOGIN_DIR, SESSION_DIR, TAGS_FILE
    DATA_DIR = set_data_dir(data_dir)  # also points the echo helpers (find_art_path) at it
    LOGIN_DIR = DATA_DIR / ".deviantart-login"
    SESSION_DIR = DATA_DIR / ".deviantart-session"
    tags = (config if config is not None else load_config(DATA_DIR)).get("tags")
    TAGS_FILE = DATA_DIR / tags if tags else None
    return DATA_DIR


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


DESCRIPTION_SELECTOR = '[contenteditable="true"]'  # rich-text editor; the only contenteditable on the form


def _step_description(page, e):
    page.locator(DESCRIPTION_SELECTOR).first.fill(e.get("description", ""))


def _step_checkboxes(page, e):
    set_checkbox(page, "matureContent", True)
    set_checkbox(page, "isAiGenerated", True)


def _step_clear_tags(page, e):
    """Step 6: remove any tags already in the Tags field so only <publicator.toml: "tags" path>
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
    """Step 7: type each tag from <publicator.toml: "tags" path> into the Tags field."""
    if TAGS_FILE is None:
        raise RuntimeError("no 'tags' path configured in publicator.toml")
    tags = [t.strip() for t in TAGS_FILE.read_text().splitlines() if t.strip()]
    tag_input = _tag_input(page)
    tag_input.click()
    for t in tags:
        tag_input.type(t, delay=10)
        page.keyboard.press("Enter")
        page.wait_for_timeout(120)


PREMIUM_TOGGLE_LABEL = "Submit as Premium Download"          # a <button> linked to this label
PREMIUM_PRICE_SELECTOR = 'input[name="downloadDollarPrice"]'  # appears once the toggle is on


def _step_premium(page, e):
    price = e.get("price")
    if price is None:
        return
    # ponytail: fresh upload starts with the toggle off, so one click turns it on.
    page.get_by_label(PREMIUM_TOGGLE_LABEL).click()
    page.wait_for_timeout(500)
    page.locator(PREMIUM_PRICE_SELECTOR).fill(f"{float(price):g}")


# Tier and gallery are role=combobox checkbox dropdowns with dynamic ids, so each
# is anchored to its stable section heading (first following combobox).
def _pick_in_combo(page, combo, text) -> None:
    """Open a role=combobox dropdown, tick the option whose text is `text`, then
    close it so it can't overlay later steps."""
    combo.click()
    page.wait_for_timeout(500)
    page.get_by_text(text, exact=True).first.click()
    combo.click()
    page.wait_for_timeout(200)


def _step_tier(page, e):
    tier = e.get("tier")
    if not tier:
        return
    combo = page.locator('xpath=//*[normalize-space(text())="Submit to your Subscribers"]/following::*[@role="combobox"][1]')
    combo.scroll_into_view_if_needed()
    _pick_in_combo(page, combo, tier)


def _step_galleries(page, e):
    for g in e.get("galleries") or []:
        combo = page.locator('xpath=//*[normalize-space(text())="Gallery"]/following::*[@role="combobox"][1]')
        combo.scroll_into_view_if_needed()
        # The combobox lists its selected folders as text; skip if already ticked
        # (blindly clicking would UNtick the default "Featured").
        if g not in combo.inner_text():
            _pick_in_combo(page, combo, g)


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
    ('Copy the content of the tags file (<data-dir>/<"tags" path>, per publicator.toml [tags]) into the "Tags" field', _step_add_tags),
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
    print("check-steps DRIFT: SKILL.md and deviantart.STEPS differ "
          f"(skill has {len(skill)}, script implements {len(ours)}):", file=sys.stderr)
    for i in range(max(len(skill), len(ours))):
        s = skill[i] if i < len(skill) else "<none — remove from STEPS?>"
        o = ours[i] if i < len(ours) else "<none — IMPLEMENT THIS STEP in deviantart.STEPS>"
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


def _da_login_cookies() -> list[dict]:
    """DeviantArt cookies from the human login profile's cookies.sqlite, shaped
    for context.add_cookies. Firefox reloads a copied profile but drops its
    httpOnly cookies (auth/auth_secure) on load, so we re-inject those from the
    source DB — without them the session reads as logged-out.
    ponytail: session cookies (no expires) — they outlive one publish run."""
    db = LOGIN_DIR / "cookies.sqlite"
    if not db.exists():
        return []
    con = sqlite3.connect(f"file:{db}?immutable=1", uri=True)
    try:
        rows = con.execute(
            "select name, value, host, path, isSecure, isHttpOnly "
            "from moz_cookies where host like '%deviantart%'").fetchall()
    finally:
        con.close()
    return [{"name": n, "value": v, "domain": h, "path": p or "/",
             "secure": bool(s), "httpOnly": bool(ho)}
            for n, v, h, p, s, ho in rows]


def _session_authed(page) -> bool:
    """True if the session reaches the submit studio without a bounce to the
    login page. Cookie presence lies: a stale `userinfo` cookie (30-day life)
    outlives the real `auth_secure` session token, so the old jar check
    green-lit dead sessions and only failed pages deep into the flow. Ask the
    site instead — this also catches a server-expired token, not just a missing
    cookie."""
    page.goto("https://www.deviantart.com/studio?new=1",
              wait_until="domcontentloaded", timeout=45000)
    page.wait_for_timeout(2000)
    return "/users/login" not in page.url


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
        # The copied Firefox profile drops its httpOnly auth cookies on load;
        # re-inject them from the login DB before checking the session.
        ctx.add_cookies(_da_login_cookies())

        if not _session_authed(page):
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
