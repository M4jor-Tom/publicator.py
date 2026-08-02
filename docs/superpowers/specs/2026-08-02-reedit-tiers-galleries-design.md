# Re-edit queued publications + tier/galleries + config.toml

Date: 2026-08-02

## Summary

Three coupled changes to the publish workflow:

1. **Re-edit** entries already in the RAM queue or persisted to the publish pad
   (`publications.json`), from the gallery UI — including a **price** input.
2. **`config.toml`** (in cwd) declares the allowed DeviantArt **tiers** and
   **galleries**. Two new optional apparition fields (`tier`, `galleries`)
   carry the choice; validation enforces DA-only + values-in-config.
3. **DA submit steps** for tier and galleries, driven in the browser during
   publish (same unverified-selector caveat as premium/description).

Decisions (from brainstorming): also add the submit steps now · flat
`[deviantart]` lists in config · `tier`=string + `galleries`=list, both
optional and DA-only · config read from **cwd** · persisted cards lose the
RAM "Remove from queue" button (Edit only) · galleries = multi-pick
checkboxes · a placeholder `config.toml` is created in `../Art/`.

## 1. Schema — `publicationsSchema.json`

Add to `Apparition.properties`:
- `tier`: `{ "type": "string" }`
- `galleries`: `{ "type": "array", "items": { "type": "string" } }`

Both optional. Forbid them on non-DA apparitions with a conditional:

```json
"if":   { "properties": { "platformName": { "const": "deviantart" } } },
"else": { "properties": { "tier": false, "galleries": false } }
```

(`false` as a property schema forbids the property when present.) The
values-in-config rule is **not** expressible in JSON Schema → custom Python.

## 2. `config.toml` + validation — refactor `validate.py`

`validate.py` currently executes at import. Refactor into importable functions
behind a `main()` guard:

- `load_config(cwd) -> {"tiers": [...], "galleries": [...]}` — reads
  `<cwd>/config.toml`, section `[deviantart]`. Missing file → empty lists.
- `validate_publications(data, config)` — JSON-Schema validate, **then** for
  each `deviantart` apparition: `tier` (if present) ∈ `config.tiers`, every
  gallery ∈ `config.galleries`; raise `ValueError` otherwise.

Config is read from **cwd**; publications from `--data-dir`. `tomllib`
(stdlib, py3.11+) — no new dependency.

Single validation path: `validate.py` **and** `publish_next` (`/stage` via
`write_publications`, and the new `/update`) both call `validate_publications`.

Placeholder `../Art/config.toml`:

```toml
[deviantart]
# Replace with your real DeviantArt subscription tiers and gallery folder names.
tiers = ["free", "premium", "exclusive"]
galleries = ["Portraits", "Landscapes", "NSFW"]
```

## 3. DA submit steps — `da_publish.py` + `SKILL.md`

Two new conditional steps (skip when the field is absent), inserted after
premium and before schedule. Best-guess selectors as single-point-of-fix
constants, **unverified** (same caveat as `PREMIUM_*` / `DESCRIPTION_SELECTOR`):

- `_step_tier(page, e)` — `TIER_SELECTOR`; select `e["tier"]`.
- `_step_galleries(page, e)` — `GALLERY_*`; add the deviation to each of
  `e["galleries"]`.

`tier`/`galleries` plumbed into the entry dict from the apparition
(`load_pending_entries`) and from the gallery queue, exactly like `price`.
`STEPS` ↔ `SKILL.md` stay 1:1 (drift guard → **12 steps**). New step order:

```
1 goto · 2 submit · 3 upload · 4 title · 5 description · 6 checkboxes
7 clear-tags · 8 add-tags · 9 premium · 10 tier · 11 galleries · 12 schedule
```

## 4. Gallery UI — `publish_next.py`

### Config into the page
`serve()` calls `load_config(Path.cwd())`, stores it on `GalleryHandler.config`,
and injects `tiers`/`galleries` into the page. When both lists are empty the
tier/galleries inputs are omitted.

### One shared edit form (used for Add and Edit)
Fields: title, description, **price**, schedule, **tier `<select>`** (config
options + a "(none)" entry), **galleries checkboxes** (config options).
`openForm(cardId)` pre-fills from the card's queue entry when one exists
(edit), else defaults the schedule slot (new).

### Buttons per card state
- **Candidate (unsaved):** View · Add (openForm) · Delete (`os.remove`).
- **Saved to RAM queue (no uuid):** View · Edit (openForm) · Delete
  (`os.remove`) — not yet persisted, so file-delete still discards it.
- **Persisted (has uuid — staged or pre-loaded pending):** View · Edit.
  **No remove button** (removing from RAM is pointless — it persists in
  `publications.json`).

### `saveCard` branches
- New card → push to `queue`, mark queued, swap Add→Edit.
- Existing entry → `Object.assign` the queue entry; if it has a `uuid`, POST
  **`/update`** and reflect success/error; refresh the card's title/schedule
  display.

### New endpoint `/update`
Body `{uuid, title, description, price, scheduleTs, tier, galleries}`. Loads
`publications.json`, finds the entry by uuid, patches:
- `description` → publication
- title → `apparition.urlElsePublicationName`
- schedule → `apparition.apparitionTimestampIfDifferentThanSubmission`
- price/tier/galleries → set on the apparition when provided, **removed when
  cleared**

Then `validate_publications(pubs, config)` and atomic write. 404 on unknown
uuid. Uses `deviantart_apparition` (imported from the echo module).

### Data plumbing
`load_pending_entries` and the page's `pending_js` / `PENDING` push gain
`tier` + `galleries` (like `description`/`price` already do). `write_publications`
writes `tier`/`galleries` onto the apparition when present.

## Verification (self-checks, no new frameworks)

- `da_publish --check-steps` → 12 in sync.
- `da_publish --selfcheck` → `_step_tier`/`_step_galleries` are no-ops on an
  entry lacking the field (sentinel `_NoPage` raises if the page is touched).
- `publish_next --selfcheck` → `write_publications` round-trip now includes
  `tier`/`galleries` on the apparition; plus a negative `validate_publications`
  case (tier not in config → `ValueError`).
- `validate.py --data-dir ../Art` → migrated data (no tier/galleries) still
  `Valid` against the new schema + placeholder config.

## Out of scope / non-goals

- No migration of existing data (tier/galleries are optional; the 92 entries
  omit them and stay valid).
- No delete-from-pad path (removing the RAM button is the whole ask; entries
  persist in `publications.json` by design).
- Editing is offered only for `state=unpublished` entries (the only ones the
  pad surfaces).
