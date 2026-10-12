import copy

import pytest

from publicator import migration
from publicator.entries import (
    PUBLICATION_STATE_APPEARED,
    PUBLICATION_STATE_DESCRIBED,
    PUBLICATION_STATE_SCHEDULED,
)


def _legacy(**overrides):
    pub = {
        "uuid": "00000000-0000-4000-8000-000000000001",
        "submissionTimestamp": 1700000000,
        "description": "d",
        "files": [{"basename": "a.png", "sha512sum": "0" * 128}],
        "apparitions": [{
            "platformName": "deviantart",
            "state": "published_or_scheduled",
            "urlElsePublicationName": "https://www.deviantart.com/user/art/title-123",
            "apparitionTimestampIfDifferentThanSubmission": 1800000000,
        }],
    }
    pub.update(overrides)
    return pub


def test_migrate_legacy_publication_moves_url_date_and_recovers_title():
    src = [_legacy()]
    out = migration.migrate_publications(src, {
        "00000000-0000-4000-8000-000000000001": "Recovered Title",
    }, {"tiers": [], "galleries": []})

    assert src[0]["apparitions"][0]["urlElsePublicationName"].startswith("https://")
    pub = out[0]
    assert pub["title"] == "Recovered Title"
    assert pub["state"] == PUBLICATION_STATE_APPEARED
    assert pub["apparitionTimestampIfDifferentThanSubmission"] == 1800000000
    assert pub["apparitions"][0]["url"] == "https://www.deviantart.com/user/art/title-123"
    assert "urlElsePublicationName" not in pub["apparitions"][0]
    assert "apparitionTimestampIfDifferentThanSubmission" not in pub["apparitions"][0]


def test_migration_is_idempotent_and_preserves_new_drafts():
    draft = {
        "uuid": "00000000-0000-4000-8000-000000000002",
        "state": "PICKED",
        "title": "",
        "submissionTimestamp": 1700000000,
        "description": "",
        "files": [{"basename": "a.png", "sha512sum": "0" * 128}],
        "apparitions": [],
    }
    first = migration.migrate_publications([draft], {}, {"tiers": [], "galleries": []})
    second = migration.migrate_publications(first, {}, {"tiers": [], "galleries": []})
    assert first == second == [draft]


def test_migration_fails_unknown_titles_without_mutating_input():
    src = [_legacy()]
    before = copy.deepcopy(src)

    with pytest.raises(ValueError, match="recovered title"):
        migration.migrate_publications(src, {}, {"tiers": [], "galleries": []})

    assert src == before


def test_migration_fails_conflicting_names_and_dates_without_mutating_input():
    src = [_legacy(apparitions=[
        {"platformName": "deviantart", "state": "unpublished",
         "urlElsePublicationName": "Name A", "apparitionTimestampIfDifferentThanSubmission": 1800000000},
        {"platformName": "pixiv", "state": "unpublished",
         "urlElsePublicationName": "Name B", "apparitionTimestampIfDifferentThanSubmission": 1800000600},
    ])]
    before = copy.deepcopy(src)

    with pytest.raises(ValueError, match="conflicting legacy titles"):
        migration.migrate_publications(src, {}, {"tiers": [], "galleries": []})

    assert src == before

    src[0]["apparitions"][1]["urlElsePublicationName"] = "Name A"
    before = copy.deepcopy(src)
    with pytest.raises(ValueError, match="conflicting apparition dates"):
        migration.migrate_publications(src, {}, {"tiers": [], "galleries": []})
    assert src == before


def test_migration_infers_described_for_pending_legacy_with_metadata():
    pub = _legacy(apparitions=[{
        "platformName": "deviantart",
        "state": "unpublished",
        "urlElsePublicationName": "Title",
        "apparitionTimestampIfDifferentThanSubmission": 1800000000,
    }])

    out = migration.migrate_publications([pub], {}, {"tiers": [], "galleries": []})

    assert out[0]["state"] == PUBLICATION_STATE_DESCRIBED
    assert out[0]["title"] == "Title"


@pytest.mark.parametrize("field", ["title", "description"])
@pytest.mark.parametrize("marked", [
    f" {migration.TODO_GENERATE} ",
    f"Before {migration.TODO_GENERATE} after",
])
def test_migration_keeps_marker_bearing_pending_metadata_scheduled(field, marked):
    fields = {"title": "Title", "description": "D", field: marked}
    pub = _legacy(**fields, apparitions=[{
        "platformName": "deviantart",
        "state": "unpublished",
        "urlElsePublicationName": fields["title"],
        "apparitionTimestampIfDifferentThanSubmission": 1800000000,
    }])
    src = [pub]
    before = copy.deepcopy(src)

    out = migration.migrate_publications(src, {}, {"tiers": [], "galleries": []})

    assert out[0]["state"] == PUBLICATION_STATE_SCHEDULED
    assert out[0][field] == marked
    assert src == before


@pytest.mark.parametrize("old_name", ["", None])
def test_migration_recovers_all_empty_legacy_titles_or_fails_without_mutation(old_name):
    pub = _legacy()
    if old_name is None:
        pub["apparitions"][0].pop("urlElsePublicationName")
    else:
        pub["apparitions"][0]["urlElsePublicationName"] = old_name
    src = [pub]
    before = copy.deepcopy(src)
    with pytest.raises(ValueError, match="recovered title"):
        migration.migrate_publications(src, {}, {"tiers": [], "galleries": []})
    assert src == before
    out = migration.migrate_publications(src, {pub["uuid"]: "Recovered"}, {})
    assert out[0]["title"] == "Recovered"
    assert out[0]["state"] == PUBLICATION_STATE_APPEARED
    assert src == before


@pytest.mark.parametrize("state", ["PICKED", "SCHEDULED"])
def test_migration_preserves_only_already_new_intentionally_blank_drafts(state):
    draft = _legacy(state=state, title="", apparitions=[])
    assert migration.migrate_publications([draft], {draft["uuid"]: "Not a draft title"}, {}) == [draft]
    legacy = _legacy(state=state, title="")
    legacy["apparitions"][0]["urlElsePublicationName"] = ""
    with pytest.raises(ValueError, match="recovered title"):
        migration.migrate_publications([legacy], {}, {})
