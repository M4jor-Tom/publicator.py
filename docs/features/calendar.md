# Calendar

- **Does:** Read-only native month view of every apparition using configured-timezone dates, shared publication titles and actual URLs.
- **Run:** `nix run .#ui -- --data-dir DIR`, then select Calendar.
- **Code:** `src/calendar.rs`, `src/ui/calendar.rs`, `src/ui/`.
- **Tests:** `tests/calendar.rs`, `tests/ui_publish_calendar.rs`.
- **Config:** `[schedule] timezone`; optional prompt archive settings.
- **Data:** Current Store records, apparition URLs and shared `.thumbs/` previews. Missing artwork remains a text event.
- **Decisions:** No browser clock or URL-slug title guessing; unregistered historical apparitions remain visible.
- **Verify:** `nix develop -c cargo test --features desktop --test calendar --test ui_publish_calendar`

## How it works

Calendar projects each apparition as a distinct event, falling back to publication submission time when no date override exists. It refreshes from persisted data and navigates to the target workflow phase. Stored URLs can be opened only when valid; URLs are never synthesized from titles.

## Invariants

- Date bucketing uses the configured schedule timezone.
- Every apparition is retained with its platform, delivery state and actual URL; title/date remain publication-shared.
- Preview/cache access is limited to known artwork; missing files do not fabricate previews.
- Prompt content is shared with the Pick view; a nearest match is never represented as exact prompt text.

## Gotchas

- Duplicate artwork basenames in one data directory may be ambiguous; retain unique file identities.
- Calendar is read-only and does not reserve, approve or publish work.
