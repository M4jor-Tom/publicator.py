import sys
import types
from unittest.mock import ANY, MagicMock

import pytest

from publicator import deviantart
from publicator.apps import da_publish

PENDING = [{"uuid": "u1", "title": "one", "path": "/a.png", "scheduleTs": 0},
           {"uuid": "u2", "title": "two", "path": "/b.png", "scheduleTs": 0}]


@pytest.fixture
def cli(monkeypatch, tmp_path):
    """Run da_publish.main() with argv; returns (rc, publish_batch mock)."""
    monkeypatch.setattr(da_publish, "load_pending_entries", lambda _: list(PENDING))
    pb = MagicMock(return_value=(1, 0, None))
    monkeypatch.setattr(da_publish, "publish_batch", pb)

    def run(*argv):
        monkeypatch.setattr(sys, "argv", ["da-publish", "--data-dir", str(tmp_path), *argv])
        return da_publish.main(), pb
    return run


def test_default_publishes_only_the_first_pending_entry(cli):
    rc, pb = cli()
    assert rc == 0
    pb.assert_called_once_with([PENDING[0]], ["u1"], ANY, headless=False)


def test_all_publishes_every_pending_entry_in_one_batch(cli):
    rc, pb = cli("--all")
    assert rc == 0
    pb.assert_called_once_with(PENDING, ["u1", "u2"], ANY, headless=False)


def test_headless_flag_reaches_publish_batch(cli):
    _, pb = cli("--headless")
    assert pb.call_args.kwargs == {"headless": True}


def test_all_and_uuid_are_mutually_exclusive(cli, capsys):
    with pytest.raises(SystemExit) as e:
        cli("--all", "--uuid", "u1")
    assert e.value.code == 2
    assert "not allowed with" in capsys.readouterr().err


def test_publish_batch_launches_firefox_headless_when_asked(monkeypatch, tmp_path):
    pw = MagicMock()
    pw.__enter__.return_value = pw
    monkeypatch.setitem(sys.modules, "playwright.sync_api",
                        types.SimpleNamespace(sync_playwright=lambda: pw))
    monkeypatch.setattr(deviantart, "_prepare_session_from_login", lambda: None)
    monkeypatch.setattr(deviantart, "_da_login_cookies", lambda: [])
    monkeypatch.setattr(deviantart, "_session_authed", lambda _page: False)

    deviantart.publish_batch(PENDING[:1], ["u1"], str(tmp_path / "p.json"), headless=True)
    pw.firefox.launch_persistent_context.assert_called_once_with(
        str(deviantart.SESSION_DIR), headless=True)
