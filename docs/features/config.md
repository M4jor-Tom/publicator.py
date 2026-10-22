# Configuration (`publicator.toml`)

- **Does:** Loads and validates data-dir configuration, publication schema and runtime options.
- **Run:** `nix run .#validate -- --data-dir DIR`
- **Code:** `src/config.rs`, `src/model.rs`, `src/store.rs`; consumers in `src/scheduling.rs`, `src/prompts.rs`, `src/bin/` and `src/ui/`.
- **Tests:** `tests/config.rs`, `tests/store.rs`, `tests/docs.rs`.
- **Config:** `publicator.toml` is optional and resolved relative to `--data-dir` or CWD. It defines publicable image roots, DeviantArt allow-lists, schedule profiles, prompt grammar and tags.
- **Data:** Config and its paths belong to the separate data directory. `assets/publicationsSchema.json` is a code asset; it does not move with data.
- **Decisions:** Rust `toml`/serde parsing and named prompt regex captures; unknown/invalid values fail at the boundary rather than falling back to a guessed convention.
- **Verify:** `nix develop -c cargo test --test config --test store`
- **Verify:** `nix run .#validate -- --data-dir DIR`

## How it works

CLI and GUI resolve data paths from the explicit data directory, defaulting to CWD. Configuration and the complete publication document are validated before mutations or provider work. The `[prompts]` filename grammar provides named `version` and optional `lineage` captures plus a configured digest algorithm; it is data, not a hardcoded filename convention.

## Invariants

- Missing optional config uses documented defaults; malformed config is a visible error.
- Paths for artwork, tags and prompt archive resolve under the data directory; schema and packaged skill/worker assets resolve from the application package.
- `publicable` choices in Pick are a runtime subset of configured roots, never arbitrary filesystem paths or TOML edits.
- DeviantArt tiers/galleries and schedule defaults are validated before persistence; configuration locality follows the selected `--data-dir`, including an external `--json` path.

## Gotchas

- A missing prompt archive disables prompt lookup; a malformed declared grammar does not silently disable it.
- `.thumbs/`, `.publicator/`, the persistent publication lock, and browser profiles are runtime data. Ignore them in the separate data repository; do not alter Art from this code repository.
