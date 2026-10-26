import logging
import shutil
import sqlite3
from pathlib import Path

log = logging.getLogger(__name__)


def validate_profiles(data_root: Path, source: Path, work: Path) -> tuple[Path, Path]:
    data_root = Path(data_root).resolve()
    source, work = Path(source), Path(work)
    if not data_root.is_dir():
        raise ValueError("trusted data root must be a directory")
    if source.is_symlink() or work.is_symlink():
        raise ValueError("login and work profiles cannot be symlinks")
    source, work = source.resolve(), work.resolve()
    if source != data_root / ".deviantart-login" or work != data_root / ".deviantart-session":
        raise ValueError("profiles must be the designated children of the trusted data root")
    if source == work or source in work.parents or work in source.parents:
        raise ValueError("login and work profiles must be disjoint trees")
    return source, work


class BrowserSession:
    def __init__(self, open_command, data_root):
        self.command = open_command
        self.source, self.work = validate_profiles(
            data_root,
            Path(open_command["login_profile"]),
            Path(open_command["work_profile"]),
        )
        from playwright.sync_api import sync_playwright
        if not self.source.is_dir() or not any(self.source.iterdir()):
            raise RuntimeError("no valid DeviantArt session; run the real Firefox login first")
        if self.work.exists():
            shutil.rmtree(self.work)
        shutil.copytree(self.source, self.work, ignore=shutil.ignore_patterns(
            "cache2", "startupCache", "thumbnails", "*.log",
            "lock", ".parentlock", "parent.lock", "compatibility.ini",
        ))
        self.playwright = None
        self.context = None
        try:
            self.playwright = sync_playwright().start()
            self.context = self.playwright.firefox.launch_persistent_context(
                str(self.work), headless=open_command["headless"], timezone_id=open_command["timezone"])
            self.page = self.context.pages[0] if self.context.pages else self.context.new_page()
            self.context.add_cookies(self._cookies())
            self.page.goto("https://www.deviantart.com/studio?new=1", wait_until="domcontentloaded", timeout=45000)
            self.page.wait_for_timeout(2000)
            if "/users/login" in self.page.url:
                raise RuntimeError("no valid DeviantArt session; sign in using the real Firefox login profile")
        except Exception:
            self.close()
            raise

    def _cookies(self):
        db = self.source / "cookies.sqlite"
        if not db.exists():
            return []
        con = sqlite3.connect(f"file:{db}?immutable=1", uri=True)
        try:
            rows = con.execute("select name,value,host,path,isSecure,isHttpOnly from moz_cookies where host like '%deviantart%'").fetchall()
        finally:
            con.close()
        return [{"name": n, "value": v, "domain": h, "path": p or "/", "secure": bool(s), "httpOnly": bool(ho)} for n, v, h, p, s, ho in rows]

    def submit(self, job):
        state = {"commit_attempted": False}
        try:
            _submit_entry(self.page, job, lambda: state.__setitem__("commit_attempted", True))
            event = {"protocol": 1, "type": "scheduled", "attempt_id": job["attempt_id"], "uuid": job["uuid"]}
        except Exception as exc:
            event = {"protocol": 1, "type": "unknown" if state["commit_attempted"] else "failed", "attempt_id": job["attempt_id"], "uuid": job["uuid"], "error": str(exc)}
        return event

    def close(self):
        context, self.context = getattr(self, "context", None), None
        playwright, self.playwright = getattr(self, "playwright", None), None
        try:
            if context:
                context.close()
        finally:
            if playwright:
                playwright.stop()


def set_checkbox(page, name, want=True):
    for _ in range(3):
        checked = page.evaluate('n => document.querySelector(`input[name="${n}"]`).checked', name)
        if checked == want:
            return
        page.evaluate('n => document.querySelector(`input[name="${n}"]`).click()', name)
        page.wait_for_timeout(300)
    raise RuntimeError(f"checkbox {name} would not stay {want}")


def _tag_input(page):
    return page.locator('input[aria-errormessage$="-error"]').last


def _pick_schedule(page, s):
    y, m, d, hour = s["year"], s["month"], s["day"], s["hour"]
    caret = page.locator('button[aria-haspopup="menu"]').last
    caret.scroll_into_view_if_needed(); caret.click(); page.wait_for_timeout(800)
    page.locator('[role="menuitem"][label="Schedule"]').first.click(force=True)
    page.locator('#schedule-draft-date-picker [role="button"]').click(); page.wait_for_timeout(500)
    import calendar
    import re
    month = re.escape(calendar.month_name[m])
    date_label = re.compile(rf"(?<![A-Za-z]){month}\s+0?{d}(?:st|nd|rd|th)?[,]?\s+{y}(?!\d)")
    selected = None
    for _ in range(24):
        buttons = page.locator('#schedule-draft-date-picker button[aria-label]')
        matches = [
            buttons.nth(index)
            for index in range(buttons.count())
            if date_label.search(buttons.nth(index).get_attribute("aria-label") or "")
        ]
        if len(matches) > 1:
            raise RuntimeError(f"calendar exposes an ambiguous full requested date {y}-{m:02}-{d:02}")
        if matches:
            selected = matches[0]
            break
        page.locator('button[aria-label="Go to the Next Month"]').click(); page.wait_for_timeout(200)
    if selected is None:
        raise RuntimeError(f"calendar did not expose the full requested date {y}-{m:02}-{d:02}")
    selected.click()
    page.select_option('#schedule-draft-time-picker', value=str(hour)); page.wait_for_timeout(300)
    page.get_by_text("Confirm Schedule", exact=True).click(); page.wait_for_timeout(1500)


def _pick_combo(page, combo, text):
    combo.click(); page.wait_for_timeout(500)
    page.get_by_text(text, exact=True).first.click()
    combo.click(); page.wait_for_timeout(200)


def _submit_entry(page, job, before_commit):
    page.goto("https://www.deviantart.com", wait_until="domcontentloaded", timeout=45000)
    page.get_by_role("link", name="Submit").first.click(); page.wait_for_load_state("domcontentloaded", timeout=30000)
    with page.expect_file_chooser(timeout=15000) as fc:
        page.get_by_text("Upload Your Art", exact=True).click()
    fc.value.set_files(job["image_path"])
    page.get_by_label("Title", exact=False).first.fill(job["title"])
    page.locator('[contenteditable="true"]').first.fill(job["description"])
    set_checkbox(page, "matureContent", True); set_checkbox(page, "isAiGenerated", True)
    tags = _tag_input(page); tags.click()
    chips = page.locator('[role="combobox"] span[role="button"][data-tag]')
    for _ in range(60):
        if not chips.count(): break
        try: chips.first.click()
        except Exception: pass
        page.wait_for_timeout(80)
    for tag in job["tags"]:
        tags.type(tag, delay=10); page.keyboard.press("Enter"); page.wait_for_timeout(120)
    if job.get("price") is not None:
        page.get_by_label("Sell as Premium Download").click(); page.wait_for_timeout(500)
        page.locator('input[name="downloadDollarPrice"]').fill(f'{float(job["price"]):g}')
    if job["tiers"]:
        combo = page.get_by_text("Submit to your subscribers").locator('xpath=following::*[@role="combobox"][1]')
        combo.scroll_into_view_if_needed()
        for tier in job["tiers"]: _pick_combo(page, combo, tier)
    for gallery in job["galleries"]:
        combo = page.locator('xpath=//*[normalize-space(text())="Gallery"]/following::*[@role="combobox"][1]')
        combo.scroll_into_view_if_needed()
        if gallery not in combo.inner_text(): _pick_combo(page, combo, gallery)
    _pick_schedule(page, job["schedule"])
    set_checkbox(page, "matureContent", True)
    before_commit()
    page.get_by_text("Schedule", exact=True).last.click()
    page.wait_for_timeout(5000)


def _noop_optional(page, job):
    return None


STEPS = [
    "Connect to www.deviantart.com", "Click on submit",
    'Click on "upload your art" and pick file `<pub.path>`',
    "Set as title <pub.title>", "Set as description <pub.description>",
    'Tick boxes "Mature" and "Created using AI tools"',
    'Drop all the pre-filled tags in the "Tags" field',
    'Copy the content of the tags file (<data-dir>/<"tags" path>, per the top-level "tags" key of publicator.toml) into the "Tags" field',
    'If the piece has a price, tick "Sell as Premium Download" and set the price',
    "Set the subscription tiers <pub.tiers>", "Add to galleries <pub.galleries>",
    "Schedule publication for the <pub.schedule>",
]


def check_steps(skill, steps):
    import re
    lines = Path(skill).read_text(encoding="utf-8").splitlines()
    in_steps, actual = False, []
    for line in lines:
        if line.strip().lower().startswith("### steps"):
            in_steps = True
            continue
        if in_steps and line.startswith("#"):
            break
        if in_steps and (m := re.match(r"\s*\d+\.\s+(.*\S)\s*$", line)):
            actual.append(m.group(1))
    norm = lambda value: re.sub(r"\s+", " ", value.replace("`", "")).strip()
    if [norm(s) for s in actual] == [norm(s) for s in steps]:
        print(f"check-steps OK: {len(steps)} steps in sync with the skill")
        return True
    print(f"check-steps DRIFT: skill has {len(actual)} steps, worker implements {len(steps)}", file=__import__("sys").stderr)
    return False
