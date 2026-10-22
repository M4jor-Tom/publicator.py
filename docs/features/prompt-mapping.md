# Prompt mapping

- **Does:** Resolves an image identity against every archived prompt Git blob as Exact, Nearest or Unknown; supports prompt search and audit.
- **Run:** `nix run .#prompt-audit -- --data-dir DIR`; use prompt search in the native UI.
- **Code:** `src/prompts.rs`, `src/config.rs`, `src/ui/`, `src/bin/publicator.rs`.
- **Tests:** `tests/prompts.rs`, `tests/ui_workflow.rs`, `tests/docs.rs`.
- **Config:** Data-dir `[prompts]` declares repository, regex with required `version` and optional `lineage` captures, and digest algorithm.
- **Data:** Prompt Git repo and image files live in the data directory; archive is read-only to Publicator.
- **Decisions:** Digest all Git blobs, including historical/unreachable content; Nearest intentionally has no prompt text. No basename-at-HEAD lookup.
- **Verify:** `nix develop -c cargo test --test prompts --test ui_workflow`
- **Verify:** `nix run .#prompt-audit -- --data-dir DIR`

## How it works

The configured filename grammar maps image names to version digest and optional lineage. Exact digest hits identify immutable text. If the digest is absent, lineage can rank archived candidates as Nearest, explicitly not authoritative. Search is exact-text-only; missing optional archive disables the feature without guessing a grammar.

The data repository's prompt archive is a separate concern: the image producer attempts to commit prompt content before minting its identity. Publicator does not edit that repository.

## Invariants

- `Nearest` exposes candidate lineage, never renderable prompt text.
- Digest is computed from prompt blob content using the configured algorithm, not Git object ID.
- Filename shape and digest choice are configuration; no `sha1` or delimiter convention is hardcoded in Rust.
- Archive failures are visible diagnostics; an optional archive outage does not turn into false exact matches.

## Gotchas

- Grammar drift appears as falling parsed counts; fix the data-dir configuration, not a code fallback.
- Missing archive and malformed archive/configuration are distinct conditions.
