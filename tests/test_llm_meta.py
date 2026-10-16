import base64, json, subprocess, pytest
from types import SimpleNamespace
from publicator import llm_meta

GEMMA = "openrouter/google/gemma-4-26b-a4b-it:free"
GEMMA_31 = "openrouter/google/gemma-4-31b-it:free"
NVIDIA = "openrouter/nvidia/nemotron-3-nano-omni:free"
RL = "Error: Error code: 429 - temporarily rate-limited upstream"


def listing(ids):
    """`llm openrouter models --free --json` stdout, listing `ids` as vision models."""
    return json.dumps([{"id": i.removeprefix("openrouter/"),
                        "architecture": {"input_modalities": ["text", "image"]}}
                       for i in ids])


def with_listing(ids, then):
    """A run= stub answering the model listing from `ids`, delegating every other call."""
    def run(argv, stdin):
        if argv[:2] == ["llm", "openrouter"]:
            return SimpleNamespace(returncode=0, stdout=listing(ids), stderr="")
        return then(argv, stdin)
    return run


def make_run(stdout="", returncode=0, stderr="", capture=None):
    def run(argv, stdin):
        if capture is not None:
            capture["argv"], capture["stdin"] = argv, stdin
        return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)
    return run


def test_run_llm_prefers_batch_model_and_reports_successful_candidate():
    from types import SimpleNamespace
    calls = []
    def run(argv, prompt):
        calls.append(argv)
        return SimpleNamespace(returncode=0, stdout='{"title":"T","description":"D"}', stderr='')
    used = []
    result = llm_meta.run_llm("openrouter/selected", "prompt", schema=llm_meta.TITLE_DESC_SCHEMA,
        attach="/image.png", run=run, preferred_model="openrouter/preferred", model_used=used)
    assert result == {"title": "T", "description": "D"}
    assert calls[0][2] == "openrouter/preferred"
    assert used == ["openrouter/preferred"]


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


@pytest.mark.parametrize("failure", ["nonzero", "timeout", "invalid-json"])
def test_run_llm_retries_after_any_candidate_failure(failure, monkeypatch):
    monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())
    tried = []

    def attempt(argv, stdin):
        tried.append(argv[2])
        if argv[2] != GEMMA:
            return SimpleNamespace(returncode=0, stdout='{"a": 1}', stderr="")
        if failure == "timeout":
            raise subprocess.TimeoutExpired(argv, 1)
        if failure == "invalid-json":
            return SimpleNamespace(returncode=0, stdout="not json", stderr="")
        return SimpleNamespace(returncode=1, stdout="", stderr="Unknown model: stale id")

    result = llm_meta.run_llm(
        GEMMA, "hi", schema={"type": "object"}, attach="/i.png",
        run=with_listing([GEMMA, NVIDIA], attempt))
    assert result == {"a": 1}
    assert tried == [GEMMA, NVIDIA]


@pytest.mark.parametrize("stderr", [
    "Error code: 503 - unavailable", "Connection error: connection refused",
])
def test_run_llm_provider_errors_raise_provider_unavailable(stderr):
    with pytest.raises(llm_meta.ProviderUnavailable, match="llm failed"):
        llm_meta.run_llm("m", "hi", cwd="/w",
                         run=make_run(returncode=1, stderr=stderr))


@pytest.mark.parametrize("stderr", ["Unknown model: m", "Error code: 401 - missing API key"])
def test_run_llm_configuration_errors_are_not_provider_unavailable(stderr):
    with pytest.raises(RuntimeError) as error:
        llm_meta.run_llm("m", "hi", cwd="/w",
                         run=make_run(returncode=1, stderr=stderr))
    assert type(error.value) is RuntimeError


def test_run_llm_bad_json_with_schema_raises():
    with pytest.raises(RuntimeError, match="non-JSON"):
        llm_meta.run_llm("m", "hi", schema={"type": "object"}, cwd="/w",
                         run=make_run(stdout="not json"))


def test_run_llm_timeout_raises_provider_unavailable():
    def run(argv, stdin):
        raise subprocess.TimeoutExpired(argv, 90)
    with pytest.raises(llm_meta.ProviderUnavailable, match="timed out"):
        llm_meta.run_llm("m", "hi", cwd="/w", run=run)


def test_run_llm_command_start_error_is_not_provider_unavailable():
    def run(argv, stdin):
        raise FileNotFoundError("llm")
    with pytest.raises(RuntimeError) as error:
        llm_meta.run_llm("m", "hi", cwd="/w", run=run)
    assert type(error.value) is RuntimeError


def test_generate_metadata_returns_title_desc():
    cap = {}
    title, desc = llm_meta.generate_metadata(
        "/imgs/thumb.jpg", None, source="claude",
        run=make_run(stdout='{"title": "Dawn", "description": "A quiet field."}', capture=cap))
    assert title == "Dawn" and desc == "A quiet field."
    # image reaches the model as a Read-able path in its own dir, not an attachment
    assert "thumb.jpg" in cap["stdin"]
    assert cap["argv"][:3] == ["llm", "-m", llm_meta.DEFAULT_MODEL]
    i = cap["argv"].index("cwd")
    assert cap["argv"][i + 1] == "/imgs"
    j = cap["argv"].index("--schema")
    assert json.loads(cap["argv"][j + 1]) == llm_meta.TITLE_DESC_SCHEMA


def test_generate_metadata_uses_configured_claude_model():
    cap = {}
    llm_meta.generate_metadata(
        "/imgs/thumb.jpg", None, source="claude", claude_model="claude-custom-opus",
        run=make_run(stdout='{"title": "Dawn", "description": "A quiet field."}', capture=cap))
    assert cap["argv"][:3] == ["llm", "-m", "claude-custom-opus"]


def test_generate_metadata_empty_raises():
    with pytest.raises(RuntimeError, match="missing title/description"):
        llm_meta.generate_metadata("/imgs/x.jpg", None, source="claude",
                                    run=make_run(stdout='{"title": "", "description": "x"}'))


def test_openrouter_falls_back_when_model_returns_empty_metadata(monkeypatch):
    monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())
    tried = []

    def attempt(argv, stdin):
        model = argv[2]
        tried.append(model)
        metadata = ({"title": "", "description": ""} if model == GEMMA else
                    {"title": "Fallback", "description": "A usable result."})
        return SimpleNamespace(returncode=0, stdout=json.dumps(metadata), stderr="")

    assert llm_meta.generate_metadata(
        "/imgs/thumb.jpg", llm_meta.VISION_MODELS[0], source="openrouter",
        run=with_listing([GEMMA, NVIDIA], attempt)) == ("Fallback", "A usable result.")
    assert tried == [GEMMA, NVIDIA]


def test_generate_metadata_openrouter_uses_attachment():
    cap = {}
    option = {"ollama_name": "local-tag", "openrouter_name": "openrouter/google/gemini-2.0-flash-exp:free", "llmfit_name": "vendor/model"}
    title, desc = llm_meta.generate_metadata(
        "/imgs/thumb.jpg", option, source="openrouter",
        run=make_run(stdout='{"title": "Dawn", "description": "A quiet field."}', capture=cap))
    assert title == "Dawn" and desc == "A quiet field."
    argv = cap["argv"]
    assert argv[:3] == ["llm", "-m", option["openrouter_name"]]
    i = argv.index("-a")
    assert argv[i + 1].endswith("thumb.jpg")      # image as attachment, absolute path
    assert "allowedTools" not in argv and "-o" not in argv  # no claude-cli Read path
    j = argv.index("--schema")
    assert json.loads(argv[j + 1]) == llm_meta.TITLE_DESC_SCHEMA
    assert "attached" in cap["stdin"].lower()      # attachment prompt, not "read the file"


def test_vision_models_keep_explicit_provider_ids():
    assert llm_meta.VISION_MODELS == [
        {"ollama_name": "gemma4:26b-a4b",
         "openrouter_name": "openrouter/google/gemma-4-26b-a4b-it:free",
         "llmfit_name": "google/gemma-4-26B-A4B-it"},
        {"ollama_name": "gemma4:31b",
         "openrouter_name": "openrouter/google/gemma-4-31b-it:free",
         "llmfit_name": "google/gemma-4-31B-it"},
    ]


def test_generate_metadata_local_uses_ollama_attachment_schema_and_thumbnail(tmp_path, monkeypatch):
    image = tmp_path / "thumb.jpg"
    image.write_bytes(b"thumbnail")
    option = llm_meta.VISION_MODELS[0]
    monkeypatch.setattr(llm_meta, "prepare_local_model", lambda model, **kwargs: None)
    calls = []

    class Response:
        def __enter__(self): return self
        def __exit__(self, *args): pass
        def read(self):
            return b'{"message":{"content":"{\\"title\\":\\"Dawn\\",\\"description\\":\\"A quiet field.\\"}"}}'

    def urlopen(request, timeout):
        calls.append((request, timeout))
        return Response()

    monkeypatch.setattr(llm_meta.urllib.request, "urlopen", urlopen)
    assert llm_meta.generate_metadata(image, option, source="local", timeout=17) == ("Dawn", "A quiet field.")
    request, timeout = calls[0]
    body = json.loads(request.data)
    assert request.full_url.endswith("/api/chat") and timeout == 17
    assert body["model"] == option["ollama_name"]
    assert body["messages"][0]["images"] == [base64.b64encode(b"thumbnail").decode("ascii")]
    assert "attached artwork" in body["messages"][0]["content"]
    assert body["format"] == llm_meta.TITLE_DESC_SCHEMA and body["stream"] is False


def test_local_model_fit_joins_llmfit_note_lists(monkeypatch):
    option = llm_meta.VISION_MODELS[0]
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda *args, **kwargs: SimpleNamespace(
        returncode=0,
        stdout=json.dumps({"models": [{"name": option["llmfit_name"], "fit_level": "Good", "notes": ["Fits.", "Enough RAM."]}]}),
        stderr=""))

    assert llm_meta.local_model_fit(option)["reason"] == "Fits.; Enough RAM."


def test_local_model_fit_reads_llmfit_without_ollama(monkeypatch):
    option = llm_meta.VISION_MODELS[0]
    calls = []
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda argv, **kwargs: (
        calls.append(argv) or SimpleNamespace(
            returncode=0,
            stdout=json.dumps({"models": [{"name": option["llmfit_name"], "fit_level": "Good", "notes": "Fits."}]}),
            stderr="")))
    monkeypatch.setattr(llm_meta.urllib.request, "urlopen",
                        lambda *args, **kwargs: pytest.fail("fit check must not require Ollama"))

    assert llm_meta.local_model_fit(option) == {
        "fit": True, "fit_level": "Good", "reason": "Fits."}
    assert calls == [["llmfit", "info", option["llmfit_name"], "--json"]]


def test_local_model_status_reads_llmfit_and_ollama_tags(monkeypatch):
    option = llm_meta.VISION_MODELS[0]
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda argv, **kwargs: SimpleNamespace(
        returncode=0, stdout=json.dumps({"models": [{"name": option["llmfit_name"], "fit_level": "Good", "notes": "Evaluator verdict."}], "system": {}}), stderr=""))

    class Response:
        def __enter__(self): return self
        def __exit__(self, *args): pass
        def read(self): return b'{"models":[{"name":"gemma4:26b-a4b"}]}'

    monkeypatch.setattr(llm_meta.urllib.request, "urlopen", lambda request, timeout: Response())
    assert llm_meta.local_model_status(option) == {
        "fit": True, "fit_level": "Good", "installed": True, "available": True,
        "reason": "Evaluator verdict."}


@pytest.mark.parametrize(("fit_level", "fit", "reason"), [
    ("Good", True, "ok"), ("Too Tight", False, "too large"),
])
def test_local_model_status_uses_llmfit_fit_level_and_notes(monkeypatch, fit_level, fit, reason):
    option = llm_meta.VISION_MODELS[0]
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda *args, **kwargs: SimpleNamespace(
        returncode=0, stdout=json.dumps({"models": [{"name": option["llmfit_name"], "fit_level": fit_level, "notes": reason}], "system": {}}), stderr=""))
    class Response:
        def __enter__(self): return self
        def __exit__(self, *args): pass
        def read(self): return b'{"models":[]}'
    monkeypatch.setattr(llm_meta.urllib.request, "urlopen", lambda *args, **kwargs: Response())
    result = llm_meta.local_model_status(option)
    assert result["fit"] is fit and result["fit_level"] == fit_level
    assert result["reason"] == reason
    assert result["available"] is fit


@pytest.mark.parametrize("stdout", ["not json", "{}", json.dumps({"models": [{"name": "other", "fit_level": "Good", "notes": ""}]} )])
def test_local_model_status_fails_closed_for_bad_llmfit_output(monkeypatch, stdout):
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda *args, **kwargs: SimpleNamespace(returncode=0, stdout=stdout, stderr=""))
    result = llm_meta.local_model_status(llm_meta.VISION_MODELS[0])
    assert result["fit"] is False and result["available"] is False


def test_local_model_status_fails_closed_for_llmfit_exit(monkeypatch):
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda *args, **kwargs: SimpleNamespace(returncode=1, stdout="", stderr="llmfit failed"))
    result = llm_meta.local_model_status(llm_meta.VISION_MODELS[0])
    assert not result["fit"] and not result["available"]
    assert result["fit_level"] == "Unavailable"


def test_ollama_installed_state_and_pull_api(monkeypatch):
    option = llm_meta.VISION_MODELS[0]
    calls = []
    class Response:
        def __init__(self, body): self.body = body
        def __enter__(self): return self
        def __exit__(self, *args): pass
        def read(self): return self.body
    def urlopen(request, timeout):
        calls.append(request)
        if request.full_url.endswith("/api/tags"):
            return Response(b'{"models":[]}')
        return Response(b'{"status":"success"}')
    monkeypatch.setattr(llm_meta.urllib.request, "urlopen", urlopen)
    monkeypatch.setattr(llm_meta.subprocess, "run", lambda *args, **kwargs: SimpleNamespace(returncode=0, stdout=json.dumps({"models": [{"name": option["llmfit_name"], "fit_level": "Good", "notes": "ok"}], "system": {}}), stderr=""))
    llm_meta.prepare_local_model(option, install=True)
    assert calls[0].method == "GET" and calls[0].full_url.endswith("/api/tags")
    assert calls[1].method == "POST" and calls[1].full_url.endswith("/api/pull")
    assert json.loads(calls[1].data) == {"model": option["ollama_name"], "stream": False}


@pytest.mark.parametrize("body", [b"", b"not-json"])
def test_ollama_chat_bad_response_fails(tmp_path, monkeypatch, body):
    image = tmp_path / "image.jpg"
    image.write_bytes(b"image")
    monkeypatch.setattr(llm_meta, "prepare_local_model", lambda *args, **kwargs: None)
    class Response:
        def __enter__(self): return self
        def __exit__(self, *args): pass
        def read(self): return body
    monkeypatch.setattr(llm_meta.urllib.request, "urlopen", lambda *args, **kwargs: Response())
    with pytest.raises(RuntimeError):
        llm_meta.generate_metadata(image, llm_meta.VISION_MODELS[0], source="local")


# --- OpenRouter free-vision catalogue fallback -------------------------------

def test_free_vision_models_keeps_only_the_ids_that_take_images():
    assert llm_meta.free_vision_models([
        {"id": "google/gemma-4-26b-a4b-it:free",
         "architecture": {"input_modalities": ["text", "image"]}},
        {"id": "some/text-only:free", "architecture": {"input_modalities": ["text"]}},
        {"id": "malformed:free"},
    ]) == (GEMMA,)


def test_run_llm_429_walks_to_another_vendor_before_a_sibling(monkeypatch):
    monkeypatch.setattr(llm_meta, "_NO_SCHEMA", set())
    tried = []

    def attempt(argv, stdin):
        tried.append(argv[2])
        if argv[2] == GEMMA:
            return SimpleNamespace(returncode=1, stdout="", stderr=RL)
        return SimpleNamespace(returncode=0, stdout='{"a": 1}', stderr="")

    assert llm_meta.run_llm(GEMMA, "hi", schema={"type": "object"}, attach="/i.png",
                            run=with_listing([GEMMA, GEMMA_31, NVIDIA], attempt)) == {"a": 1}
    # 429s are per upstream provider, so the same-vendor sibling is tried LAST
    assert tried == [GEMMA, NVIDIA]


def test_run_llm_429_everywhere_tries_every_candidate_then_raises():
    tried = []

    def attempt(argv, stdin):
        tried.append(argv[2])
        return SimpleNamespace(returncode=1, stdout="", stderr=RL)

    with pytest.raises(RuntimeError, match="429"):
        llm_meta.run_llm(GEMMA, "hi", attach="/i.png",
                         run=with_listing([GEMMA, GEMMA_31, NVIDIA], attempt))
    assert tried == [GEMMA, NVIDIA, GEMMA_31]      # each eligible id, exactly once


def test_run_llm_429_on_the_claude_path_never_lists_models():
    seen = []

    def run(argv, stdin):
        seen.append(argv)
        return SimpleNamespace(returncode=1, stdout="", stderr=RL)

    with pytest.raises(RuntimeError, match="429"):
        llm_meta.run_llm("claude-cli-opus", "hi", cwd="/w", run=run)
    # no sibling to fall back to and no key to reach one, so nothing is listed
    assert len(seen) == 1 and llm_meta.FREE_MODELS_ARGV not in seen


def test_run_llm_unlistable_models_still_raise_the_original_429():
    def run(argv, stdin):
        if argv[:2] == ["llm", "openrouter"]:
            raise OSError("network is down")
        return SimpleNamespace(returncode=1, stdout="", stderr=RL)

    with pytest.raises(RuntimeError, match="429"):
        llm_meta.run_llm(GEMMA, "hi", attach="/i.png", run=run)


def test_run_llm_stops_walking_once_the_timeout_budget_is_spent(monkeypatch):
    clock = [0.0]
    monkeypatch.setattr(llm_meta, "time", SimpleNamespace(monotonic=lambda: clock[0]))
    tried = []

    def slow_429(argv, stdin):
        tried.append(argv[2])
        clock[0] += 50          # each candidate eats 50s of the 90s budget
        return SimpleNamespace(returncode=1, stdout="", stderr=RL)

    with pytest.raises(RuntimeError, match="429"):
        llm_meta.run_llm(GEMMA, "hi", attach="/i.png", timeout=90,
                         run=with_listing([GEMMA, GEMMA_31, NVIDIA], slow_429))
    assert tried == [GEMMA, NVIDIA]      # the third never starts: the budget is gone
