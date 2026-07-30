# AI Model Selector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a global header dropdown to publish-next that selects which model writes AI title/description — the key-free Claude subscription (default) or an OpenRouter free vision model.

**Architecture:** `llm_meta.generate_metadata` gains a provider branch (`openrouter/*` ids use an `-a` image attachment; everything else keeps today's Read-tool path). `publish_next.py` grows a `--openrouter-model` CLI flag, a header `<select>` whose value is sent to `/ai`, and an allow-list guard on the `/ai` handler. `flake.nix` vendors `llm-openrouter` into the python env; the API key comes from the `OPENROUTER_KEY` env var, not the flake.

**Tech Stack:** Python 3.12 stdlib `http.server`, the `llm` CLI + `llm-claude-cli` (+ new `llm-openrouter`) plugins, Nix flake, pytest.

## Global Constraints

- Python 3.12; stdlib only in app code (no new pip deps beyond what the flake provides).
- The Claude path stays **key-free** — do not require an API key for the existing behavior.
- OpenRouter key reaches the process via the `OPENROUTER_KEY` env var only; never write it into the flake or repo.
- OpenRouter default model must be **vision-capable and free**: `openrouter/google/gemini-2.0-flash-exp:free` (swappable via the flag).
- `llm` model id for the openrouter branch is detected by the prefix `openrouter/`.
- Preserve existing `generate_metadata(image_path, model=DEFAULT_MODEL, *, run=None, timeout=300)` public signature — the branch is internal.
- Conventional Commits; commit after each task.

---

### Task 1: Provider branch in `llm_meta.generate_metadata`

**Files:**
- Modify: `llm_meta.py` (add `_PROMPT_ATTACH`, add `attach=` path to `run_llm`, branch in `generate_metadata`)
- Test: `test_llm_meta.py` (add openrouter attachment test)

**Interfaces:**
- Consumes: nothing new.
- Produces: `generate_metadata(image_path, model, *, run=None, timeout=300)` now routes `openrouter/*` ids through an attachment call. `run_llm(..., attach=<path>)` builds `["llm","-m",model,"-a",str(attach)]` (+`--schema`) with **no** claude-cli `-o` options. `cwd` becomes optional (default `None`).

- [ ] **Step 1: Write the failing test**

Add to `test_llm_meta.py`:

```python
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest test_llm_meta.py::test_generate_metadata_openrouter_uses_attachment -v`
Expected: FAIL — current `generate_metadata` always uses the Read path, so argv has `allowedTools`/`-o` and no `-a`.

- [ ] **Step 3: Write minimal implementation**

In `llm_meta.py`, add the attachment prompt next to `_PROMPT`:

```python
_PROMPT_ATTACH = (
    "Look at the attached artwork. Reply with a title (<= 50 characters, "
    "evocative) and a description (2-3 sentences, in the artist's voice). "
    "No hashtags, no emojis."
)
```

Change `run_llm` to accept `attach` and make `cwd` optional:

```python
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
```

Branch in `generate_metadata`:

```python
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
```

Also update the module docstring's "small branch to add when a second provider actually lands" note to say it has landed (openrouter attachment path).

- [ ] **Step 4: Run the whole test file to verify pass + no regression**

Run: `python -m pytest test_llm_meta.py -v`
Expected: PASS — the new test plus all existing ones (the claude-path tests still pass `cwd=` and hit the `else` branch).

- [ ] **Step 5: Commit**

```bash
git add llm_meta.py test_llm_meta.py
git commit -m "feat(llm): route openrouter/* models through -a image attachment path"
```

---

### Task 2: Vendor `llm-openrouter` in `flake.nix`

**Files:**
- Modify: `flake.nix` (add derivation + add to `withPackages`)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `llm` inside the flake env discovers `openrouter/*` models. No Python-code interface.

- [ ] **Step 1: Prefer nixpkgs, else get the PyPI sdist URL**

First check whether the pin already ships it (laziest path — rung 5):

```bash
nix eval --raw .#devShells.x86_64-linux.default.buildInputs 2>/dev/null >/dev/null
nix eval nixpkgs#python312Packages.llm-openrouter.version 2>/dev/null \
  && echo "USE nixpkgs ps.llm-openrouter" || echo "VENDOR from PyPI"
```

If `USE nixpkgs`: skip Step 2's derivation and just add `ps.llm-openrouter` to the `withPackages` list in Step 3. Otherwise get the sdist URL:

```bash
curl -s https://pypi.org/pypi/llm-openrouter/json \
  | jq -r '.urls[] | select(.packagetype=="sdist") | .url'
```

- [ ] **Step 2: Add the vendored derivation (only if VENDOR)**

After the `llm-claude-cli` derivation (around `flake.nix:41`), add — with `<URL>` from Step 1 and `hash` left as `pkgs.lib.fakeHash` for now:

```nix
      # `llm-openrouter` isn't guaranteed in the pin — vendor it from PyPI the
      # same way as llm-claude-cli so `llm` sees `openrouter/*` models. The API
      # key is NOT baked in: llm-openrouter reads $OPENROUTER_KEY at runtime.
      llm-openrouter = pyInterp.pkgs.buildPythonPackage {
        pname = "llm-openrouter";
        version = "<VERSION>";
        pyproject = true;
        src = pkgs.fetchurl {
          url = "<URL>";
          hash = pkgs.lib.fakeHash;
        };
        build-system = [ pyInterp.pkgs.setuptools ];
        dependencies = [ pyInterp.pkgs.llm pyInterp.pkgs.openai ];
        pythonImportsCheck = [ "llm_openrouter" ];
        doCheck = false;  # upstream tests hit the network
      };
```

- [ ] **Step 3: Add it to the python env**

Change `flake.nix:42`:

```nix
      python = pyInterp.withPackages (ps: [ ps.jsonschema ps.playwright ps.llm llm-claude-cli llm-openrouter ]);
```

(If using the nixpkgs package instead, write `ps.llm-openrouter` in place of the bare `llm-openrouter`.)

- [ ] **Step 4: Resolve the real hash (only if VENDOR with fakeHash)**

Run: `nix build .#apps.x86_64-linux.publish-next.program 2>&1 | rg -A2 'got:'`
Expected: build fails with a hash mismatch printing the real `sha256-...`. Copy that value over `pkgs.lib.fakeHash` in Step 2.

- [ ] **Step 5: Build to verify the env resolves + llm sees openrouter**

```bash
nix build .#apps.x86_64-linux.publish-next.program
nix develop -c llm models 2>/dev/null | rg -i 'openrouter' | head -3
```
Expected: build succeeds; at least one `openrouter/...` model listed. (Listing works without a key; a real generation needs `OPENROUTER_KEY`.)

- [ ] **Step 6: Commit**

```bash
git add flake.nix flake.lock
git commit -m "build(flake): vendor llm-openrouter for the OpenRouter model option"
```

---

### Task 3: Wire the selector through `publish_next.py`

**Files:**
- Modify: `publish_next.py` (CLI flag, class attr, header `<select>`, `aiGen` read, `/ai` allow-list, page render)
- Test: `test_publish_next.py` (new — page renders both options)

**Interfaces:**
- Consumes: `generate_metadata` provider branch from Task 1 (public signature unchanged).
- Produces: `/ai` accepts `model` only when it is one of `{ai_model, openrouter_model}`; page exposes a `#ai-model` select.

- [ ] **Step 1: Write the failing test**

Create `test_publish_next.py`:

```python
import publish_next


def test_page_offers_both_models():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = []; H.pending = []
    H.initial_max_ts = 0
    H.ai_model = "claude-cli-opus"
    H.openrouter_model = "openrouter/google/gemini-2.0-flash-exp:free"
    # ponytail: _build_page reads only class attrs, so call it with the class as
    # `self` — avoids constructing a real BaseHTTPRequestHandler (needs a socket).
    page = publish_next.GalleryHandler._build_page(H)
    assert 'id="ai-model"' in page
    assert 'claude-cli-opus' in page
    assert 'openrouter/google/gemini-2.0-flash-exp:free' in page
    assert '__AI_MODEL__' not in page and '__OPENROUTER_MODEL__' not in page
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python -m pytest test_publish_next.py -v`
Expected: FAIL — no `#ai-model` select yet, `openrouter_model` attr missing, `__OPENROUTER_MODEL__` unreplaced.

- [ ] **Step 3: Add the CLI flag**

After `publish_next.py:676` (`--ai-model`):

```python
    parser.add_argument("--openrouter-model",
                        default="openrouter/google/gemini-2.0-flash-exp:free",
                        help="free vision model for the OpenRouter option; needs $OPENROUTER_KEY")
```

- [ ] **Step 4: Add the class attribute and wire it in `serve`**

Add to the `GalleryHandler` class attributes (near `publish_next.py:393`, after `ai_model = DEFAULT_MODEL`):

```python
    openrouter_model = ""
```

In `serve` (after `publish_next.py:640`, `GalleryHandler.ai_model = args.ai_model`):

```python
    GalleryHandler.openrouter_model = args.openrouter_model
```

- [ ] **Step 5: Add the header select and drop the JS constant**

Replace the header block (`publish_next.py:231-235`):

```html
<header>
  <h1>Publish next</h1>
  <span id="queue-count">0 queued</span>
  <select id="ai-model" title="AI model for Generate with AI">
    <option value="__AI_MODEL__">Claude (subscription)</option>
    <option value="__OPENROUTER_MODEL__">OpenRouter (free)</option>
  </select>
  <button id="publish-btn" onclick="publishQueue()" disabled>Publish</button>
</header>
```

Delete the now-unused JS constant line (`publish_next.py:241`):

```javascript
const AI_MODEL = "__AI_MODEL__";
```

- [ ] **Step 6: Read the select value in `aiGen`**

In `aiGen` (`publish_next.py:307-308`), change the body sent to `/ai`:

```javascript
    const model = document.getElementById("ai-model").value;
    const r = await fetch("/ai", {method:"POST", headers:{"Content-Type":"application/json"},
                                   body: JSON.stringify({path, model})});
```

- [ ] **Step 7: Allow-list the model in the `/ai` handler**

In `do_POST`'s `/ai` branch (`publish_next.py:530`), after `model = data.get("model") or self.ai_model`:

```python
                if model not in (self.ai_model, self.openrouter_model):
                    self._send(400, {"error": f"model not allowed: {model}"}); return
```

- [ ] **Step 8: Render both placeholders**

Replace the single render line (`publish_next.py:459`):

```python
        page = page.replace("__AI_MODEL__", html.escape(self.ai_model, quote=True))
        page = page.replace("__OPENROUTER_MODEL__", html.escape(self.openrouter_model, quote=True))
```

- [ ] **Step 9: Run the test + selfcheck to verify pass**

Run: `python -m pytest test_publish_next.py -v && python publish_next.py --selfcheck`
Expected: test PASS; selfcheck exits 0 (no syntax/import regressions).

- [ ] **Step 10: Commit**

```bash
git add publish_next.py test_publish_next.py
git commit -m "feat(publish-next): header selector for Claude vs OpenRouter free model"
```

---

### Task 4: End-to-end browser verification

**Files:** none (verification only).

**Interfaces:** consumes the full feature from Tasks 1-3.

- [ ] **Step 1: Launch the app against real data**

Run: `OPENROUTER_KEY=<key> nix run .#publish-next -- --data-dir <data-dir> -v`
Expected: gallery opens; the header shows the model `<select>` defaulting to *Claude (subscription)*.

- [ ] **Step 2: Verify the dropdown in the browser (playwright-cli skill)**

Using the `playwright-cli` skill against the gallery URL:
- Confirm `#ai-model` has both options with the configured values.
- Open a card's form, leave the select on Claude, click *Generate with AI* → title/description fill (existing key-free path still works).
- Switch the select to *OpenRouter (free)*, click *Generate with AI* on another card → title/description fill from the free model.
- Temporarily unset `OPENROUTER_KEY` (or pick an invalid model) and confirm the card's `.err` line shows a `RuntimeError`, not a silent hang or a fall-back to Claude.

Expected: all four hold. Capture a screenshot of the populated card for the record.

- [ ] **Step 3: Finish the branch**

Run `/simplify` and `/ponytail-review` on the diff; apply any trims. Then hand off per the finishing-a-development-branch skill (merge `feature/ai-model-selector` into `master` with `--no-ff`).

---

## Self-Review

**Spec coverage:**
- flake `llm-openrouter` + key-free/OPENROUTER_KEY → Task 2 + Global Constraints. ✓
- `--openrouter-model` vision default → Task 3 Step 3 + Global Constraints. ✓
- Header `<select>`, `aiGen` reads it → Task 3 Steps 5-6. ✓
- `/ai` allow-list → Task 3 Step 7. ✓
- `generate_metadata` provider branch (attachment vs Read) → Task 1. ✓
- Errors surface in `.err`, no silent fallback → Task 4 Step 2 (verified); mechanism unchanged from existing `/ai` error path. ✓
- Testing (openrouter argv, Read path preserved, schema emitted) → Task 1 Step 1 + retained existing tests. ✓
- Skipped items (per-card, auto-fallback, >2 providers, persistence) → not implemented, as specced. ✓

**Placeholder scan:** `<URL>`/`<VERSION>`/`<key>`/`<data-dir>` are explicit fill-from-command / user-supplied values with the exact command to obtain them, not vague TODOs. No "add error handling"-style gaps.

**Type consistency:** `openrouter_model` class attr, `--openrouter-model` flag, `self.openrouter_model` handler read, `__OPENROUTER_MODEL__` placeholder, and `openrouter/` prefix detection are named consistently across Tasks 1 and 3. `generate_metadata` signature unchanged throughout.
