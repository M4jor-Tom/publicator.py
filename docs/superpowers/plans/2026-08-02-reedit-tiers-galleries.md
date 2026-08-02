# Re-edit queue + tier/galleries + config.toml — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the gallery re-edit queued/pad publications (incl. price), add DeviantArt `tier`/`galleries` fields governed by a cwd `config.toml`, validate them, and drive them in the DA submit flow.

**Architecture:** JSON-Schema gains two optional DA-only apparition fields; a refactored `validate.py` exposes `load_config`/`validate_publications` (schema + config allow-list) reused by the gallery backend. `da_publish` gets two conditional submit steps. `publish_next` reads `config.toml`, renders one shared edit form on every card, and patches `publications.json` via a new `/update` endpoint.

**Tech Stack:** Python 3.12 (stdlib `tomllib`), `jsonschema`, Playwright (Firefox), stdlib `http.server`. Run everything under `nix develop --command bash -c '…'`.

## Global Constraints

- No new dependencies — `tomllib` is stdlib (py3.11+); no `toml`/`tomli` package.
- `config.toml` is read from **cwd**; `publications.json` from `--data-dir`.
- Testing convention: inline `--selfcheck` / `--check-steps` + plain-assert `test_publish_next.py`. **No pytest.** Run checks with `nix develop --command`.
- `tier` = single string; `galleries` = list of strings. Both **optional**, allowed **only** on `platformName == "deviantart"` apparitions.
- `STEPS` in `da_publish.py` must stay 1:1 with `SKILL.md`'s numbered steps (the `--check-steps` drift guard enforces it).
- DA submit selectors that can't be verified offline are best-guess module constants, **unverified** (same contract as `PREMIUM_*` / `DESCRIPTION_SELECTOR`): a miss trips the step and falls through to manual.
- Commit style: conventional commits, `--no-ff` not relevant (direct commits on `master`, repo convention). End messages with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

---

### Task 1: Schema — DA-only `tier`/`galleries`

**Files:**
- Modify: `publicationsSchema.json` (`$defs.Apparition`)

**Interfaces:**
- Produces: apparition may carry `tier` (string) and `galleries` (array of strings), only when `platformName == "deviantart"`.

- [ ] **Step 1: Add the fields + conditional.** Replace the `Apparition` definition's `properties` tail and add `if`/`else` so it reads:

```json
"Apparition": {
    "type": "object",
    "required": ["platformName", "urlElsePublicationName", "state"],
    "properties": {
        "platformName": { "type": "string", "enum": ["deviantart", "pixiv"] },
        "state": { "type": "string", "enum": ["unpublished", "published_or_scheduled"] },
        "urlElsePublicationName": { "type": "string" },
        "apparitionTimestampIfDifferentThanSubmission": { "type": "integer" },
        "priceIfNotFree": { "type": "number", "minimum": 0 },
        "tier": { "type": "string" },
        "galleries": { "type": "array", "items": { "type": "string" } }
    },
    "if": { "properties": { "platformName": { "const": "deviantart" } } },
    "else": { "properties": { "tier": false, "galleries": false } }
}
```

- [ ] **Step 2: Verify.** DA apparition with tier/galleries passes; pixiv apparition with tier fails.

```bash
nix develop --command python -c '
import json
from jsonschema import validate, ValidationError
S=json.load(open("publicationsSchema.json"))
base=lambda app:[{"uuid":"x"*36,"state":None,"submissionTimestamp":1,"description":"d","files":[],"apparitions":[app]}]
# state lives on apparition now; give a valid DA one
da={"platformName":"deviantart","urlElsePublicationName":"t","state":"unpublished","tier":"free","galleries":["G"]}
validate(base(da),S); print("DA tier/galleries OK")
px={"platformName":"pixiv","urlElsePublicationName":"t","state":"unpublished","tier":"free"}
try: validate(base(px),S); print("BUG: pixiv tier allowed")
except ValidationError: print("pixiv tier rejected OK")
'
```
Expected: `DA tier/galleries OK` then `pixiv tier rejected OK`.

- [ ] **Step 3: Commit.**

```bash
git add publicationsSchema.json
git commit -m "feat(schema): DA-only tier/galleries apparition fields

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `validate.py` refactor + placeholder config

**Files:**
- Modify: `validate.py` (full rewrite into functions + `main()` guard)
- Create: `../Art/config.toml` (placeholder, outside this repo)

**Interfaces:**
- Produces: `load_config(cwd=".") -> {"tiers": list[str], "galleries": list[str]}`; `validate_publications(data, config) -> None` (raises `jsonschema.ValidationError` or `ValueError`).
- Consumes: schema from Task 1.

- [ ] **Step 1: Rewrite `validate.py`.**

```python
import argparse
import json
import tomllib
from pathlib import Path

from jsonschema import validate as _js_validate

SCHEMA = Path(__file__).resolve().parent / "publicationsSchema.json"


def load_config(cwd="."):
    """DeviantArt tier/gallery allow-lists from <cwd>/config.toml [deviantart].
    Missing file -> empty lists."""
    f = Path(cwd) / "config.toml"
    if not f.exists():
        return {"tiers": [], "galleries": []}
    da = tomllib.loads(f.read_text()).get("deviantart", {})
    return {"tiers": list(da.get("tiers", [])), "galleries": list(da.get("galleries", []))}


def validate_publications(data, config):
    """JSON-Schema validate, then enforce DA apparition tier/gallery values
    against config (the schema already forbids these fields on non-DA)."""
    with open(SCHEMA) as s:
        _js_validate(instance=data, schema=json.load(s))
    tiers, galleries = set(config.get("tiers", [])), set(config.get("galleries", []))
    for p in data:
        for a in p.get("apparitions", []):
            if a.get("platformName") != "deviantart":
                continue
            if "tier" in a and a["tier"] not in tiers:
                raise ValueError(f"{p.get('uuid')}: tier {a['tier']!r} not in config.toml")
            for g in a.get("galleries", []):
                if g not in galleries:
                    raise ValueError(f"{p.get('uuid')}: gallery {g!r} not in config.toml")


def main():
    ap = argparse.ArgumentParser(description="Validate publications.json against the schema.")
    ap.add_argument("--data-dir", default=None, help="publication database dir; default: CWD")
    args = ap.parse_args()
    data_dir = Path(args.data_dir).resolve() if args.data_dir else Path.cwd()
    data = json.loads((data_dir / "publications.json").read_text())
    validate_publications(data, load_config(Path.cwd()))
    print("Valid")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Create placeholder `../Art/config.toml`.**

```toml
[deviantart]
# Replace with your real DeviantArt subscription tiers and gallery folder names.
tiers = ["free", "premium", "exclusive"]
galleries = ["Portraits", "Landscapes", "NSFW"]
```

- [ ] **Step 3: Verify.** Migrated data (no tier/galleries) still validates; a bad tier raises.

```bash
nix develop --command bash -c '
python validate.py --data-dir ../Art
python -c "
from validate import load_config, validate_publications
c=load_config(\"../Art\"); print(\"config:\",c)
good=[{\"uuid\":\"u\",\"submissionTimestamp\":1,\"description\":\"d\",\"files\":[],\"apparitions\":[{\"platformName\":\"deviantart\",\"urlElsePublicationName\":\"t\",\"state\":\"unpublished\",\"tier\":\"free\",\"galleries\":[\"Portraits\"]}]}]
validate_publications(good,c); print(\"good OK\")
bad=[dict(good[0])]; bad[0][\"apparitions\"]=[dict(good[0][\"apparitions\"][0], tier=\"nope\")]
try: validate_publications(bad,c); print(\"BUG: bad tier accepted\")
except ValueError as e: print(\"bad tier rejected:\", e)
"
'
```
Expected: `Valid`, `config: {...}`, `good OK`, `bad tier rejected: ...`.

- [ ] **Step 4: Commit** (repo only — `../Art` is a separate repo; note the placeholder was created there).

```bash
git add validate.py
git commit -m "refactor(validate): importable load_config + validate_publications

Adds config.toml (cwd) tier/gallery allow-list enforcement on top of the
JSON schema; reused by the gallery backend. Placeholder ../Art/config.toml
seeded separately.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `da_publish.py` tier/galleries submit steps

**Files:**
- Modify: `da_publish.py` (constants + steps + `STEPS` + `load_pending_entries` + `_selfcheck`)
- Modify: `.claude/skills/publish-deviantart/SKILL.md` (steps 10–11)

**Interfaces:**
- Consumes: entry dict may carry `tier: str|None`, `galleries: list[str]`.
- Produces: `STEPS` has 12 entries; `load_pending_entries` output includes `tier`, `galleries`.

- [ ] **Step 1: Add constants + steps** (place directly before `def _step_schedule`):

```python
TIER_SELECTOR = 'select[name="premiumFolderTier"]'
GALLERY_ADD_BUTTON = 'button[aria-label="Add to Gallery"]'


def _step_tier(page, e):
    tier = e.get("tier")
    if not tier:
        return
    page.select_option(TIER_SELECTOR, label=tier)


def _step_galleries(page, e):
    for g in e.get("galleries") or []:
        page.locator(GALLERY_ADD_BUTTON).first.click()
        page.wait_for_timeout(300)
        page.get_by_text(g, exact=True).first.click()
        page.wait_for_timeout(200)
```

- [ ] **Step 2: Wire into `STEPS`** (insert the two lines between premium and schedule):

```python
    ('If the piece has a price, tick "Submit as Premium Download" and set the price', _step_premium),
    ("Set the subscription tier <pub.tier>", _step_tier),
    ("Add to galleries <pub.galleries>", _step_galleries),
    ("Schedule publication for the <pub.schedule>", _step_schedule),
```

- [ ] **Step 3: Plumb into `load_pending_entries`** (add two keys to the `out.append({...})` dict):

```python
            "price": app.get("priceIfNotFree"),
            "tier": app.get("tier"),
            "galleries": app.get("galleries", []),
```

- [ ] **Step 4: Extend `_selfcheck`** (after the existing premium no-op guard, reuse `_NoPage`):

```python
    _step_tier(_NoPage(), {})        # no tier -> no-op
    _step_galleries(_NoPage(), {})   # no galleries -> no-op
```

- [ ] **Step 5: Update `SKILL.md`** steps to (premium is 9):

```markdown
9. If the piece has a price, tick "Submit as Premium Download" and set the price
10. Set the subscription tier `<pub.tier>`
11. Add to galleries `<pub.galleries>`
12. Schedule publication for the `<pub.schedule>`
```

- [ ] **Step 6: Verify.**

```bash
nix develop --command bash -c 'python da_publish.py --selfcheck && python da_publish.py --check-steps'
```
Expected: `check-steps OK: 12 steps in sync with the skill` and `selfcheck OK`.

- [ ] **Step 7: Commit.**

```bash
git add da_publish.py .claude/skills/publish-deviantart/SKILL.md
git commit -m "feat(publish): DA tier + galleries submit steps

CAVEAT: TIER_SELECTOR / GALLERY_ADD_BUTTON are unverified best-guesses;
first run may trip steps 10-11 and fall through to manual, where the two
constants get fixed against the live form.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: `publish_next.py` backend — write, validate, `/update`

**Files:**
- Modify: `publish_next.py` (imports, `write_publications`, new `apply_update`, `/update` handler, `GalleryHandler.config`, `serve`, `_selfcheck`)

**Interfaces:**
- Consumes: `load_config`, `validate_publications` (Task 2); `deviantart_apparition` (echo module).
- Produces: `apply_update(pubs, uuid_, fields) -> None` (raises `KeyError` if uuid absent); `/update` endpoint; entries persist `tier`/`galleries`.

- [ ] **Step 1: Imports.** Add to the `from da_publish import (...)` neighbourhood:

```python
from echo_first_unpublished_publication_data import deviantart_apparition
from validate import load_config, validate_publications
```

- [ ] **Step 2: `write_publications`** — add tier/galleries to the apparition and swap the inline jsonschema validate for `validate_publications`:

```python
        if "price" in e and e["price"] is not None:
            apparition["priceIfNotFree"] = float(e["price"])
        if e.get("tier"):
            apparition["tier"] = e["tier"]
        if e.get("galleries"):
            apparition["galleries"] = list(e["galleries"])
```
Replace the trailing validate block (`from jsonschema import validate ... validate(instance=data, schema=schema)`) with:

```python
    validate_publications(data, load_config(Path.cwd()))
    _atomic_write_json(json_path, data)
    return new_uuids
```

- [ ] **Step 3: Add `apply_update` + helper** (module-level, near `write_publications`):

```python
def _set_or_pop(d, k, v):
    if v in (None, [], ""):
        d.pop(k, None)
    else:
        d[k] = v


def apply_update(pubs, uuid_, fields):
    """Patch the publication (and its DA apparition) with uuid_ in place.
    Cleared price/tier/galleries are removed. Raises KeyError if not found."""
    for p in pubs:
        if p.get("uuid") == uuid_:
            break
    else:
        raise KeyError(uuid_)
    p["description"] = fields.get("description", p.get("description", ""))
    app = deviantart_apparition(p)
    app["urlElsePublicationName"] = fields["title"]
    app["apparitionTimestampIfDifferentThanSubmission"] = int(fields["scheduleTs"])
    price = fields.get("price")
    _set_or_pop(app, "priceIfNotFree", float(price) if price not in (None, "") else None)
    _set_or_pop(app, "tier", fields.get("tier") or None)
    _set_or_pop(app, "galleries", list(fields["galleries"]) if fields.get("galleries") else None)
```

- [ ] **Step 4: `/update` handler** (add an `elif` in `do_POST`, before the final `else`):

```python
            elif self.path == "/update":
                data = self._json_body()
                u = data.get("uuid")
                if not u:
                    self._send(400, {"error": "missing uuid"}); return
                with open(self.json_path) as f:
                    pubs = json.load(f)
                try:
                    apply_update(pubs, u, data)
                except KeyError:
                    self._send(404, {"error": f"uuid not found: {u}"}); return
                try:
                    validate_publications(pubs, self.config)
                except Exception as e:
                    self._send(400, {"error": f"invalid: {e}"}); return
                _atomic_write_json(self.json_path, pubs)
                self._send(200, {"ok": True})
```

- [ ] **Step 5: Handler config attr + `serve` wiring.** Add `config: dict = {"tiers": [], "galleries": []}` to `GalleryHandler`'s class attrs, and in `serve()` add:

```python
    GalleryHandler.config = load_config(Path.cwd())
```

- [ ] **Step 6: Extend `_selfcheck`** (append inside the existing `with tempfile.TemporaryDirectory()` block, after the state assert):

```python
        import copy
        ok = copy.deepcopy(_rows)
        ok[0]["apparitions"][0]["tier"] = "gold"; ok[0]["apparitions"][0]["galleries"] = ["Art"]
        validate_publications(ok, {"tiers": ["gold"], "galleries": ["Art"]})
        try:
            validate_publications(ok, {"tiers": [], "galleries": []}); assert False, "tier not rejected"
        except ValueError:
            pass
        apply_update(_rows, _uuids[0], {"title": "T2", "description": "D2", "scheduleTs": s1,
                                        "price": 3, "tier": "gold", "galleries": ["Art"]})
        a = _rows[0]["apparitions"][0]
        assert a["urlElsePublicationName"] == "T2" and a["priceIfNotFree"] == 3.0
        assert a["tier"] == "gold" and a["galleries"] == ["Art"], a
        apply_update(_rows, _uuids[0], {"title": "T3", "description": "D3", "scheduleTs": s1})
        a = _rows[0]["apparitions"][0]
        assert "priceIfNotFree" not in a and "tier" not in a and "galleries" not in a, a
```

- [ ] **Step 7: Verify.**

```bash
nix develop --command python publish_next.py --selfcheck
```
Expected: `selfcheck OK`.

- [ ] **Step 8: Commit.**

```bash
git add publish_next.py
git commit -m "feat(gallery): persist tier/galleries + /update endpoint

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `publish_next.py` UI — re-edit every card

**Files:**
- Modify: `publish_next.py` (page template, `_build_page`, form/JS, pending plumbing)
- Modify: `test_publish_next.py` (config render assertions)

**Interfaces:**
- Consumes: `GalleryHandler.config` (Task 4); pending entries now carry `tier`/`galleries`.
- Produces: every card has an edit form (title/description/price/schedule/tier/galleries); persisted cards have View+Edit only; edits POST `/update`.

- [ ] **Step 1: Inject config into the page.** In `_PAGE_TMPL` `<script>`, after `const PENDING = __PENDING__;` add:

```javascript
const TIERS = __TIERS__;
const GALLERIES = __GALLERIES__;
```
In `_build_page`, before `return page`, add:

```python
        page = page.replace("__TIERS__", json.dumps(self.config.get("tiers", [])))
        page = page.replace("__GALLERIES__", json.dumps(self.config.get("galleries", [])))
```

- [ ] **Step 2: Shared tier/gallery form fields.** Add a module helper and call it once in `_build_page`:

```python
def _tier_gallery_fields(config):
    if not config.get("tiers") and not config.get("galleries"):
        return ""
    tiers = "".join(f'<option value="{html.escape(t, quote=True)}">{html.escape(t)}</option>'
                    for t in config.get("tiers", []))
    gals = "".join(f'<label><input type="checkbox" class="f-gallery" '
                   f'value="{html.escape(g, quote=True)}"> {html.escape(g)}</label>'
                   for g in config.get("galleries", []))
    return (f'<label>Tier <select class="f-tier"><option value="">(none)</option>{tiers}</select></label>'
            f'<div class="galleries"><span>Galleries</span>{gals}</div>')
```
In `_build_page`: `tg = _tier_gallery_fields(self.config)`.

- [ ] **Step 3: Candidate card form** — insert `{tg}` into the candidate `.form` (between the Schedule label and the AI button), and change the post-save flow to expose Edit. The candidate card's form block becomes:

```python
  <div class="form">
    <label>Title <input type="text" class="f-title" maxlength="50"></label>
    <label>Description <textarea class="f-description"></textarea></label>
    <label>Price (optional) <input type="number" class="f-price" min="0" step="0.01"></label>
    <label>Schedule <input type="datetime-local" class="f-schedule"></label>
    {tg}
    <button class="btn-ai" onclick="aiGen('{cid}', {js_path})">Generate with AI</button>
    <div class="err"></div>
    <div class="form-actions">
      <button class="btn-save" onclick="saveCard('{cid}', {js_path})">Save to queue</button>
      <button class="btn-cancel" onclick="closeForm('{cid}')">Cancel</button>
    </div>
  </div>
```

- [ ] **Step 4: Pending card gets the same form + Edit, loses Remove.** Replace the pending card block so it renders View + Edit and the full form (pre-filled by JS on open):

```python
            cards.append(f"""<div class="card queued" id="{cid}">
  <img src="/thumbs/{safe_rel}" alt="{safe_title}">
  <div class="name"><span class="badge">queued</span> {safe_title}</div>
  <div class="sched">{html.escape(sched)}</div>
  <div class="actions">
    <a class="btn-view" href="{view_href}" target="_blank" rel="noopener">View</a>
    <button class="btn-add" onclick="openForm('{cid}')">Edit</button>
  </div>
  <div class="form">
    <label>Title <input type="text" class="f-title" maxlength="50"></label>
    <label>Description <textarea class="f-description"></textarea></label>
    <label>Price (optional) <input type="number" class="f-price" min="0" step="0.01"></label>
    <label>Schedule <input type="datetime-local" class="f-schedule"></label>
    {tg}
    <button class="btn-ai" onclick="aiGen('{cid}', {js_pathp})">Generate with AI</button>
    <div class="err"></div>
    <div class="form-actions">
      <button class="btn-save" onclick="saveCard('{cid}', {js_pathp})">Save changes</button>
      <button class="btn-cancel" onclick="closeForm('{cid}')">Cancel</button>
    </div>
  </div>
</div>""")
```
where `js_pathp = html.escape(json.dumps(e["path"]), quote=True)` (compute it above, mirroring the candidate loop's `js_path`).

- [ ] **Step 5: Pending JS plumbing.** In `pending_js.append({...})` add `"tier": e.get("tier"), "galleries": e.get("galleries", [])`. In the `PENDING` loop's `queue.push({...})` add `tier: p.tier, galleries: p.galleries`. In the top `const queue` comment, note the new fields.

- [ ] **Step 6: `openForm` pre-fills for edit.** Replace `openForm` with:

```javascript
function openForm(cardId) {
  const card = document.getElementById(cardId);
  const form = card.querySelector(".form");
  const entry = queue.find(e => e.cardId === cardId);
  const q = sel => form.querySelector(sel);
  if (entry) {
    q(".f-title").value = entry.title || "";
    q(".f-description").value = entry.description || "";
    q(".f-price").value = (entry.price == null) ? "" : entry.price;
    q(".f-schedule").value = entry.scheduleTs ? tsToLocalInput(entry.scheduleTs) : "";
    if (q(".f-tier")) q(".f-tier").value = entry.tier || "";
    form.querySelectorAll(".f-gallery").forEach(cb => cb.checked = (entry.galleries || []).includes(cb.value));
  } else {
    q(".f-schedule").value = tsToLocalInput(slotForIndex(INITIAL_MAX_TS, queue.length));
  }
  form.style.display = "flex";
  card.querySelector(".actions").style.display = "none";
}
```

- [ ] **Step 7: `saveCard` branches new vs edit.** Replace `saveCard` with:

```javascript
function readForm(card) {
  const g = sel => card.querySelector(sel);
  const title = g(".f-title").value.trim();
  const desc = g(".f-description").value.trim();
  const priceRaw = g(".f-price").value.trim();
  const sched = g(".f-schedule").value;
  const err = g(".err");
  if (!title) { err.textContent = "title required"; return null; }
  if (!desc) { err.textContent = "description required"; return null; }
  if (!sched) { err.textContent = "schedule required"; return null; }
  const out = {title, description: desc, scheduleTs: localInputToTs(sched)};
  if (priceRaw !== "") {
    const p = parseFloat(priceRaw);
    if (isNaN(p) || p < 0) { err.textContent = "price must be >= 0"; return null; }
    out.price = p;
  }
  const tierEl = g(".f-tier");
  if (tierEl && tierEl.value) out.tier = tierEl.value;
  const gals = [...card.querySelectorAll(".f-gallery:checked")].map(cb => cb.value);
  if (gals.length) out.galleries = gals;
  return out;
}

async function saveCard(cardId, path) {
  const card = document.getElementById(cardId);
  const fields = readForm(card);
  if (!fields) return;
  const entry = queue.find(e => e.cardId === cardId);
  if (entry) {                       // re-edit
    Object.assign(entry, {price: undefined, tier: undefined, galleries: undefined}, fields);
    if (entry.uuid) {
      const btn = card.querySelector(".btn-save");
      btn.disabled = true; btn.textContent = "Saving...";
      const r = await fetch("/update", {method:"POST", headers:{"Content-Type":"application/json"},
                                        body: JSON.stringify({uuid: entry.uuid, ...fields})});
      const d = await r.json();
      btn.disabled = false; btn.textContent = "Save changes";
      if (!r.ok || d.error) { card.querySelector(".err").textContent = d.error || ("HTTP " + r.status); return; }
    }
    card.querySelector(".name").lastChild.textContent = " " + fields.title;
  } else {                           // first save of a candidate
    queue.push({cardId, path, ...fields});
    card.classList.add("queued");
    const add = card.querySelector(".btn-add");
    add.textContent = "Edit"; add.disabled = false; add.onclick = () => openForm(cardId);
    if (!card.querySelector(".badge")) {
      const b = document.createElement("span"); b.className = "badge"; b.textContent = "queued";
      card.querySelector(".name").prepend(b, " ");
    }
  }
  card.querySelector(".form").style.display = "none";
  card.querySelector(".actions").style.display = "flex";
  refreshCount();
}
```

- [ ] **Step 8: On stage, drop the delete button for now-persisted cards.** In `stageQueue`, replace the `del.textContent = "Remove from queue"...` block with removal:

```javascript
    const card = document.getElementById(e.cardId);
    const del = card && card.querySelector(".btn-del");
    if (del) del.remove();   // persisted in publications.json -> no RAM remove
```
Delete the now-unused `unqueuePending` only if nothing else references it (pending cards no longer call it). Grep before removing.

- [ ] **Step 9: Test assertions.** Add to `test_publish_next.py`:

```python
def test_page_renders_config_tiers_galleries():
    H = publish_next.GalleryHandler
    H.thumb_dir = "/t"; H.thumb_map = {}; H.candidate_paths = ["/t/a.png"]; H.pending = []
    H.initial_max_ts = 0; H.ai_model = "m"; H.openrouter_model = "o"
    H.config = {"tiers": ["gold"], "galleries": ["Art"]}
    page = publish_next.GalleryHandler._build_page(H)
    assert 'class="f-tier"' in page and '>gold<' in page
    assert 'class="f-gallery"' in page and 'value="Art"' in page
    assert 'const TIERS = ["gold"]' in page
```

- [ ] **Step 10: Verify.**

```bash
nix develop --command bash -c '
python publish_next.py --selfcheck
python -c "import test_publish_next as t; t.test_page_offers_both_models(); t.test_page_renders_config_tiers_galleries(); print(\"tests PASS\")"
'
```
Expected: `selfcheck OK` then `tests PASS`.

- [ ] **Step 11: Commit.**

```bash
git add publish_next.py test_publish_next.py
git commit -m "feat(gallery): re-edit queued/pad cards incl. price + tier/galleries

Persisted (uuid) cards lose the RAM Remove button (Edit only); edits POST
/update. Config tiers/galleries render as a select + checkboxes.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:** re-edit (T5) · price input (T5 form) · config.toml cwd (T2) · schema tier/galleries (T1) · DA-only + values-in-config validation (T1 schema + T2 custom) · DA submit steps (T3) · remove RAM button for persisted (T5 Step 8) · placeholder config (T2). All covered.

**Placeholder scan:** none — every step has concrete code/commands.

**Type consistency:** `load_config`/`validate_publications`/`apply_update` signatures match across T2/T4; entry keys `tier`/`galleries` consistent across T3/T4/T5; `GalleryHandler.config` set in T4, read in T5; `js_pathp` defined where used (T4 Step 4).

**Known live-UI risk:** T3 selectors and T5 gallery-add interaction are unverified best-guesses (documented caveat). T5 `card.querySelector(".name").lastChild` assumes the title is the last text node — acceptable; verify in the render test / browser.
