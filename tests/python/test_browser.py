import sqlite3
from pathlib import Path
from types import SimpleNamespace

import pytest

from publicator_da import browser


class CalendarLocator:
    def __init__(self, page, labels=None, index=0):
        self.page = page
        self.labels = labels if labels is not None else []
        self.index = index

    @property
    def first(self):
        return self.nth(0)

    @property
    def last(self):
        return self.nth(len(self.labels) - 1)

    def nth(self, index):
        return CalendarLocator(self.page, self.labels, index)

    def count(self):
        return len(self.labels)

    def get_attribute(self, name):
        if self.page.stale_index == self.index:
            raise RuntimeError("calendar button detached")
        return self.labels[self.index] if name == "aria-label" else None

    def evaluate_all(self, expression):
        self.page.label_snapshots += 1
        return list(self.labels)

    def click(self, *args, **kwargs):
        if self.labels:
            if self.labels[self.index] == "Confirm Schedule":
                self.page.confirmed = True
            else:
                self.page.selected = self.labels[self.index]
            return
        if self.page.selector == 'button[aria-label="Go to the Next Month"]':
            self.page.next_months += 1
        elif self.page.selector == "#schedule-draft-date-picker [role=button]":
            self.page.date_picker_opened = True
        elif self.page.selector == 'button[aria-haspopup="menu"]':
            self.page.menu_opened = True
        elif self.page.selector == '[role=menuitem][label="Schedule"]':
            self.page.schedule_menu_opened = True

    def scroll_into_view_if_needed(self):
        pass


class CalendarPage:
    def __init__(self, labels):
        self.labels = labels
        self.selector = None
        self.selected = None
        self.stale_index = None
        self.label_snapshots = 0
        self.next_months = 0
        self.date_picker_opened = False
        self.menu_opened = False
        self.schedule_menu_opened = False
        self.confirmed = False

    def locator(self, selector):
        self.selector = selector
        if selector == "#schedule-draft-date-picker button[aria-label]":
            labels = self.labels
        else:
            labels = []
        return CalendarLocator(self, labels)

    def select_option(self, *args, **kwargs):
        pass

    def wait_for_timeout(self, milliseconds):
        pass

    def get_by_text(self, text, **kwargs):
        if text == "Confirm Schedule":
            return CalendarLocator(self, ["Confirm Schedule"])
        return CalendarLocator(self)

def test_schedule_chooses_the_requested_full_year_and_exact_day():
    page = CalendarPage([
        "Wednesday, January 1, 2025",
        "Thursday, January 1, 2026",
        "Saturday, January 10, 2026",
    ])
    browser._pick_schedule(page, {"year": 2026, "month": 1, "day": 1, "hour": 20})
    assert page.selected == "Thursday, January 1, 2026"
    assert page.confirmed


def test_schedule_snapshots_calendar_labels_instead_of_waiting_on_each_button():
    labels = ["Tuesday, November 10, 2026", *([""] * 32)]
    page = CalendarPage(labels)
    page.stale_index = 32

    browser._pick_schedule(page, {"year": 2026, "month": 11, "day": 10, "hour": 20})

    assert page.selected == labels[0]
    assert page.label_snapshots == 1
    assert page.confirmed


def test_schedule_refuses_unqualified_calendar_dates():
    page = CalendarPage(["January 15", "January 15"])
    with pytest.raises(RuntimeError, match="full requested date"):
        browser._pick_schedule(page, {"year": 2026, "month": 1, "day": 15, "hour": 20})
    assert page.selected is None
    assert not page.confirmed


def test_profile_trees_must_be_disjoint(tmp_path):
    source = tmp_path / ".deviantart-login"
    work = tmp_path / ".deviantart-session"
    source.mkdir()
    work.mkdir()
    browser.validate_profiles(tmp_path, source, work)
    for a, b in ((source, source), (source, source / "nested"), (source / "nested", source)):
        with pytest.raises(ValueError):
            browser.validate_profiles(tmp_path, a, b)


def test_foreign_or_symlink_work_profile_is_rejected_untouched(tmp_path, monkeypatch):
    import sys
    from types import ModuleType, SimpleNamespace

    source = tmp_path / ".deviantart-login"
    source.mkdir()
    (source / "prefs.js").write_text("login")
    foreign = tmp_path / "unrelated"
    foreign.mkdir()
    sentinel = foreign / "sentinel"
    sentinel.write_text("keep")
    work = foreign
    work_link = tmp_path / ".deviantart-session"
    work_link.symlink_to(foreign, target_is_directory=True)
    calls = []
    module = ModuleType("playwright.sync_api")
    module.sync_playwright = lambda: calls.append("acquire") or SimpleNamespace(start=lambda: None)
    monkeypatch.setitem(sys.modules, "playwright.sync_api", module)

    for work_profile in (work, work_link):
        with pytest.raises(ValueError):
            browser.BrowserSession({
                "login_profile": str(source), "work_profile": str(work_profile),
                "headless": True, "timezone": "Europe/Paris",
            }, tmp_path)
        assert sentinel.read_text() == "keep"
        assert list(foreign.iterdir()) == [sentinel]
    assert calls == []


def test_browser_session_rejects_same_name_foreign_root_before_playwright(tmp_path, monkeypatch):
    from types import ModuleType, SimpleNamespace

    trusted = tmp_path / "trusted"
    trusted.mkdir()
    source = tmp_path / "foreign" / ".deviantart-login"
    work = tmp_path / "foreign" / ".deviantart-session"
    work.parent.mkdir()
    source.mkdir()
    work.mkdir()
    sentinel = work / "sentinel"
    sentinel.write_text("keep")
    acquired = []
    module = ModuleType("playwright.sync_api")
    module.sync_playwright = lambda: acquired.append(True) or SimpleNamespace(start=lambda: None)
    monkeypatch.setitem(__import__("sys").modules, "playwright.sync_api", module)

    with pytest.raises(ValueError):
        browser.BrowserSession({
            "login_profile": str(source), "work_profile": str(work),
            "headless": True, "timezone": "Europe/Paris",
        }, trusted)
    assert acquired == []
    assert sentinel.read_text() == "keep"
    assert sorted(path.name for path in work.iterdir()) == ["sentinel"]


def test_work_profile_copy_dereferences_symlinks_and_skips_stale_lock(tmp_path, monkeypatch):
    source = tmp_path / ".deviantart-login"
    work = tmp_path / ".deviantart-session"
    external = tmp_path / "outside.sqlite"
    source.mkdir()
    external.write_text("source")
    (source / "linked.sqlite").symlink_to(external)
    (source / "lock").symlink_to(tmp_path / "missing-lock-target")
    # Copy behavior is exercised through session construction after Playwright is faked.
    import sys
    from types import ModuleType, SimpleNamespace
    fake_page = SimpleNamespace(goto=lambda *a, **k: None, wait_for_timeout=lambda ms: None, url="/studio")
    fake_context = SimpleNamespace(pages=[fake_page], new_page=lambda: fake_page,
                                   add_cookies=lambda cookies: None, close=lambda: None)
    fake_playwright = SimpleNamespace(firefox=SimpleNamespace(
        launch_persistent_context=lambda *a, **k: fake_context), stop=lambda: None)
    monkeypatch.setitem(sys.modules, "playwright.sync_api", ModuleType("playwright.sync_api"))
    sys.modules["playwright.sync_api"].sync_playwright = lambda: SimpleNamespace(start=lambda: fake_playwright)
    monkeypatch.setattr(browser.BrowserSession, "_cookies", lambda self: [])
    session = browser.BrowserSession({"login_profile": str(source), "work_profile": str(work),
                                     "headless": True, "timezone": "Europe/Paris"}, tmp_path)
    try:
        copied = work / "linked.sqlite"
        assert not copied.is_symlink()
        copied.write_text("work")
        assert external.read_text() == "source"
        assert not (work / "lock").is_symlink()
    finally:
        session.close()


def test_browser_without_cookie_database_passes_no_cookies(tmp_path, monkeypatch):
    import sys
    from types import ModuleType

    source, work = tmp_path / ".deviantart-login", tmp_path / ".deviantart-session"
    source.mkdir()
    (source / "prefs.js").write_text("disposable profile")
    calls = {}

    class Page:
        url = "https://www.deviantart.com/studio?new=1"
        def goto(self, *args, **kwargs): pass
        def wait_for_timeout(self, ms): pass

    class Context:
        pages = [Page()]
        def add_cookies(self, cookies): calls["cookies"] = cookies
        def close(self): pass

    context = Context()
    playwright = SimpleNamespace(
        firefox=SimpleNamespace(
            launch_persistent_context=lambda *args, **kwargs: context
        ),
        stop=lambda: None,
    )
    monkeypatch.setitem(sys.modules, "playwright.sync_api", ModuleType("playwright.sync_api"))
    sys.modules["playwright.sync_api"].sync_playwright = lambda: SimpleNamespace(
        start=lambda: playwright
    )

    session = browser.BrowserSession({
        "login_profile": str(source),
        "work_profile": str(work),
        "headless": True,
        "timezone": "Europe/Paris",
    }, tmp_path)
    try:
        assert calls["cookies"] == []
    finally:
        session.close()


def test_browser_setup_passes_headless_timezone_and_reinjects_cookies(tmp_path, monkeypatch):
    source, work = tmp_path / ".deviantart-login", tmp_path / ".deviantart-session"
    source.mkdir()
    db = sqlite3.connect(source / "cookies.sqlite")
    db.execute("create table moz_cookies (name,value,host,path,isSecure,isHttpOnly)")
    db.execute("insert into moz_cookies values ('auth','secret','.deviantart.com','/',1,1)")
    db.commit(); db.close()
    calls = {}

    class Page:
        url = "https://www.deviantart.com/studio?new=1"
        def goto(self, url, **kwargs): calls["goto"] = (url, kwargs)
        def wait_for_timeout(self, ms): pass

    page = Page()
    class Context:
        pages = [page]
        def add_cookies(self, cookies): calls["cookies"] = cookies
        def close(self): pass
    context = Context()
    class Playwright:
        firefox = self = None
        def launch_persistent_context(self, path, **kwargs):
            calls["launch"] = (path, kwargs); return context
        def stop(self): pass
    pw = Playwright(); pw.firefox = pw
    from types import ModuleType, SimpleNamespace
    monkeypatch.setitem(__import__("sys").modules, "playwright.sync_api", ModuleType("playwright.sync_api"))
    __import__("sys").modules["playwright.sync_api"].sync_playwright = lambda: SimpleNamespace(start=lambda: pw)

    session = browser.BrowserSession({"login_profile": str(source), "work_profile": str(work),
                                     "headless": True, "timezone": "Europe/Paris"}, tmp_path)
    session.close()
    assert calls["launch"][1] == {"headless": True, "timezone_id": "Europe/Paris"}
    assert calls["cookies"] == [{"name": "auth", "value": "secret", "domain": ".deviantart.com",
                                  "path": "/", "secure": True, "httpOnly": True}]
    assert calls["goto"][0] == "https://www.deviantart.com/studio?new=1"
    assert (source / "cookies.sqlite").is_file()


def test_initialization_failure_closes_partial_browser_resources(tmp_path, monkeypatch):
    source, work = tmp_path / ".deviantart-login", tmp_path / ".deviantart-session"
    source.mkdir(); (source / "profile").write_text("x")
    calls = []
    class Context:
        pages = []
        def new_page(self): raise RuntimeError("page failed")
        def close(self): calls.append("context.close")
    context = Context()
    class Playwright:
        firefox = self = None
        def launch_persistent_context(self, *a, **kw): return context
        def stop(self): calls.append("playwright.stop")
    pw = Playwright(); pw.firefox = pw
    from types import ModuleType, SimpleNamespace
    monkeypatch.setitem(__import__("sys").modules, "playwright.sync_api", ModuleType("playwright.sync_api"))
    __import__("sys").modules["playwright.sync_api"].sync_playwright = lambda: SimpleNamespace(start=lambda: pw)
    monkeypatch.setattr(browser.BrowserSession, "_cookies", lambda self: [])
    with pytest.raises(RuntimeError, match="page failed"):
        browser.BrowserSession({"login_profile": str(source), "work_profile": str(work),
                               "headless": False, "timezone": "Europe/Paris"}, tmp_path)
    assert calls == ["context.close", "playwright.stop"]


def test_steps_parser_detects_skill_drift(tmp_path):
    skill = tmp_path / "SKILL.md"
    skill.write_text("### Steps\n1. Connect to site\n2. Submit\n\n### Other\n")
    assert browser.check_steps(skill, ["Connect to site", "Submit"])
    assert not browser.check_steps(skill, ["Connect to site"])


def test_submit_selects_every_requested_tier(monkeypatch, tmp_path):
    selected = []

    class Locator:
        first = last = None

        def __init__(self): self.first = self.last = self
        def click(self, *args, **kwargs): pass
        def fill(self, value): pass
        def type(self, value, delay=0): pass
        def count(self): return 0
        def locator(self, selector): return self
        def scroll_into_view_if_needed(self): pass

    class Chooser:
        value = SimpleNamespace(set_files=lambda path: None)
        def __enter__(self): return self
        def __exit__(self, *args): pass

    class Page:
        keyboard = SimpleNamespace(press=lambda key: None)
        def goto(self, *args, **kwargs): pass
        def get_by_role(self, role, **kwargs): return Locator()
        def get_by_text(self, text, **kwargs): return Locator()
        def get_by_label(self, label, **kwargs): return Locator()
        def locator(self, selector): return Locator()
        def expect_file_chooser(self, **kwargs): return Chooser()
        def wait_for_load_state(self, *args, **kwargs): pass
        def wait_for_timeout(self, ms): pass

    monkeypatch.setattr(browser, "set_checkbox", lambda *args, **kwargs: None)
    monkeypatch.setattr(browser, "_tag_input", lambda page: Locator())
    monkeypatch.setattr(browser, "_pick_schedule", lambda *args: None)
    monkeypatch.setattr(browser, "_pick_combo", lambda _page, _combo, tier: selected.append(tier))
    browser._submit_entry(
        Page(),
        {
            "image_path": str(tmp_path / "art.png"),
            "title": "Title",
            "description": "Description",
            "tags": [],
            "schedule": {},
            "tiers": ["gold", "silver"],
            "galleries": [],
        },
        lambda: None,
    )
    assert selected == ["gold", "silver"]


def test_submit_skips_unset_premium_tiers_and_galleries(monkeypatch, tmp_path):
    calls = []
    class Locator:
        def __init__(self, label=""): self.label = label
        @property
        def first(self): return self
        @property
        def last(self): return self
        def click(self, *a, **kw): calls.append(("click", self.label))
        def fill(self, value): calls.append(("fill", self.label))
        def set_files(self, path): calls.append(("file", path))
        def count(self): return 0
        def wait_for_timeout(self, ms): pass
    class Chooser:
        value = Locator("chooser")
        def __enter__(self): return self
        def __exit__(self, *a): pass
    class Page:
        keyboard = SimpleNamespace(press=lambda key: None)
        def goto(self, *a, **kw): pass
        def get_by_role(self, role, **kw): return Locator(f"role:{role}:{kw}")
        def wait_for_load_state(self, *a, **kw): pass
        def expect_file_chooser(self, **kw): return Chooser()
        def get_by_text(self, text, **kw): return Locator(f"text:{text}")
        def get_by_label(self, label, **kw): return Locator(f"label:{label}")
        def locator(self, selector):
            if "input[aria-errormessage" in selector:
                return Locator("tags")
            return Locator(f"selector:{selector}")
        def wait_for_timeout(self, ms): pass
    monkeypatch.setattr(browser, "set_checkbox", lambda *a, **kw: None)
    monkeypatch.setattr(browser, "_tag_input", lambda page: Locator("tags"))
    monkeypatch.setattr(browser, "_pick_schedule", lambda *a: None)
    browser._submit_entry(Page(), {"image_path": str(tmp_path / "art.png"), "title": "Title",
                                   "description": "Description", "tags": [], "schedule": {},
                                   "tiers": [], "galleries": []}, lambda: None)
    assert not any("Sell as Premium Download" in str(call) or
                   "Submit to your subscribers" in str(call) or "Gallery" in str(call)
                   for call in calls)
