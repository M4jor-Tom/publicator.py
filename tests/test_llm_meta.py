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


def test_run_llm_retries_without_schema_then_remembers_the_model(monkeypatch):
    monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())   # module state: don't leak between tests
    calls = []

    def run(argv, stdin):
        calls.append((argv, stdin))
        if "--schema" in argv:
            return SimpleNamespace(returncode=1, stdout="",
                                   stderr=f"Error: OpenRouter: m {llm_meta.NO_SCHEMA_SUPPORT}\n")
        return SimpleNamespace(returncode=0, stdout='```json\n{"a": 1}\n```', stderr="")

    call = lambda: llm_meta.run_llm("m", "hi", schema={"type": "object"}, attach="/i.png", run=run)
    assert call() == {"a": 1}
    assert len(calls) == 2
    assert "--schema" not in calls[1][0]              # retry drops the rejected flag
    assert '"type": "object"' in calls[1][1]          # ...and puts the shape in the prompt

    # schema support is static per model, so the next image doesn't re-pay the spawn
    assert call() == {"a": 1}
    assert len(calls) == 3 and "--schema" not in calls[2][0]


def test_run_llm_other_failure_is_not_retried():
    calls = []

    def run(argv, stdin):
        calls.append(argv)
        return SimpleNamespace(returncode=1, stdout="", stderr="Error code: 429 rate-limited")

    with pytest.raises(RuntimeError, match="429"):
        llm_meta.run_llm("m", "hi", schema={"type": "object"}, attach="/i.png", run=run)
    assert len(calls) == 1


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
