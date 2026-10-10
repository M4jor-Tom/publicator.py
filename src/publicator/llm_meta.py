"""Key-free title/description generation via the `llm` CLI + `llm-claude-cli`.

Runs on the logged-in Claude subscription (no API key): `llm -m <model>` shells
out to the `claude` CLI. Vision works by handing the model the image *path* plus
the Read tool (`-o allowedTools Read -o cwd <dir>`) — `llm -a` attachments do NOT
work through this plugin. Pattern copied from scenharnist's `llmcli.run_llm`; the
~25 lines beat coupling two sibling flakes.  # ponytail: duplication < cross-repo dep

A second provider has landed: `openrouter/*` model ids (via `llm-openrouter`,
API-key based) have no Read tool, so they go through a `-a <image>` attachment
instead of the Read-path prompt. Most of them also reject `--schema` — see
`NO_SCHEMA_SUPPORT` below.

A rate-limited `:free` model is a model-*selection* problem, not a timing one --
the free tier shares one upstream pool across every OpenRouter user, so backing
off changes nothing that another vendor's model wouldn't fix instantly. OpenRouter
walks the other free vision ids after any failed model attempt; `_fallbacks`, ADR 0008.
"""
import base64
import functools
import json
import logging
import os
import re
import subprocess
import time
import urllib.request

log = logging.getLogger("publicator.llm")

DEFAULT_MODEL = "claude-cli-opus"  # llm-claude-cli's id (scenharnist's default); see `llm models`
TODO_GENERATE = "[[PUBLICATOR:TODO_GENERATE]]"


class ProviderUnavailable(RuntimeError):
    """A provider call failed before returning usable metadata."""


# ponytail: free :free ids churn on OpenRouter; this is the current free vision
# model. Lives here, not in an app's argparse, so every entrypoint shares one id.
OPENROUTER_MODEL = "openrouter/google/gemma-4-26b-a4b-it:free"
VISION_MODELS = [
    {"ollama_name": "gemma4:26b-a4b", "openrouter_name": "openrouter/google/gemma-4-26b-a4b-it:free", "llmfit_name": "google/gemma-4-26B-A4B-it"},
    {"ollama_name": "gemma4:31b", "openrouter_name": "openrouter/google/gemma-4-31b-it:free", "llmfit_name": "google/gemma-4-31B-it"},
]
DEFAULT_TIMEOUT = 300  # seconds — a vision call is slow through either provider
OLLAMA_URL = "http://localhost:11434"

# Ask the plugin, never openrouter.ai: `register_models` mints the ids `llm -m`
# accepts from this same 1h disk cache, so a listed candidate always resolves --
# and it needs no key and survives an outage. ADR 0006 alternative G.
FREE_MODELS_ARGV = ["llm", "openrouter", "models", "--free", "--json"]

# `llm` raises this when --schema meets a model that lacks `structured_outputs`
# (7 of the 8 free OpenRouter vision models). It fires before any HTTP call, so
# the retry below costs one process spawn, not API tokens.
NO_SCHEMA_SUPPORT = "does not support schemas"
_NO_SCHEMA: set[str] = set()  # static per model — pay that spawn once per process

TITLE_DESC_SCHEMA = {
    "type": "object",
    "properties": {
        "title": {"type": "string", "description": "<= 50 chars, evocative"},
        "description": {"type": "string", "description": "2-3 sentences, invented story of what could be happening in the scene"},
    },
    "required": ["title", "description"],
}

_PROMPT = (
    "Read the image file {name} in the current directory and look at the artwork. "
    "Reply with a title (<= 50 characters, evocative) and a description (2-3 "
    "sentences) that invents a short backstory for the scene — imagine what could "
    "be happening in it and narrate that. No hashtags, no emojis."
)

_PROMPT_ATTACH = (
    "Look at the attached artwork. Reply with a title (<= 50 characters, "
    "evocative) and a description (2-3 sentences) that invents a short backstory "
    "for the scene — imagine what could be happening in it and narrate that. "
    "No hashtags, no emojis."
)


_JSON_FALLBACK = (
    "\n\nReply with ONLY a JSON object matching this schema — no prose, no "
    "markdown fences:\n{schema}"
)


def _default_run(argv, stdin, timeout):
    return subprocess.run(argv, input=stdin, capture_output=True, text=True, timeout=timeout)


def _invoke(run, argv, prompt, timeout):
    log.debug("llm call (timeout=%ss): %s", timeout, " ".join(argv))
    log.debug("llm prompt: %s", prompt)
    started = time.monotonic()
    try:
        cp = run(argv, prompt)
    except subprocess.TimeoutExpired as e:
        log.debug("llm timed out after %.1fs (limit %ss)", time.monotonic() - started, timeout)
        raise ProviderUnavailable(f"llm timed out after {timeout}s") from e
    except OSError as e:
        raise RuntimeError(f"could not start llm: {e}") from e
    log.debug("llm done in %.1fs rc=%d", time.monotonic() - started, cp.returncode)
    return cp


def _loads_json(out):
    """json.loads the outermost {...}, tolerating fences/prose around it — which is
    what a model asked for JSON in the prompt, rather than by schema, tends to add."""
    try:
        return json.loads(out[out.find("{"):out.rfind("}") + 1])
    except json.JSONDecodeError as e:
        raise RuntimeError(f"llm returned non-JSON with schema: {out!r}") from e


def _metadata_fields(obj):
    if not isinstance(obj, dict):
        raise RuntimeError(f"AI reply missing title/description: {obj!r}")
    title = str(obj.get("title", "")).strip()
    description = str(obj.get("description", "")).strip()
    if not title or not description:
        raise RuntimeError(f"AI reply missing title/description: {obj!r}")
    return title, description


def free_vision_models(catalogue):
    """The models in `catalogue` that take images, prefixed for `llm -m`. `--free`
    did the tier filter; this adds the vision clause, which has no plugin flag."""
    return tuple("openrouter/" + m["id"] for m in catalogue
                 if "image" in (m.get("architecture") or {}).get("input_modalities", ()))


def _fallbacks(model, run):
    """`model`, then every other free vision id. A generator on purpose: `run_llm`
    only asks for alternatives after a failed attempt, so the happy path lists nothing."""
    yield model
    if not model.startswith("openrouter/"):
        return          # the claude-cli path has no sibling, nor a key to reach one
    try:
        others = [m for m in free_vision_models(json.loads(run(FREE_MODELS_ARGV, "").stdout))
                  if m != model]
    except (OSError, ValueError, KeyError, TypeError) as e:
        # Fail open (ADR 0004): the rescue path must never become the failure it
        # exists to rescue. Narrow on purpose — our own bugs still raise.
        log.warning("cannot list free vision models (%s); no model fallback", e)
        return
    # The 429 is per upstream provider, so a sibling from the same vendor is the
    # likeliest to be saturated too — keep it, but try it last.
    others.sort(key=lambda m: m.rsplit("/", 1)[0] == model.rsplit("/", 1)[0])
    yield from others


def _attempt(model, prompt, *, schema, cwd, attach, run, timeout):
    """One model's shot: argv build plus ADR 0005's schema retry -> CompletedProcess."""
    if attach is not None:
        # API-key providers (openrouter/*): vision via -a attachment. The
        # claude-cli -o options don't exist on these models, so omit them.
        argv = ["llm", "-m", model, "-a", str(attach)]
    else:
        # llm-claude-cli: key-free, vision via the Read tool over cwd.
        # `-o timeout` caps the plugin's own claude-code call to ours.
        argv = ["llm", "-m", model, "-o", "allowedTools", "Read",
                "-o", "cwd", str(cwd), "-o", "timeout", str(timeout)]
    if schema is None:
        return _invoke(run, argv, prompt, timeout)
    schema_json = json.dumps(schema)
    in_prompt = prompt + _JSON_FALLBACK.format(schema=schema_json)
    if model in _NO_SCHEMA:
        return _invoke(run, argv, in_prompt, timeout)
    cp = _invoke(run, argv + ["--schema", schema_json], prompt, timeout)
    if cp.returncode != 0 and NO_SCHEMA_SUPPORT in (cp.stderr or ""):
        log.debug("%s rejects --schema; retrying with the shape in the prompt", model)
        _NO_SCHEMA.add(model)
        cp = _invoke(run, argv, in_prompt, timeout)
    return cp


def _provider_unavailable(stderr):
    text = stderr.lower()
    return (re.search(r"error code:\s*(?:408|429|5\d\d)\b", text) is not None or any(
        marker in text for marker in (
            "connection error", "connectionerror", "connecterror", "connecttimeout",
            "connection refused", "connection reset", "name resolution",
            "network is unreachable", "timed out", "service unavailable",
        )))


def run_llm(model, prompt, *, schema=None, cwd=None, attach=None, run=None, timeout=DEFAULT_TIMEOUT):
    # `timeout` budgets the whole call, not each attempt: the walk must not turn a
    # 300s ceiling into N x 300s on a request the gallery tab is blocking on.
    deadline = time.monotonic() + timeout
    last_error = None
    for candidate in _fallbacks(model, run or functools.partial(_default_run, timeout=timeout)):
        left = max(1, int(deadline - time.monotonic()))
        try:
            cp = _attempt(candidate, prompt, schema=schema, cwd=cwd, attach=attach,
                          run=run or functools.partial(_default_run, timeout=left), timeout=left)
            if cp.returncode != 0:
                error = f"llm failed ({cp.returncode}): {cp.stderr}"
                if _provider_unavailable(cp.stderr or ""):
                    raise ProviderUnavailable(error)
                raise RuntimeError(error)
            out = (cp.stdout or "").strip()
            result = out if schema is None else _loads_json(out)
            if schema == TITLE_DESC_SCHEMA:
                _metadata_fields(result)
        except RuntimeError as e:
            last_error = e
            if time.monotonic() >= deadline:
                log.info("%ss budget spent; giving up the walk after %s", timeout, candidate)
                break
            if candidate.startswith("openrouter/"):
                log.info("%s failed; trying the next free vision model if available", candidate)
            continue
        if candidate != model:      # the metadata did not come from the id you picked
            log.info("%s answered instead of the failed %s", candidate, model)
        log.debug("llm output: %s", out)
        return result
    raise last_error


def _ollama_request(path, *, method="GET", data=None, timeout=DEFAULT_TIMEOUT):
    request = urllib.request.Request(
        OLLAMA_URL + path, data=data,
        headers={"Content-Type": "application/json"} if data is not None else {},
        method=method,
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
    except OSError as e:
        raise RuntimeError(f"Ollama is unavailable at {OLLAMA_URL}: {e}") from e
    try:
        return json.loads(raw)
    except (ValueError, TypeError) as e:
        raise RuntimeError(f"Ollama returned invalid JSON from {path}") from e


def local_model_fit(model_option):
    """Report llmfit's host-fit verdict without requiring an Ollama service."""
    try:
        result = subprocess.run(
            ["llmfit", "info", model_option["llmfit_name"], "--json"],
            capture_output=True, text=True,
        )
        if result.returncode:
            raise RuntimeError((result.stderr or "llmfit failed").strip())
        report = json.loads(result.stdout)
        model = next(m for m in report["models"] if m["name"] == model_option["llmfit_name"])
        fit_level = model["fit_level"]
        notes = model.get("notes", "")
        reason = "; ".join(map(str, notes)) if isinstance(notes, list) else str(notes or "")
        fit = fit_level == "Good"
        if not fit and not reason:
            reason = f"llmfit fit_level: {fit_level}"
        return {"fit": fit, "fit_level": fit_level, "reason": reason}
    except (OSError, RuntimeError, ValueError, KeyError, TypeError, StopIteration) as e:
        return {"fit": False, "fit_level": "Unavailable",
                "reason": str(e) or "llmfit could not assess this model"}


def local_model_status(model_option):
    """Report the host-fit verdict and Ollama's installed-model inventory."""
    fit_status = local_model_fit(model_option)
    if not fit_status["fit"]:
        return {**fit_status, "installed": False, "available": False}
    try:
        tags = _ollama_request("/api/tags")
        installed = any(m.get("name") == model_option["ollama_name"] for m in tags["models"])
    except (RuntimeError, KeyError, TypeError) as e:
        return {**fit_status, "installed": False, "available": False, "reason": str(e)}
    return {**fit_status, "installed": installed, "available": True}


def prepare_local_model(model_option, *, install):
    status = local_model_status(model_option)
    if not status["fit"]:
        raise RuntimeError(f"Local model does not fit: {status['reason']}")
    if not status["available"]:
        raise RuntimeError(f"Ollama is unavailable: {status['reason']}")
    if not status["installed"]:
        if not install:
            raise RuntimeError(f"Local model {model_option['ollama_name']} is not installed")
        result = _ollama_request("/api/pull", method="POST", data=json.dumps(
            {"model": model_option["ollama_name"], "stream": False}).encode())
        if result.get("status") != "success":
            raise RuntimeError(f"Ollama failed to pull {model_option['ollama_name']}: {result!r}")


def _local_metadata(image_path, model_option, *, timeout):
    try:
        with open(image_path, "rb") as image:
            encoded = base64.b64encode(image.read()).decode("ascii")
    except OSError as e:
        raise RuntimeError(f"cannot read AI image {image_path}: {e}") from e
    result = _ollama_request("/api/chat", method="POST", data=json.dumps({
        "model": model_option["ollama_name"],
        "messages": [{"role": "user", "content": _PROMPT_ATTACH, "images": [encoded]}],
        "format": TITLE_DESC_SCHEMA,
        "stream": False,
    }).encode(), timeout=timeout)
    try:
        return _loads_json(result["message"]["content"])
    except (KeyError, TypeError) as e:
        raise RuntimeError(f"Ollama returned an invalid chat response: {result!r}") from e


def generate_metadata(image_path, model_option: dict[str, str] | None, *, source: str,
                      claude_model: str | None = None, run=None, timeout=DEFAULT_TIMEOUT,
                      install_missing=False) -> tuple[str, str]:
    """Return validated title/description using the selected metadata provider."""
    abspath = os.path.abspath(image_path)
    if source == "local":
        if model_option is None:
            raise ValueError("local generation requires a model option")
        prepare_local_model(model_option, install=install_missing)
        obj = _local_metadata(abspath, model_option, timeout=timeout)
    elif source == "openrouter":
        if model_option is None:
            raise ValueError("OpenRouter generation requires a model option")
        obj = run_llm(model_option["openrouter_name"], _PROMPT_ATTACH,
                      schema=TITLE_DESC_SCHEMA, attach=abspath, run=run, timeout=timeout)
    elif source == "claude":
        obj = run_llm(claude_model if claude_model is not None else DEFAULT_MODEL,
                      _PROMPT.format(name=os.path.basename(image_path)),
                      schema=TITLE_DESC_SCHEMA, cwd=os.path.dirname(abspath),
                      run=run, timeout=timeout)
    else:
        raise ValueError(f"unknown metadata source: {source}")
    return _metadata_fields(obj)
