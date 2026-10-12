import json
import sys
import types
from pathlib import Path
from unittest.mock import MagicMock

from jsonschema.exceptions import ValidationError

import pytest

from publicator import deviantart, platforms, store
from publicator.platforms import Platform


UUID = "00000000-0000-0000-0000-000000000001"
CFG = {"schedule": {}, "tiers": [], "galleries": []}


def write_pubs(path, apparitions, *, state="DESCRIBED"):
    store.atomic_write_json(str(path), [{
        "uuid": UUID,
        "files": [],
        "submissionTimestamp": 1,
        "apparitionTimestampIfDifferentThanSubmission": 1800000000,
        "title": "Title",
        "description": "Description",
        "state": state,
        "apparitions": apparitions,
    }])


class FakePublisher:
    def __init__(self, name, *, fail=False):
        self.name = name
        self.fail = fail
        self.published = []
        self.configured = []

    def configure(self, data_dir, config=None):
        self.configured.append((data_dir, config))

    def load_pending_entries(self, json_path):
        data = json.loads(Path(json_path).read_text())
        return [{"uuid": pub["uuid"], "title": pub["title"]}
                for pub in data if pub.get("state") == "DESCRIBED"
                for app in pub.get("apparitions", [])
                if app.get("platformName") == self.name and app.get("state") == "unpublished"]

    def publish_batch(self, entries, uuids, json_path, headless=False):
        if self.fail:
            return 0, len(entries), f"{self.name} broke"
        for uuid_ in uuids:
            store.mark_state(json_path, uuid_, "published_or_scheduled", platform=self.name, config=CFG)
        self.published.extend(uuids)
        return len(entries), 0, None


@pytest.fixture
def browser_stub(monkeypatch):
    """Stub only browser/session operations; delivery uses the real store."""
    pw = MagicMock()
    pw.__enter__.return_value = pw
    monkeypatch.setitem(sys.modules, "playwright.sync_api",
                        types.SimpleNamespace(sync_playwright=lambda: pw))
    prepare = MagicMock()
    submit = MagicMock()
    monkeypatch.setattr(deviantart, "_prepare_session_from_login", prepare)
    monkeypatch.setattr(deviantart, "_da_login_cookies", lambda: [])
    monkeypatch.setattr(deviantart, "_session_authed", lambda _page: True)
    monkeypatch.setattr(deviantart, "submit_entry", submit)
    return pw, prepare, submit


def write_da_publication(path, data_dir, **app_options):
    path.parent.mkdir(parents=True, exist_ok=True)
    (data_dir / "art.png").write_bytes(b"art")
    write_pubs(path, [{"platformName": "deviantart", "state": "unpublished", **app_options}])
    rows = store.load_publications(str(path))
    rows[0]["files"] = [{"basename": "art.png"}]
    store.atomic_write_json(str(path), rows)


def test_dispatch_persists_delivery_with_explicit_config(tmp_path, browser_stub):
    path = tmp_path / "publications.json"
    write_da_publication(path, tmp_path, galleries=["Allowed"])
    config = {**CFG, "galleries": ["Allowed"]}

    assert platforms.publish_selected(str(path), str(tmp_path), config=config) == (1, 0, None)
    pub = store.load_publications(str(path))[0]
    assert pub["state"] == "APPEARED"
    assert pub["apparitions"][0]["state"] == "published_or_scheduled"
    assert platforms.publish_selected(str(path), str(tmp_path), config=config) == (0, 0, None)
    browser_stub[2].assert_called_once()


def test_da_cli_uses_data_dir_config_for_external_json(tmp_path, monkeypatch, browser_stub):
    from publicator.apps import da_publish

    data_dir = tmp_path / "data"
    data_dir.mkdir()
    (data_dir / "publicator.toml").write_text('[deviantart]\ngalleries = ["Allowed"]\n')
    path = tmp_path / "other" / "publications.json"
    write_da_publication(path, data_dir, galleries=["Allowed"])
    monkeypatch.setattr(sys, "argv", ["da-publish", "--data-dir", str(data_dir), "--json", str(path)])

    assert da_publish.main() == 0
    assert store.load_publications(str(path))[0]["state"] == "APPEARED"
    assert da_publish.main() == 0
    browser_stub[2].assert_called_once()


@pytest.mark.parametrize("invalid", ["uuid", "galleries", "tiers", "tier"])
@pytest.mark.parametrize("entrypoint", ["dispatcher", "batch"])
def test_invalid_database_never_starts_browser(tmp_path, browser_stub, invalid, entrypoint):
    path = tmp_path / "publications.json"
    write_da_publication(path, tmp_path)
    rows = store.load_publications(str(path))
    if invalid == "uuid":
        rows[0]["uuid"] = "short"
    else:
        rows[0]["apparitions"][0][invalid] = "Denied" if invalid == "tier" else ["Denied"]
    store.atomic_write_json(str(path), rows)
    before = path.read_bytes()

    if entrypoint == "dispatcher":
        with pytest.raises((ValidationError, ValueError)):
            platforms.publish_selected(str(path), str(tmp_path), config=CFG)
    else:
        deviantart.configure(str(tmp_path), CFG)
        pending = deviantart.load_pending_entries(str(path))
        result = deviantart.publish_batch(pending, [rows[0]["uuid"]], str(path))
        assert result[:2] == (0, 1)
        assert result[2]
    pw, prepare, submit = browser_stub
    prepare.assert_not_called()
    pw.firefox.launch_persistent_context.assert_not_called()
    submit.assert_not_called()
    assert path.read_bytes() == before


@pytest.mark.parametrize("title", ["", "[[PUBLICATOR:TODO_GENERATE]]"])
def test_batch_rejects_stale_queue_after_metadata_approval_withdrawn(tmp_path, browser_stub, title):
    path = tmp_path / "publications.json"
    write_da_publication(path, tmp_path)
    deviantart.configure(str(tmp_path), CFG)
    pending = deviantart.load_pending_entries(str(path))
    store.return_to_metadata(str(path), UUID, config=CFG)
    store.save_publication_metadata(str(path), UUID, title=title, config=CFG)
    before = path.read_bytes()

    result = deviantart.publish_batch(pending, [UUID], str(path))

    assert result[:2] == (0, 1)
    assert result[2]
    pw, prepare, submit = browser_stub
    prepare.assert_not_called()
    pw.firefox.launch_persistent_context.assert_not_called()
    submit.assert_not_called()
    assert path.read_bytes() == before


def test_batch_rejects_stale_queue_after_successful_delivery(tmp_path, browser_stub):
    path = tmp_path / "publications.json"
    write_da_publication(path, tmp_path)
    deviantart.configure(str(tmp_path), CFG)
    pending = deviantart.load_pending_entries(str(path))
    assert deviantart.publish_batch(pending, [UUID], str(path)) == (1, 0, None)
    assert store.load_publications(str(path))[0]["state"] == "APPEARED"
    before = path.read_bytes()
    for mock in browser_stub:
        mock.reset_mock()

    result = deviantart.publish_batch(pending, [UUID], str(path))

    assert result[:2] == (0, 1)
    assert result[2]
    pw, prepare, submit = browser_stub
    prepare.assert_not_called()
    pw.firefox.launch_persistent_context.assert_not_called()
    submit.assert_not_called()
    assert path.read_bytes() == before


def test_batch_uses_current_approved_metadata_not_stale_queue(tmp_path, browser_stub):
    path = tmp_path / "publications.json"
    write_da_publication(path, tmp_path)
    deviantart.configure(str(tmp_path), CFG)
    pending = deviantart.load_pending_entries(str(path))
    store.return_to_metadata(str(path), UUID, config=CFG)
    store.save_publication_metadata(str(path), UUID, title="Updated title",
                                    description="Updated description", config=CFG)
    store.approve_publication_metadata(str(path), UUID, config=CFG)

    assert deviantart.publish_batch(pending, [UUID], str(path)) == (1, 0, None)

    submitted = browser_stub[2].call_args.args[1]
    assert submitted["title"] == "Updated title"
    assert submitted["description"] == "Updated description"
    assert store.load_publications(str(path))[0]["state"] == "APPEARED"


def test_registry_exposes_only_deviantart_by_default():
    assert list(platforms.PLATFORMS) == ["deviantart"]
    assert platforms.PLATFORMS["deviantart"].label == "DeviantArt"


def test_publish_selected_rejects_unregistered_selected_platform(tmp_path):
    path = tmp_path / "publications.json"
    write_pubs(path, [{"platformName": "pixiv", "state": "unpublished"}])

    with pytest.raises(ValueError, match="unregistered platform: pixiv"):
        platforms.publish_selected(str(path), str(tmp_path), platform_names=["pixiv"])


def test_publish_selected_skips_early_phases_and_preserves_success_for_retry(tmp_path, monkeypatch):
    path = tmp_path / "publications.json"
    write_pubs(path, [
        {"platformName": "deviantart", "state": "unpublished"},
        {"platformName": "example", "state": "unpublished"},
    ])
    da = FakePublisher("deviantart")
    example = FakePublisher("example", fail=True)
    skill = tmp_path / "SKILL.md"
    skill.write_text("ok")
    monkeypatch.setattr(platforms, "PLATFORMS", {
        "deviantart": Platform("deviantart", "DeviantArt", da, skill),
        "example": Platform("example", "Example", example, skill),
    })

    assert platforms.publish_selected(str(path), str(tmp_path), config=CFG) == (1, 1, "example broke")
    pub = store.load_publications(str(path))[0]
    assert pub["state"] == "DESCRIBED"
    assert [a["state"] for a in pub["apparitions"]] == ["published_or_scheduled", "unpublished"]

    assert platforms.publish_selected(str(path), str(tmp_path), config=CFG) == (0, 1, "example broke")
    assert da.published == [UUID]

    rows = store.load_publications(str(path))
    rows[0]["state"] = "PICKED"
    store.atomic_write_json(str(path), rows)
    assert platforms.publish_selected(str(path), str(tmp_path), config=CFG) == (0, 0, None)
