from pathlib import Path

import pytest

from publicator import deviantart
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


def test_configure_resolves_tags_file_under_data_dir(tmp_path):
    configure(str(tmp_path), {"tags": "sub/tags.txt"})
    assert deviantart.TAGS_FILE == Path(tmp_path) / "sub/tags.txt"


def test_configure_leaves_tags_file_unset_without_config(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart.TAGS_FILE is None


def test_login_cookies_empty_without_profile_db(tmp_path):
    configure(str(tmp_path), {})
    assert deviantart._da_login_cookies() == []
