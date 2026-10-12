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


def test_publish_batch_refuses_invalid_metadata_before_starting_browser(monkeypatch, tmp_path):
    from publicator.apps.generate_meta import TODO_GENERATE
    from publicator import store

    uuid_ = "00000000-0000-0000-0000-000000000001"
    path = tmp_path / "publications.json"
    (tmp_path / "art.png").write_bytes(b"art")
    configure(str(tmp_path), {})
    pub = {"uuid": uuid_, "state": "DESCRIBED", "title": "Approved title",
           "description": "Approved description", "submissionTimestamp": 1,
           "files": [{"basename": "art.png"}],
           "apparitions": [{"platformName": "deviantart", "state": "unpublished"}]}
    store.atomic_write_json(str(path), [pub])
    pending = deviantart.load_pending_entries(str(path))
    calls = []
    def refuse_session():
        calls.append("session")
        raise AssertionError("invalid stored metadata reached session preparation")

    monkeypatch.setattr(deviantart, "_prepare_session_from_login", refuse_session)
    for entry, error in [
        ({"title": TODO_GENERATE, "description": "D"}, "AI metadata"),
        ({"title": "T", "description": TODO_GENERATE}, "AI metadata"),
        ({"title": "x" * 51, "description": "D"}, "title too long"),
        ({"title": "", "description": "D"}, "title and description required"),
    ]:
        store.atomic_write_json(str(path), [{**pub, **entry}])
        result = deviantart.publish_batch(pending, [uuid_], str(path))
        assert result[0:2] == (0, 1)
        assert error in result[2]
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


def test_load_pending_entries_uses_shared_metadata_and_skips_ineligible_rows(tmp_path):
    configure(str(tmp_path), {})
    (tmp_path / "multi-tier-art.png").write_bytes(b"art")
    rows = [
        {"uuid": "picked", "state": "PICKED", "title": "early", "description": "d",
         "submissionTimestamp": 1, "files": [{"basename": "multi-tier-art.png"}],
         "apparitions": [{"platformName": "deviantart", "state": "unpublished"}]},
        {"uuid": "pixiv", "state": "DESCRIBED", "title": "other", "description": "d",
         "submissionTimestamp": 1, "files": [{"basename": "multi-tier-art.png"}],
         "apparitions": [{"platformName": "pixiv", "state": "unpublished"}]},
    ]
    for uuid, fields in (("legacy", {"tier": "gold"}),
                         ("multiple", {"tiers": ["gold", "silver"]})):
        rows.append({"uuid": uuid, "state": "DESCRIBED", "title": uuid, "description": "d",
                     "submissionTimestamp": 1,
                     "apparitionTimestampIfDifferentThanSubmission": 1800000000,
                     "files": [{"basename": "multi-tier-art.png"}],
                     "apparitions": [{"platformName": "deviantart", "state": "unpublished", **fields}]})
    json_path = tmp_path / "publications.json"
    json_path.write_text(json.dumps(rows))
    pending = deviantart.load_pending_entries(str(json_path))
    assert [(entry["uuid"], entry["title"], entry["scheduleTs"], entry["tiers"])
            for entry in pending] == [
        ("legacy", "legacy", 1800000000, ["gold"]),
        ("multiple", "multiple", 1800000000, ["gold", "silver"]),
    ]


def test_echo_first_uses_shared_title_date_and_skips_other_platforms(tmp_path, monkeypatch, capsys):
    from publicator.apps import echo_first

    (tmp_path / "art.png").write_bytes(b"art")
    rows = [
        {"uuid": "pixiv", "state": "DESCRIBED", "title": "Other", "description": "d",
         "submissionTimestamp": 1, "files": [{"basename": "art.png"}],
         "apparitions": [{"platformName": "pixiv", "state": "unpublished"}]},
        {"uuid": "da", "state": "DESCRIBED", "title": "Shared Title", "description": "d",
         "submissionTimestamp": 1, "apparitionTimestampIfDifferentThanSubmission": 1800000000,
         "files": [{"basename": "art.png"}],
         "apparitions": [{"platformName": "deviantart", "state": "unpublished"}]},
    ]
    (tmp_path / "publications.json").write_text(json.dumps(rows))
    monkeypatch.setattr("sys.argv", ["echo-first", "--data-dir", str(tmp_path)])
    monkeypatch.setattr(echo_first, "format_schedule", lambda ts: f"schedule:{ts}")

    assert echo_first.main() == 0
    out = capsys.readouterr().out
    assert "title: Shared Title" in out
    assert "schedule: schedule:1800000000" in out


def test_configure_resolves_tags_file_under_data_dir(tmp_path):
    configure(str(tmp_path), {"tags": "sub/tags.txt"})
    assert deviantart.TAGS_FILE == Path(tmp_path) / "sub/tags.txt"


def test_configure_leaves_tags_file_unset_without_config(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart.TAGS_FILE is None


def test_login_cookies_empty_without_profile_db(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart._da_login_cookies() == []


@pytest.mark.parametrize('suffix', ['.WEBP', '.WeBp'])
def test_resolve_art_never_uses_case_variant_webp_as_uploadable_sibling(tmp_path, monkeypatch, suffix):
    data_dir = tmp_path / 'data'; data_dir.mkdir()
    conversion_dir = tmp_path / 'converted'; conversion_dir.mkdir()
    source = data_dir / f'art{suffix}'; source.write_bytes(b'webp')
    (data_dir / 'art-alternative.WEbP').write_bytes(b'another webp')
    configure(str(data_dir), {})
    monkeypatch.setattr(deviantart.tempfile, 'gettempdir', lambda: str(conversion_dir))
    calls = []
    def convert(argv, *, check):
        calls.append((argv, check))
        Path(argv[-1]).write_bytes(b'stub converted png')
    monkeypatch.setattr(deviantart.subprocess, 'run', convert)

    with pytest.raises(SystemExit, match='no non-webp file'):
        entries.find_art_path(source.name)
    converted = conversion_dir / 'art.png'
    assert deviantart._resolve_art(str(source)) == str(converted)
    assert calls == [(['convert', str(source), str(converted)], True)]
    assert converted.read_bytes() == b'stub converted png'

    sibling = data_dir / 'art.png'; sibling.write_bytes(b'existing png')
    assert entries.find_art_path(source.name) == sibling.resolve()
    assert deviantart._resolve_art(str(source)) == str(sibling.resolve())
    assert len(calls) == 1  # existing non-webp sibling still avoids conversion
