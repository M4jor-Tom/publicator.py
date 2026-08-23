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
"""
import functools
import json
import logging
import os
import subprocess
import time

log = logging.getLogger("publicator.llm")

DEFAULT_MODEL = "claude-cli-opus"  # llm-claude-cli's id (scenharnist's default); see `llm models`
# ponytail: free :free ids churn on OpenRouter; this is the current free vision
# model. Lives here, not in an app's argparse, so every entrypoint shares one id.
OPENROUTER_MODEL = "openrouter/google/gemma-4-26b-a4b-it:free"
DEFAULT_TIMEOUT = 300  # seconds — a vision call is slow through either provider

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
        raise RuntimeError(f"llm timed out after {timeout}s") from e
    log.debug("llm done in %.1fs rc=%d", time.monotonic() - started, cp.returncode)
    return cp


def _loads_json(out):
    """json.loads the outermost {...}, tolerating fences/prose around it — which is
    what a model asked for JSON in the prompt, rather than by schema, tends to add."""
    try:
        return json.loads(out[out.find("{"):out.rfind("}") + 1])
    except json.JSONDecodeError as e:
        raise RuntimeError(f"llm returned non-JSON with schema: {out!r}") from e


def run_llm(model, prompt, *, schema=None, cwd=None, attach=None, run=None, timeout=DEFAULT_TIMEOUT):
    run = run or functools.partial(_default_run, timeout=timeout)
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
        cp = _invoke(run, argv, prompt, timeout)
    else:
        schema_json = json.dumps(schema)
        in_prompt = prompt + _JSON_FALLBACK.format(schema=schema_json)
        if model in _NO_SCHEMA:
            cp = _invoke(run, argv, in_prompt, timeout)
        else:
            cp = _invoke(run, argv + ["--schema", schema_json], prompt, timeout)
            if cp.returncode != 0 and NO_SCHEMA_SUPPORT in (cp.stderr or ""):
                log.debug("%s rejects --schema; retrying with the shape in the prompt", model)
                _NO_SCHEMA.add(model)
                cp = _invoke(run, argv, in_prompt, timeout)
    if cp.returncode != 0:
        raise RuntimeError(f"llm failed ({cp.returncode}): {cp.stderr}")
    out = (cp.stdout or "").strip()
    log.debug("llm output: %s", out)
    return out if schema is None else _loads_json(out)


def generate_metadata(image_path, model=DEFAULT_MODEL, *, run=None, timeout=DEFAULT_TIMEOUT):
    """(title, description) for an artwork, via the key-free `llm` vision path."""
    abspath = os.path.abspath(image_path)
    if model.startswith("openrouter/"):
        obj = run_llm(model, _PROMPT_ATTACH, schema=TITLE_DESC_SCHEMA,
                      attach=abspath, run=run, timeout=timeout)
    else:
        obj = run_llm(model, _PROMPT.format(name=os.path.basename(image_path)),
                      schema=TITLE_DESC_SCHEMA, cwd=os.path.dirname(abspath),
                      run=run, timeout=timeout)
    title = str(obj.get("title", "")).strip()
    description = str(obj.get("description", "")).strip()
    if not title or not description:
        raise RuntimeError(f"AI reply missing title/description: {obj!r}")
    return title, description
