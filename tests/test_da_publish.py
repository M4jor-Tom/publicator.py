import sys
import types
from unittest.mock import ANY, MagicMock

import pytest

from publicator import deviantart
from publicator.apps import da_publish

PENDING = [{"uuid": "u1", "title": "one", "description": "d", "path": "/a.png", "scheduleTs": 0},
           {"uuid": "u2", "title": "two", "description": "d", "path": "/b.png", "scheduleTs": 0}]


@pytest.fixture
def cli(monkeypatch, tmp_path):
    """Run da_publish.main() with argv; returns (rc, publish_selected mock)."""
    monkeypatch.setattr(da_publish, "load_pending_entries", lambda _: list(PENDING))
    ps = MagicMock(return_value=(1, 0, None))
    monkeypatch.setattr(da_publish, "publish_selected", ps)

    def run(*argv):
        monkeypatch.setattr(sys, "argv", ["da-publish", "--data-dir", str(tmp_path), *argv])
        return da_publish.main(), ps
    return run


def test_default_publishes_only_the_first_pending_entry(cli):
    rc, ps = cli()
    assert rc == 0
    ps.assert_called_once_with(ANY, ANY, platform_names=["deviantart"], uuids=["u1"], headless=False)


def test_all_publishes_every_pending_entry_in_one_batch(cli):
    rc, ps = cli("--all")
    assert rc == 0
    ps.assert_called_once_with(ANY, ANY, platform_names=["deviantart"], uuids=["u1", "u2"], headless=False)


def test_headless_flag_reaches_dispatcher(cli):
    _, ps = cli("--headless")
    assert ps.call_args.kwargs["headless"] is True


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

    from publicator import store

    uuid_ = "00000000-0000-0000-0000-000000000001"
    path = tmp_path / "p.json"
    (tmp_path / "art.png").write_bytes(b"art")
    store.atomic_write_json(str(path), [{
        "uuid": uuid_, "state": "DESCRIBED", "title": "Title", "description": "Description",
        "submissionTimestamp": 1, "files": [{"basename": "art.png"}],
        "apparitions": [{"platformName": "deviantart", "state": "unpublished"}],
    }])
    deviantart.configure(str(tmp_path), {})
    pending = deviantart.load_pending_entries(str(path))
    deviantart.publish_batch(pending, [uuid_], str(path), headless=True)
    pw.firefox.launch_persistent_context.assert_called_once_with(
        str(deviantart.SESSION_DIR), headless=True)
