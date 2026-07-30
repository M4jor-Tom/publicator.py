"""Key-free title/description generation via the `llm` CLI + `llm-claude-cli`.

Runs on the logged-in Claude subscription (no API key): `llm -m <model>` shells
out to the `claude` CLI. Vision works by handing the model the image *path* plus
the Read tool (`-o allowedTools Read -o cwd <dir>`) — `llm -a` attachments do NOT
work through this plugin. Pattern copied from scenharnist's `llmcli.run_llm`; the
~25 lines beat coupling two sibling flakes.  # ponytail: duplication < cross-repo dep

A second provider has landed: `openrouter/*` model ids (via `llm-openrouter`,
API-key based) have no Read tool, so they go through a `-a <image>` attachment
instead of the Read-path prompt.
"""
import functools
import json
import logging
import os
import subprocess
import time

log = logging.getLogger("publicator.llm")

DEFAULT_MODEL = "claude-cli-opus"  # llm-claude-cli's id (scenharnist's default); see `llm models`

TITLE_DESC_SCHEMA = {
    "type": "object",
    "properties": {
        "title": {"type": "string", "description": "<= 50 chars, evocative"},
        "description": {"type": "string", "description": "2-3 sentences, artist voice"},
    },
    "required": ["title", "description"],
}

_PROMPT = (
    "Read the image file {name} in the current directory and look at the artwork. "
    "Reply with a title (<= 50 characters, evocative) and a description (2-3 "
    "sentences, in the artist's voice). No hashtags, no emojis."
)

_PROMPT_ATTACH = (
    "Look at the attached artwork. Reply with a title (<= 50 characters, "
    "evocative) and a description (2-3 sentences, in the artist's voice). "
    "No hashtags, no emojis."
)


def _default_run(argv, stdin, timeout):
    return subprocess.run(argv, input=stdin, capture_output=True, text=True, timeout=timeout)


def run_llm(model, prompt, *, schema=None, cwd=None, attach=None, run=None, timeout=300):
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
    if schema is not None:
        argv += ["--schema", json.dumps(schema)]
    log.debug("llm call (timeout=%ss): %s", timeout, " ".join(argv))
    log.debug("llm prompt: %s", prompt)
    started = time.monotonic()
    try:
        cp = run(argv, prompt)
    except subprocess.TimeoutExpired as e:
        log.debug("llm timed out after %.1fs (limit %ss)", time.monotonic() - started, timeout)
        raise RuntimeError(f"llm timed out after {timeout}s") from e
    log.debug("llm done in %.1fs rc=%d", time.monotonic() - started, cp.returncode)
    if cp.returncode != 0:
        raise RuntimeError(f"llm failed ({cp.returncode}): {cp.stderr}")
    out = (cp.stdout or "").strip()
    log.debug("llm output: %s", out)
    if schema is None:
        return out
    try:
        return json.loads(out)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"llm returned non-JSON with schema: {out!r}") from e


def generate_metadata(image_path, model=DEFAULT_MODEL, *, run=None, timeout=300):
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
