import json
import threading
from uuid import NAMESPACE_URL, uuid5

import pytest

from publicator import store
from publicator.apps import generate_meta
from publicator.config import load_config, validate_publications
from publicator.llm_meta import DEFAULT_MODEL, OPENROUTER_MODEL, VISION_MODELS


MARKER = generate_meta.TODO_GENERATE


def publication(uuid, basename, *, description=MARKER, title=MARKER, apparitions=None):
    return {
        "uuid": str(uuid5(NAMESPACE_URL, uuid)),
        "submissionTimestamp": 1,
        "apparitionTimestampIfDifferentThanSubmission": 1800000000,
        "state": "SCHEDULED",
        "title": title,
        "description": description,
        "files": [{"basename": basename}],
        "apparitions": [] if apparitions is None else apparitions,
    }


def batch(tmp_path, monkeypatch, pubs, generated=None, *, source="openrouter", yes=False):
    json_path = tmp_path / "publications.json"
    json_path.write_text(json.dumps(pubs))
    monkeypatch.setattr(generate_meta.entries, "find_art_path", lambda basename: tmp_path / basename)
    monkeypatch.setattr(generate_meta, "thumb_for_ai", lambda path, cache: f"thumb:{path}")
    monkeypatch.setattr(generate_meta, "generate_metadata", generated or (lambda *a, **k: ("title", "description")))
    model = next(m for m in VISION_MODELS if m["openrouter_name"] == OPENROUTER_MODEL) if source == "openrouter" else None
    return generate_meta.generate_pending(str(json_path), str(tmp_path), source=source,
                                          model_option=model, yes=yes), json_path


def test_only_exact_top_level_markers_are_replaced_without_apparitions(tmp_path, monkeypatch):
    pubs = [publication("marked", "a.png"),
            publication("near", "b.png", description=MARKER + " ", title="todo_generate"),
            publication("plain", "c.png", description="keep", title="keep")]
    (counts, path) = batch(tmp_path, monkeypatch, pubs)
    result = json.loads(path.read_text())
    assert counts == (1, 0)
    assert result[0]["title"] == "title"
    assert result[0]["description"] == "description"
    assert result[1]["description"] == MARKER + " "
    assert result[1]["title"] == "todo_generate"
    assert result[2] == pubs[2]
    assert result[0]["state"] == "SCHEDULED"
    validate_publications(result, load_config(tmp_path))


def test_generation_preserves_edit_during_provider_call(tmp_path, monkeypatch):
    pub = publication("one", "a.png")
    path = tmp_path / "publications.json"

    def generated(*args, **kwargs):
        store.save_publication_metadata(str(path), pub["uuid"], title="Manual title",
                                        config=load_config(tmp_path))
        return "Generated title", "Generated description"

    counts, path = batch(tmp_path, monkeypatch, [pub], generated)
    assert counts == (1, 0)
    saved = store.load_publications(str(path))[0]
    assert saved["title"] == "Manual title"
    assert saved["description"] == "Generated description"
    assert saved["state"] == "SCHEDULED"


def test_generation_preserves_concurrent_store_edit_before_replacement(tmp_path, monkeypatch):
    pub = publication("one", "a.png")
    path = tmp_path / "publications.json"
    original_replace = store.os.replace
    generating_thread = threading.current_thread()
    editor_started = threading.Event()
    editor_done = threading.Event()
    errors = []
    editors = []

    def edit():
        editor_started.set()
        try:
            store.save_publication_metadata(str(path), pub["uuid"], description="Manual description",
                                            config=load_config(tmp_path))
        except Exception as error:
            errors.append(error)
        finally:
            editor_done.set()

    def replace(source, destination):
        if threading.current_thread() is generating_thread and str(destination) == str(path):
            editor = threading.Thread(target=edit)
            editors.append(editor)
            editor.start()
            assert editor_started.wait(5)
            # Without the shared mutation lock, the edit commits before this stale replacement.
            editor_done.wait(1)
        return original_replace(source, destination)

    monkeypatch.setattr(store.os, "replace", replace)
    try:
        counts, path = batch(tmp_path, monkeypatch, [pub])
    finally:
        for editor in editors:
            editor.join(5)
    assert editors and all(not editor.is_alive() for editor in editors)
    assert not errors
    assert counts == (1, 0)
    saved = store.load_publications(str(path))[0]
    assert saved["title"] == "title"
    assert saved["description"] == "Manual description"
    assert saved["state"] == "SCHEDULED"


@pytest.mark.parametrize("invalid_field", ["uuid", "submissionTimestamp"])
def test_generation_rejects_invalid_database_without_replacement(tmp_path, monkeypatch, invalid_field):
    pub = publication("one", "a.png")
    if invalid_field == "uuid":
        pub["uuid"] = "short"
    else:
        pub.pop(invalid_field)
    counts, path = batch(tmp_path, monkeypatch, [pub])
    assert counts == (0, 1)
    assert json.loads(path.read_text()) == [pub]


@pytest.mark.parametrize("gallery, expected", [("Allowed", (1, 0)), ("Not allowed", (0, 1))])
def test_generation_validates_using_data_directory_config(tmp_path, monkeypatch, gallery, expected):
    (tmp_path / "publicator.toml").write_text('[deviantart]\ngalleries = ["Allowed"]\n')
    pub = publication("one", "a.png", apparitions=[{
        "platformName": "deviantart", "state": "unpublished", "galleries": [gallery],
    }])
    counts, path = batch(tmp_path, monkeypatch, [pub])
    assert counts == expected
    saved = store.load_publications(str(path))
    if expected == (1, 0):
        validate_publications(saved, load_config(tmp_path))
    else:
        assert saved == [pub]


def test_success_is_written_before_next_record_and_failures_continue(tmp_path, monkeypatch):
    pubs = [publication("one", "one.png"), publication("two", "two.png"), publication("three", "three.png")]
    json_path = tmp_path / "publications.json"
    json_path.write_text(json.dumps(pubs))
    monkeypatch.setattr(generate_meta.entries, "find_art_path", lambda name: tmp_path / name)
    monkeypatch.setattr(generate_meta, "thumb_for_ai", lambda path, cache: str(path))
    calls = []

    def generated(path, model, **kwargs):
        calls.append(path)
        saved = json.loads(json_path.read_text())
        assert saved[0]["description"] == "description" if len(calls) > 1 else True
        if path.endswith("two.png"):
            raise RuntimeError("generation broke")
        return "title", "description"

    monkeypatch.setattr(generate_meta, "generate_metadata", generated)
    assert generate_meta.generate_pending(str(json_path), str(tmp_path), source="openrouter",
        model_option=next(m for m in VISION_MODELS if m["openrouter_name"] == OPENROUTER_MODEL)) == (2, 1)
    result = json.loads(json_path.read_text())
    assert [p["description"] for p in result] == ["description", MARKER, "description"]
    assert [p["title"] for p in result] == ["title", MARKER, "title"]


def test_lookup_failure_retains_markers_and_batch_returns_failure(tmp_path, monkeypatch):
    pubs = [publication("bad", "bad.png")]
    json_path = tmp_path / "publications.json"
    json_path.write_text(json.dumps(pubs))
    monkeypatch.setattr(generate_meta.entries, "find_art_path", lambda _: (_ for _ in ()).throw(SystemExit("missing")))
    assert generate_meta.generate_pending(str(json_path), str(tmp_path), source="claude", model_option=None) == (0, 1)
    assert json.loads(json_path.read_text()) == pubs


def test_no_markers_skips_local_preflight(tmp_path, monkeypatch):
    pubs = [publication("plain", "a.png", description="x", title="y")]
    result, path = batch(tmp_path, monkeypatch, pubs, source="local")
    assert result == (0, 0)
    assert json.loads(path.read_text()) == pubs


def test_local_missing_model_consent_is_asked_once_and_refusal_changes_nothing(tmp_path, monkeypatch):
    pubs = [publication("one", "a.png"), publication("two", "b.png")]
    json_path = tmp_path / "publications.json"
    original = json.dumps(pubs)
    json_path.write_text(original)
    local = VISION_MODELS[0]
    monkeypatch.setattr(generate_meta.entries, "find_art_path", lambda name: tmp_path / name)
    monkeypatch.setattr(generate_meta, "local_model_status", lambda option: {"fit": True, "installed": False, "available": True, "reason": ""})
    prompts = []
    monkeypatch.setattr(generate_meta.sys.stdin, "isatty", lambda: True)
    monkeypatch.setattr("builtins.input", lambda prompt: prompts.append(prompt) or "n")
    assert generate_meta.generate_pending(str(json_path), str(tmp_path), source="local", model_option=local) == (0, 2)
    assert len(prompts) == 1
    assert json_path.read_text() == original


def test_local_install_consent_empty_accepts_and_installed_does_not_prompt(tmp_path, monkeypatch):
    pub = [publication("one", "a.png")]
    local = VISION_MODELS[0]
    statuses = iter([{"fit": True, "installed": False, "available": True, "reason": ""},
                     {"fit": True, "installed": True, "available": True, "reason": ""}])
    monkeypatch.setattr(generate_meta, "local_model_status", lambda option: next(statuses))
    monkeypatch.setattr(generate_meta.entries, "find_art_path", lambda name: tmp_path / name)
    monkeypatch.setattr(generate_meta, "thumb_for_ai", lambda path, cache: str(path))
    monkeypatch.setattr(generate_meta, "generate_metadata", lambda *a, **k: ("T", "D"))
    prompts = []
    monkeypatch.setattr(generate_meta.sys.stdin, "isatty", lambda: True)
    monkeypatch.setattr("builtins.input", lambda prompt: prompts.append(prompt) or "")
    for _ in range(2):
        path = tmp_path / f"{_}.json"
        path.write_text(json.dumps(pub))
        assert generate_meta.generate_pending(str(path), str(tmp_path), source="local", model_option=local) == (1, 0)
    assert len(prompts) == 1


def test_local_fit_failure_and_no_tty_abort_before_writes(tmp_path, monkeypatch):
    pub = [publication("one", "a.png")]
    path = tmp_path / "publications.json"
    original = json.dumps(pub)
    path.write_text(original)
    local = VISION_MODELS[0]
    monkeypatch.setattr(generate_meta, "local_model_status", lambda option: {"fit": False, "installed": False, "available": False, "reason": "Too Tight"})
    assert generate_meta.generate_pending(str(path), str(tmp_path), source="local", model_option=local) == (0, 1)
    assert path.read_text() == original
    monkeypatch.setattr(generate_meta, "local_model_status", lambda option: {"fit": True, "installed": False, "available": True, "reason": ""})
    monkeypatch.setattr(generate_meta.sys.stdin, "isatty", lambda: False)
    assert generate_meta.generate_pending(str(path), str(tmp_path), source="local", model_option=local) == (0, 1)
    assert path.read_text() == original


def test_yes_bypasses_input_and_cli_is_publication_driven(tmp_path, monkeypatch):
    pub = [publication("one", "a.png")]
    path = tmp_path / "publications.json"
    path.write_text(json.dumps(pub))
    monkeypatch.setattr(generate_meta, "local_model_status", lambda option: {"fit": True, "installed": False, "available": True, "reason": ""})
    monkeypatch.setattr(generate_meta.sys.stdin, "isatty", lambda: False)
    monkeypatch.setattr("builtins.input", lambda *_: pytest.fail("-y must not read stdin"))
    monkeypatch.setattr(generate_meta.entries, "find_art_path", lambda name: tmp_path / name)
    monkeypatch.setattr(generate_meta, "thumb_for_ai", lambda path, cache: str(path))
    monkeypatch.setattr(generate_meta, "generate_metadata", lambda *a, **k: ("T", "D"))
    assert generate_meta.generate_pending(str(path), str(tmp_path), source="local", model_option=VISION_MODELS[0], yes=True) == (1, 0)
    with pytest.raises(SystemExit):
        generate_meta.main_args(["IMAGE.png", "--source", "openrouter"])


def test_cli_requires_source_and_local_model_but_keeps_defaults(monkeypatch):
    with pytest.raises(SystemExit):
        generate_meta.parse_args([])
    with pytest.raises(SystemExit):
        generate_meta.parse_args(["--source", "local"])
    args = generate_meta.parse_args(["--source", "openrouter"])
    assert args.model_option["openrouter_name"] == OPENROUTER_MODEL
    assert generate_meta.parse_args(["--source", "claude"]).model_option is None
    assert DEFAULT_MODEL
