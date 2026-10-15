import json

import pytest

from publicator import store


def test_mutate_publications_returns_result_and_persists_valid_data(tmp_path):
    path = tmp_path / "publications.json"
    path.write_text("[]")
    pub = {"uuid": "00000000-0000-0000-0000-000000000001",
           "state": "PICKED", "submissionTimestamp": 1,
           "title": "", "description": "", "files": []}
    result = store.mutate_publications(
        str(path), lambda rows: rows.append(pub), config={})
    assert result is None
    assert json.loads(path.read_text()) == [pub]


def test_mutate_publications_does_not_write_invalid_schema(tmp_path):
    from jsonschema import ValidationError
    path = tmp_path / "publications.json"
    path.write_text("[]")
    before = path.read_bytes()
    with pytest.raises(ValidationError):
        store.mutate_publications(str(path), lambda rows: rows.append({"invalid": True}), config={})
    assert path.read_bytes() == before


def test_mutate_publications_does_not_write_after_callback_error(tmp_path):
    path = tmp_path / "publications.json"
    path.write_text("[]")
    before = path.read_bytes()
    def fail(rows):
        rows.append({"invalid": True})
        raise ValueError("rejected")
    with pytest.raises(ValueError, match="rejected"):
        store.mutate_publications(str(path), fail, config={})
    assert path.read_bytes() == before
