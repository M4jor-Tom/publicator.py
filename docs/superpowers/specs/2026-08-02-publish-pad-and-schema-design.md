# Publish pad + hardcoded schema — design

Date: 2026-08-02
Scope: `publish_next.py` only.

Two independent changes to the gallery publish workflow.

## 1. Drop the `--schema` flag

The schema is a code asset that travels with the package (read from `PKG`,
not CWD or `--data-dir`), so it should not be a CLI flag. Make it a module
constant and remove all the plumbing.

- Add near `PKG` (line ~48): `SCHEMA_PATH = PKG / "publicationsSchema.json"`.
- Delete the `--schema` argparse line in `main()`.
- Delete `GalleryHandler.schema_path` class attribute.
- Delete `GalleryHandler.schema_path = args.schema` in `serve()`.
- `write_publications(entries, json_path)` — drop the `schema_path` parameter;
  read `SCHEMA_PATH` directly inside.
- Update the call site in the `/publish` handler to `write_publications(new, self.json_path)`.

Path note: the request said `./publicationSchema.json`, but the real file is
`publicationsSchema.json` (plural) and a CWD-relative `./` would break under
`--data-dir`. The constant stays package-relative (`PKG / …`) — the intent is
"always the bundled schema, no flag", not a literal working-directory path.

## 2. "Add to publish pad" button (keep session alive)

`write_publications()` already appends the queue to `publications.json` as
`state=unpublished`. Today it runs only inside `/publish`, immediately before
`publish_batch()` drives Firefox to post to DeviantArt. The new button runs
just the write half and leaves the gallery open.

### Server: new `/stage` endpoint

In `do_POST`, alongside `/publish`:

```python
elif self.path == "/stage":
    data = self._json_body()
    entries = data.get("entries", []) or []
    if not entries:
        self._send(400, {"error": "nothing to stage"}); return
    try:
        uuids = write_publications(entries, self.json_path)
    except Exception as e:
        self._send(400, {"error": f"schema/write failed: {e}"}); return
    self._send(200, {"staged": len(uuids), "uuids": uuids})
```

Crucially it does **not** set `GalleryHandler.publish_done`, so the
`while publish_done is None` serve loop keeps running — the session stays alive.

Chosen over a `mode` flag on `/publish` because it keeps the terminal
publish path (sets `publish_done`, exits) cleanly separate from the
non-terminal stage path.

### Frontend

- Header: add `<button id="stage-btn" onclick="stageQueue()" disabled>Add to publish pad</button>`
  next to the Publish button. Reuse existing button styling.
- `stageQueue()`:
  1. `const fresh = queue.filter(e => !e.uuid);` (already-pending entries carry
     a uuid and are already persisted — skip them).
  2. If empty, return.
  3. POST `fresh.map(({cardId, ...rest}) => rest)` to `/stage`.
  4. On success, zip `d.uuids` back onto the `fresh` objects by index
     (`fresh[i].uuid = d.uuids[i]`). Because `fresh` holds references into
     `queue`, this promotes them to "existing" in place.
  5. For each staged card: change its badge text `queued` → `on pad` and
     disable its Delete button (a persisted entry must not be deletable from
     disk here — that would orphan the JSON row).
  6. Update a `#stage-status` span in the header to read `N on pad`.
- `refreshCount()` also sets `stage-btn.disabled = queue.filter(e => !e.uuid).length === 0`.

### Interaction with Publish

After staging, every queue entry has a uuid, so a later **Publish** sends them
all as `existing` (the `e.get("uuid")` branch in `/publish`) → `publish_batch`
handles them, no double-write. Mixed sessions (some staged, some fresh) work:
`/publish` still splits `existing` vs `new` and writes only the new ones.

## Testing

- Extend `_selfcheck()` (or add a `__main__` assertion) covering the new
  `write_publications(entries, json_path)` signature: write a temp entry, assert
  it lands in `publications.json` with `state == STATE_UNPUBLISHED` and that the
  returned list has one uuid matching the written row. This also guards the
  schema-constant change (validation must still pass).
- UI (button enable/disable, badge → `on pad`, session stays open, subsequent
  Publish) verified live via the `playwright-cli` skill after implementation.

## Out of scope

- No rename of `publicationsSchema.json`.
- No change to `validate.py` or `llm_meta.py` (their `--schema` usage is unrelated).
- No persistence of AI-model selection or other gallery state.
