import sqlite3
from pathlib import Path
from types import SimpleNamespace

import pytest

from publicator_da import browser


def test_profile_trees_must_be_disjoint(tmp_path):
    source = tmp_path / "login"
    work = tmp_path / "work"
    source.mkdir()
    work.mkdir()
    browser.validate_profiles(source, work)
    for a, b in ((source, source), (source, source / "nested"), (source / "nested", source)):
        with pytest.raises(ValueError):
            browser.validate_profiles(a, b)


def test_work_profile_copy_dereferences_symlinks(tmp_path, monkeypatch):
    source = tmp_path / "login"
    work = tmp_path / "work"
    external = tmp_path / "outside.sqlite"
    source.mkdir()
    external.write_text("source")
    (source / "linked.sqlite").symlink_to(external)
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
                                     "headless": True, "timezone": "Europe/Paris"})
    try:
        copied = work / "linked.sqlite"
        assert not copied.is_symlink()
        copied.write_text("work")
        assert external.read_text() == "source"
    finally:
        session.close()


def test_browser_without_cookie_database_passes_no_cookies(tmp_path, monkeypatch):
    import sys
    from types import ModuleType

    source, work = tmp_path / "login", tmp_path / "work"
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
    })
    try:
        assert calls["cookies"] == []
    finally:
        session.close()


def test_browser_setup_passes_headless_timezone_and_reinjects_cookies(tmp_path, monkeypatch):
    source, work = tmp_path / "login", tmp_path / "work"
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
                                     "headless": True, "timezone": "Europe/Paris"})
    session.close()
    assert calls["launch"][1] == {"headless": True, "timezone_id": "Europe/Paris"}
    assert calls["cookies"] == [{"name": "auth", "value": "secret", "domain": ".deviantart.com",
                                  "path": "/", "secure": True, "httpOnly": True}]
    assert calls["goto"][0] == "https://www.deviantart.com/studio?new=1"
    assert (source / "cookies.sqlite").is_file()


def test_initialization_failure_closes_partial_browser_resources(tmp_path, monkeypatch):
    source, work = tmp_path / "login", tmp_path / "work"
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
                               "headless": False, "timezone": "Europe/Paris"})
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
