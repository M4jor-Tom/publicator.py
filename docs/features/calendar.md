# Calendar tab

- **Does:** Read-only month grids of every apparition, past and scheduled, with server-side schedule-timezone bucketing and native prompt popovers.
- **Run:** `nix run .#ui -- --data-dir DIR`, then Calendar; history-only startup is supported.
- **Code:** `src/publicator/webui/calendar_view.py` (`timeline`, `render_calendar`, `event_html`, `month_html`), `src/publicator/webui/server.py` (`thumb_maps`, `_build_page`, `_prompt_html`), `src/publicator/webui/page.py` (tab reveal and anchor navigation), `src/publicator/images.py` (cache/index).
- **Tests:** `tests/test_calendar.py` (shared title/date/URL, multi-platform identities, timezone and popovers), `tests/test_page.py`, `tests/test_server.py` (refresh and allow-lists).
- **Config:** `[schedule] timezone`, default Europe/Paris; optional `[prompts]` archive.
- **Data:** Current `publications.json` on page refresh, publication title/date, apparition url; shared lazy `.thumbs/` cache. Missing artwork remains a text event.
- **Decisions:** Pure renderer; no browser clock or URL-slug title guessing; historical unregistered platforms remain visible.
- **Verify:** `nix develop -c pytest tests/test_calendar.py tests/test_page.py tests/test_server.py -q`

## How it works

Each apparition has its own event identity even when a publication appears on
multiple platforms. Dates fall back to publication submissionTimestamp when no
override exists. Successful submission may still represent a future appearance.
A refreshed page reconstructs the timeline, not a startup snapshot.

## Invariants

- All day/month math is server-side in the schedule timezone; the month range includes today even with no events.
- Every stored apparition is retained, including unregistered historical platforms, with its platformName, own delivery state/URL and shared publication title/date. Titles never come from a URL slug. External prompt-panel links name that platform; otherwise a UUID anchor reveals the correct workflow phase before scrolling to its row/card.
- Prompt popovers receive the same prompt renderer output used by candidate cards; a Nearest has no renderable real prompt text. Unknown prompts retain plain one-click links.
- Each panel gets a unique date/index popover ID, independent of publication UUID, so multiple apparitions cannot collide.
- Thumbnails serve only allow-listed cache names; original-image paths are likewise allow-listed. Hidden browser/cache directories are excluded from basename indexing.

## Gotchas

- Basename indexing uses the first matching visible file; duplicate basenames across directories are ambiguous. Keep stored file identities unique in a data directory.
- Epoch-zero legacy dates can expand the rendered month range back to 1970; migration/validation should retain real submission timestamps, not invent dates.
- Calendar prompts are resolved per page request and can be expensive over a large archive; no live-update channel is maintained.
