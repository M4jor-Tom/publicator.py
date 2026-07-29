"""Key-free title/description generation via the `llm` CLI + `llm-claude-cli`.

Runs on the logged-in Claude subscription (no API key): `llm -m <model>` shells
out to the `claude` CLI. Vision works by handing the model the image *path* plus
the Read tool (`-o allowedTools Read -o cwd <dir>`) — `llm -a` attachments do NOT
work through this plugin. Pattern copied from scenharnist's `llmcli.run_llm`; the
~25 lines beat coupling two sibling flakes.  # ponytail: duplication < cross-repo dep

To add an API-key provider later: `llm install llm-anthropic` (or -openai/-gemini),
set the key (`llm keys set ...` or env var), and pass its `-m` id. Those models have
no Read tool, so swap the Read-path prompt for a `-a <image>` attachment then — a
small branch to add when a second provider actually lands, not before (YAGNI).
"""
import functools
import json
import os
import subprocess

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


def _default_run(argv, stdin, timeout):
    return subprocess.run(argv, input=stdin, capture_output=True, text=True, timeout=timeout)


def run_llm(model, prompt, *, schema=None, cwd, run=None, timeout=90):
    run = run or functools.partial(_default_run, timeout=timeout)
    # `-o timeout` caps the plugin's own claude-code call to ours; its default
    # (300s) is longer than our subprocess timeout and would otherwise cut off.
    argv = ["llm", "-m", model, "-o", "allowedTools", "Read",
            "-o", "cwd", str(cwd), "-o", "timeout", str(timeout)]
    if schema is not None:
        argv += ["--schema", json.dumps(schema)]
    try:
        cp = run(argv, prompt)
    except subprocess.TimeoutExpired as e:
        raise RuntimeError(f"llm timed out after {timeout}s") from e
    if cp.returncode != 0:
        raise RuntimeError(f"llm failed ({cp.returncode}): {cp.stderr}")
    out = (cp.stdout or "").strip()
    if schema is None:
        return out
    try:
        return json.loads(out)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"llm returned non-JSON with schema: {out!r}") from e


def generate_metadata(image_path, model=DEFAULT_MODEL, *, run=None):
    """(title, description) for an artwork, via the key-free `llm` vision path."""
    cwd = os.path.dirname(os.path.abspath(image_path))
    obj = run_llm(model, _PROMPT.format(name=os.path.basename(image_path)),
                  schema=TITLE_DESC_SCHEMA, cwd=cwd, run=run)
    title = str(obj.get("title", "")).strip()
    description = str(obj.get("description", "")).strip()
    if not title or not description:
        raise RuntimeError(f"AI reply missing title/description: {obj!r}")
    return title, description
