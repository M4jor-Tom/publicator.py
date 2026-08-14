import json, subprocess, pytest
from types import SimpleNamespace
from publicator import llm_meta


def make_run(stdout="", returncode=0, stderr="", capture=None):
    def run(argv, stdin):
        if capture is not None:
            capture["argv"], capture["stdin"] = argv, stdin
        return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)
    return run


def test_run_llm_builds_argv_and_parses_schema():
    cap = {}
    obj = llm_meta.run_llm("m", "hi", schema={"type": "object"}, cwd="/w",
                           run=make_run(stdout='{"a": 1}', capture=cap))
    assert obj == {"a": 1}
    argv = cap["argv"]
    assert argv[:3] == ["llm", "-m", "m"]
    assert "allowedTools" in argv and "Read" in argv   # vision via Read tool, not attachment
    assert "cwd" in argv and "/w" in argv
    assert "--schema" in argv
    assert cap["stdin"] == "hi"


def test_run_llm_nonzero_raises():
    with pytest.raises(RuntimeError, match="llm failed"):
        llm_meta.run_llm("m", "hi", cwd="/w", run=make_run(returncode=1, stderr="boom"))


def test_run_llm_bad_json_with_schema_raises():
    with pytest.raises(RuntimeError, match="non-JSON"):
        llm_meta.run_llm("m", "hi", schema={"type": "object"}, cwd="/w",
                         run=make_run(stdout="not json"))


def test_run_llm_timeout_raises():
    def run(argv, stdin):
        raise subprocess.TimeoutExpired(argv, 90)
    with pytest.raises(RuntimeError, match="timed out"):
        llm_meta.run_llm("m", "hi", cwd="/w", run=run)


def test_generate_metadata_returns_title_desc():
    cap = {}
    title, desc = llm_meta.generate_metadata(
        "/imgs/thumb.jpg",
        run=make_run(stdout='{"title": "Dawn", "description": "A quiet field."}', capture=cap))
    assert title == "Dawn" and desc == "A quiet field."
    # image reaches the model as a Read-able path in its own dir, not an attachment
    assert "thumb.jpg" in cap["stdin"]
    i = cap["argv"].index("cwd")
    assert cap["argv"][i + 1] == "/imgs"
    j = cap["argv"].index("--schema")
    assert json.loads(cap["argv"][j + 1]) == llm_meta.TITLE_DESC_SCHEMA


def test_generate_metadata_empty_raises():
    with pytest.raises(RuntimeError, match="missing title/description"):
        llm_meta.generate_metadata("/imgs/x.jpg",
                                    run=make_run(stdout='{"title": "", "description": "x"}'))


def test_generate_metadata_openrouter_uses_attachment():
    cap = {}
    title, desc = llm_meta.generate_metadata(
        "/imgs/thumb.jpg", model="openrouter/google/gemini-2.0-flash-exp:free",
        run=make_run(stdout='{"title": "Dawn", "description": "A quiet field."}', capture=cap))
    assert title == "Dawn" and desc == "A quiet field."
    argv = cap["argv"]
    assert argv[:3] == ["llm", "-m", "openrouter/google/gemini-2.0-flash-exp:free"]
    i = argv.index("-a")
    assert argv[i + 1].endswith("thumb.jpg")      # image as attachment, absolute path
    assert "allowedTools" not in argv and "-o" not in argv  # no claude-cli Read path
    j = argv.index("--schema")
    assert json.loads(argv[j + 1]) == llm_meta.TITLE_DESC_SCHEMA
    assert "attached" in cap["stdin"].lower()      # attachment prompt, not "read the file"
