# AGENTS.md

DeviantArt does not accept `.webp` publications.

## What this is

Publicator is a local Rust application for reviewing, scheduling and publishing artwork to DeviantArt. The code repository contains the application; publication data, `publicator.toml`, images, prompt archive and browser profiles live in a separate data directory (usually `../Art`). Run from that directory or pass `--data-dir`. No Art data or profiles belong in this repository.

Rust owns configuration, validation, publication state, scheduling, metadata providers, publishing intent/recovery, CLI and the native Iced UI. Python is retained only for the Playwright browser worker and explicit DOM prober. The static `.claude/skills/publish-deviantart/SKILL.md` is the browser step/recovery contract, not a Claude CLI dependency.

## Commands

Use the Nix environment; no pip/venv. Python test discovery is limited to `tests/python`.

```sh
nix run .#ui -- --data-dir DIR
nix develop -c cargo fmt --check
nix develop -c cargo test --all-targets --features desktop
nix develop -c env PYTHONPATH=python pytest tests/python -q
nix run .#check-steps
nix flake check
```

Apps are Nix aliases backed by Rust binaries: `ui`, `da-publish`, `generate-meta`, `vision-models`, `validate`, `echo-first`, `prompt-audit`, `check-steps`, `login`, and `pw-daemon`. `login` opens real Firefox for human sign-in; `pw-daemon` is the explicit Python Playwright DOM prober. The GUI is Iced, not an HTTP server. Do not run the real login/publisher for tests.

## Code and data

- `src/` owns Rust core (`config.rs`, `model.rs`, `store.rs`, `workflow.rs`, `application.rs`, `scheduling.rs`, `images.rs`, `prompts.rs`, `metadata/`, `publishing.rs`, `publish_journal.rs`), CLI (`bin/`) and native UI (`ui/`).
- `python/publicator_da/` owns the browser worker and DOM prober; it does not read the publication database or `publicator.toml`.
- `assets/publicationsSchema.json` is a code asset. Data paths resolve against `--data-dir`/CWD.
- `publications.json` is authoritative. Store writes validate and atomically replace under persistent thread/file locks; never remove the sibling `.lock` while writers may exist.
- `.publicator/` holds durable publishing receipts/intents and staged upload state; preserve it for recovery and reconcile unknown remote outcomes before retrying.
- `.thumbs/` is a persistent data-dir cache. Add `.thumbs/`, `.publications.json.lock`, `.publicator/`, `.deviantart-login/`, and `.deviantart-session/` to the separate data repository's ignore rules as applicable. Do not edit that data repository without authorization.
- Playwright publishing copies the named `.deviantart-login/` profile into a work profile; it must not mutate the original. Login is an out-of-band human action because DeviantArt's PerimeterX blocks Playwright login.

## Operations and safety

Read the owning file in `docs/features/` and relevant Rust callers before changing behavior. UI and CLI use shared Rust application operations and Store; no HTTP UI or Python business-logic bridge remains. File locks cover short validated mutations only, not provider/browser work. Successful sends are recorded per target; an intent with unknown outcome requires explicit reconciliation, never automatic resend.

AI generation supports OpenRouter and local Ollama. A local service URL override is available only to the CLI via `PUBLICATOR_OLLAMA_URL`; it is a dependency endpoint, not a fake/test mode. Tests use loopback fixtures. Never install a model, spend provider credentials, access the real profile, or publish real artwork as part of validation.

See [docs/README.md](docs/README.md) for feature ownership and ADRs. Task plans/reports and test artifacts stay in `/tmp`, not Git; the current Rust migration frontier in `docs/ROADMAP.md` must distinguish implementation/validation from pending final review and parent integration.
