# Unified Publish Workflow — Design

**Date:** 2026-07-19
**Status:** Approved (brainstorm), pending implementation

## Goal

Replace the three-step pipeline (`publish_next.py` → `name_publications.py` → `publish_deviantart.py`) with a single command: **`publish_next.py`**. One browser window handles candidate selection, per-image metadata entry (including AI-generated title/description), scheduling override, and batch publication via a separate Playwright Chromium session.

## User Flow

1. `python publish_next.py [-n 10]` — scans `picked/`, hashes, filters against `publications.json`, generates thumbnails, opens Firefox to `http://127.0.0.1:8765`.
2. Gallery shows candidate cards. Each card has two actions: **Delete** (hard `os.remove`, card disappears) and **Add** (expands the card into a form).
3. Form fields: title, description, optional price, schedule (default = next slot from `compute_slots`-style packing), and a **Generate with AI** button.
4. User builds the queue by adding N cards. Queue lives in browser memory; no writes to `publications.json` yet.
5. User clicks **Publish**. Server:
   - Writes all queued entries to `publications.json` atomically (one write, schema-validated).
   - Launches a **separate Playwright Chromium** with a persistent context at `./.deviantart-session/` (gitignored). If context is empty/missing, launches headed so user can log in; on close, cookies persist. Subsequent runs use the saved session.
   - Iterates entries in order, running the DA submission flow (port of `publish_deviantart.py`).
   - On each success, flips `state` to `published_or_scheduled`.
   - **On first failure: stops.** Remaining unpublished entries stay in `publications.json` with `state="unpublished"` for the next run.
6. Result summary rendered in the same browser tab. User closes tab.

## Architecture

Single script `publish_next.py`. Layout:

- **Gallery HTTP server** (stdlib `http.server`) — extends the current handler:
  - `GET /` — cards with Delete/Add controls + form template
  - `POST /delete` — `{ path }` → `os.remove(path)` → `{ ok: true }`
  - `POST /ai` — `{ path }` → returns `{ title, description }` via provider
  - `POST /publish` — `{ entries: [...] }` → writes JSON, runs publish loop, streams status back
- **Playwright module** — the `run(pub)` function from `publish_deviantart.py`, ported to Chromium + persistent context. Called in-process (blocking) from `POST /publish`.
- **AI module** — `generate_metadata(image_path, provider, model)` → `(title, description)`. Only Claude implemented (uses `anthropic` SDK, vision message with base64 image). Provider dispatch table so adding OpenAI is a small addition.
- **Legacy** — `name_publications.py` deleted. `publish_deviantart.py` kept for `--loop` mode use. `publish.py` untouched (manual add tool, orthogonal).

## Data & Files

- **`publications.json`** — unchanged schema. `description` on the Publication object gets the AI/user description. `priceIfNotFree` on the deviantart apparition holds the optional price. `urlElsePublicationName` holds the title.
- **`./.deviantart-session/`** — Playwright Chromium persistent context. Added to `.gitignore`.
- **Deletion** — `os.remove(path)` on the original image path from `picked/`. No trash, no confirmation dialog on the server side (browser JS confirms).

## CLI

```
publish_next.py [-n 10] [--picked-dir picked] [--json publications.json]
                [--ai-provider claude] [--ai-model claude-opus-4-7]
                [--port 8765]
```

Environment: `ANTHROPIC_API_KEY` for the AI feature. If missing, the "Generate with AI" button returns a 400 with a friendly message; other flows still work.

## Scheduling

- Default per-entry timestamp computed from a running counter: same packing as today (2 per Tue 8pm UTC, starting after `max_ts` in existing publications).
- Each new **Add** in the browser gets the next slot in that sequence.
- User can override via `<input type="datetime-local">`. Value posted to server as ISO string; server parses to unix timestamp UTC.

## Error Handling

- Schema validation on final write — abort with clear message before touching Playwright if it fails.
- Playwright errors during batch: caught per-entry, print to stderr, stop batch, return partial success count to browser.
- Missing `ANTHROPIC_API_KEY`: AI endpoint returns 400 `{ error: "..." }`; UI shows it inline near the button.
- Login required: if Playwright can't find the "Submit" link on first `run`, assume not logged in; launch headed persistent context, print instructions to terminal, wait for context.close() to save cookies. Retry once.

## Testing

Per Ponytail: one runnable check.

- Extract `compute_next_slot(existing_ts, added_count) -> int` as a pure function. Add a `__main__` block with 3-4 `assert`s covering: empty state, one existing entry, packing 3 entries (should give 2 in slot A, 1 in slot B).

Playwright/AI/browser layers aren't unit-tested — they're integration paths verified by running the tool.

## Out of Scope (YAGNI)

- OpenAI or other providers (scaffolded, not implemented)
- Multi-platform publishing (only DeviantArt; schema supports pixiv but flow doesn't)
- Batch AI generation (per-image button only)
- Undo for deletes (they're `rm`)
- Progress streaming during publish (browser polls or blocks; simplest is block until done then render summary)
