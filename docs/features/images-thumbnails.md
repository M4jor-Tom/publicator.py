# Images and thumbnails

- **Does:** Finds unpublished candidates, deduplicates by content identity, caches resized previews and prepares DeviantArt-compatible uploads.
- **Run:** `nix run .#ui -- --data-dir DIR`; `nix run .#da-publish -- --data-dir DIR`.
- **Code:** `src/images.rs`, `src/ui/`, `src/publishing.rs`, `src/store.rs`.
- **Tests:** `tests/images.rs`, `tests/ui*.rs`, `tests/publishing.rs`.
- **Config:** `publicable` image roots are configured in the data-dir `publicator.toml`.
- **Data:** `.thumbs/` is a persistent content-addressed cache; `.publicator/` holds publish staging. Both are data-dir runtime state, not source assets.
- **Decisions:** Generate previews lazily; never upload WebP. Data repo should ignore `.thumbs/` and `.publicator/`.
- **Verify:** `nix develop -c cargo test --test images --test publishing`

## How it works

Pick scans only configured roots and excludes already-published file identities. Native previews are prepared as needed and reused from `.thumbs/`. Publishing prepares a supported upload in `.publicator/` before dispatching the Python worker; the worker receives one image job and does not access the database or config.

## Invariants

- Candidate selection is bounded to configured roots; it does not accept arbitrary paths.
- Cache entries are content-addressed and persist across runs; they are not temporary `/tmp` files.
- WebP is converted to a supported format before DeviantArt upload.
- Do not remove staging files or receipts needed by unresolved publication attempts.

## Gotchas

- A missing artwork file does not justify guessing a replacement from a similar basename.
- Cache/staging ignore rules belong in the separate data repository; this project does not edit Art.
