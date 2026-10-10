import json
from pathlib import Path

import pytest

from publicator import deviantart, entries
from publicator.deviantart import STEPS, check_steps, configure, parse_schedule


@pytest.mark.parametrize("s,expected", [
    ("Tue Sep 8 08:00:00 PM CEST 2026", (2026, 9, 8, 20)),
    ("Wed Jan 1 12:00:00 AM UTC 2025", (2025, 1, 1, 0)),
    ("Wed Jan 1 12:00:00 PM UTC 2025", (2025, 1, 1, 12)),
])
def test_parse_schedule(s, expected):
    assert parse_schedule(s) == expected


def test_parse_schedule_rejects_garbage():
    with pytest.raises(ValueError):
        parse_schedule("not a date")


def test_publish_batch_refuses_unresolved_ai_metadata_before_starting_browser(monkeypatch):
    from publicator.apps.generate_meta import TODO_GENERATE

    calls = []
    monkeypatch.setattr(deviantart, "_prepare_session_from_login",
                        lambda: calls.append("session"))
    result = deviantart.publish_batch(
        [{"title": TODO_GENERATE, "description": TODO_GENERATE}], ["u"], "unused.json")
    assert result[0:2] == (0, 1)
    assert "AI metadata" in result[2]
    assert calls == []


def test_steps_map_to_callables():
    assert STEPS and all(callable(fn) for _, fn in STEPS)


def test_steps_stay_in_sync_with_the_skill():
    assert check_steps()


class _NoPage:
    def __getattr__(self, name):
        raise AssertionError(f"step touched the page ({name}) when it should no-op")


def test_optional_steps_are_no_ops_when_unset():
    deviantart._step_premium(_NoPage(), {"price": None})
    deviantart._step_tier(_NoPage(), {})
    deviantart._step_galleries(_NoPage(), {})


def test_step_tier_selects_every_requested_tier(monkeypatch):
    selected = []

    class Combo:
        def scroll_into_view_if_needed(self):
            pass

    class Section:
        def get_by_text(self, *_args, **_kwargs):
            return self

        def locator(self, _selector):
            return Combo()

    monkeypatch.setattr(deviantart, "_pick_in_combo",
                        lambda _page, _combo, tier: selected.append(tier))
    deviantart._step_tier(Section(), {"tiers": ["gold", "silver"]})
    assert selected == ["gold", "silver"]


def test_load_pending_entries_normalizes_legacy_and_multiple_tiers(tmp_path):
    configure(str(tmp_path), {})
    (tmp_path / "multi-tier-art.png").write_bytes(b"art")
    rows = []
    for uuid, fields in (("legacy", {"tier": "gold"}),
                         ("multiple", {"tiers": ["gold", "silver"]})):
        rows.append({"uuid": uuid, "description": "d", "files": [{"basename": "multi-tier-art.png"}],
                     "apparitions": [{"platformName": "deviantart", "state": "unpublished",
                                      "urlElsePublicationName": uuid, **fields}]})
    json_path = tmp_path / "publications.json"
    json_path.write_text(json.dumps(rows))
    pending = deviantart.load_pending_entries(str(json_path))
    assert [entry["tiers"] for entry in pending] == [["gold"], ["gold", "silver"]]


def test_configure_resolves_tags_file_under_data_dir(tmp_path):
    configure(str(tmp_path), {"tags": "sub/tags.txt"})
    assert deviantart.TAGS_FILE == Path(tmp_path) / "sub/tags.txt"


def test_configure_leaves_tags_file_unset_without_config(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart.TAGS_FILE is None


def test_login_cookies_empty_without_profile_db(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart._da_login_cookies() == []
