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
nix run .#quality
nix flake check
```

Apps are Nix aliases backed by Rust binaries: `ui`, `da-publish`, `generate-meta`, `vision-models`, `validate`, `echo-first`, `prompt-audit`, `check-steps`, `login`, and `pw-daemon`. Dev-only apps: `quality` (the full gate) and `native-acceptance` (Xvfb replay of the UI stories against `examples/ui_acceptance.rs` fakes). CI runs `quality`, `native-acceptance` and `nix flake check -L`. Locally, `.githooks/pre-commit` runs `nix flake check -L`, `post-commit` gates the commit with `quality` and `native-acceptance` in the background (`.git/quality-gate/`), and `pre-push` only accepts commits whose gate passed; report their results, and rerun a failure in isolation before blaming the change. `login` opens real Firefox for human sign-in; `pw-daemon` is the explicit Python Playwright DOM prober. The GUI is Iced, not an HTTP server. Do not run the real login/publisher for tests.

## Code and data

- `src/` owns Rust core (`config.rs`, `model.rs`, `store.rs`, `workflow.rs`, `application.rs`, `scheduling.rs`, `images.rs`, `artwork.rs`, `prompts.rs`, `metadata/`, `publishing.rs`, `publish_journal.rs`), CLI (`bin/`) and native UI (`ui/`).
- `python/publicator_da/` owns the browser worker and DOM prober; it does not read the publication database or `publicator.toml`. `python/publicator-da-worker` is the checkout's default worker launcher (`PUBLICATOR_WORKER` overrides; Nix packages inject their own).
- Publishing and AI generation read artwork only through `artwork::resolve`: the visible file whose bytes match the recorded SHA-512, used via a hash-verified private copy, never the mutable original.
- `tests/common/{executable,git,http,image,process}.rs` are also mounted into the crate with `#[path]`; keep them free of `publicator::` paths.
- `assets/publicationsSchema.json` is a code asset. Data paths resolve against `--data-dir`/CWD.
- `publications.json` is authoritative. Store writes validate and atomically replace under persistent thread/file locks (publishing receipts use the same `store::write_atomic`); `DurabilityUncertain` means the new bytes may already be committed. Never remove the sibling `.lock` while writers may exist.
- `.publicator/` holds durable publishing receipts/intents and staged upload state; preserve it for recovery and reconcile unknown remote outcomes before retrying. `.publicator/ui.json` holds the UI layout preference (Describe Grid/Rows).
- `.thumbs/` is a persistent data-dir cache. Add `publicator.local.toml` (secrets, e.g. `[openrouter].key`), `.thumbs/`, `.publications.json.lock`, `.publicator/`, `.deviantart-login/`, and `.deviantart-session/` to the separate data repository's ignore rules as applicable. Do not edit that data repository without authorization.
- Playwright publishing copies the named `.deviantart-login/` profile into a work profile; it must not mutate the original. Login is an out-of-band human action because DeviantArt's PerimeterX blocks Playwright login.

## Operations and safety

Read the owning Gherkin spec and `docs/features/index.toml` and relevant Rust callers before changing behavior. UI and CLI use shared Rust application operations and Store; no HTTP UI or Python business-logic bridge remains. File locks cover short validated mutations only, not provider/browser work. Successful sends are recorded per target; an intent with unknown outcome requires explicit reconciliation, never automatic resend.

AI generation supports OpenRouter and local Ollama. A local service URL override is available only to the CLI via `PUBLICATOR_OLLAMA_URL`; it is a dependency endpoint, not a fake/test mode. Tests use loopback fixtures. Never install a model, spend provider credentials, access the real profile, or publish real artwork as part of validation.

See [docs/README.md](docs/README.md) for feature ownership and ADRs. Task plans/reports and test artifacts stay in `/tmp`, not Git; `docs/ROADMAP.md` lists open work only, so remove an item when it lands.

## Software factory

`/factory run` builds every open GitHub issue labeled `factory` through
intake → spec → build → gate → review → approve, one PR per issue. State is in
`factory/jobs/` (gitignored); `factory/bin/job` is its only writer. Checks:
`factory/checks.sh` (`nix run .#quality`, `nix run .#native-acceptance`,
`nix flake check -L`, as in CI). Dashboard: `cd factory && python3 -m http.server`,
then open http://localhost:8000/dashboard.html.

Each job works in its own git worktree, a full checkout of the repo at
`.worktrees/<job-id>` (gitignored). A tool run from the repo root that doesn't
read `.gitignore` sees those copies too; exclude `.worktrees/` in its config.
A fresh worktree needs no setup: Nix provides every dependency, and the first
build in it is cold. The `.githooks` skip `factory/*` branches, because the gate
runs the same checks; push them without waiting for a quality-gate verdict.
Job commits and PR titles are Conventional Commits, `<type>[(scope)][!]: <description>`
(`feat`, `fix`, `refactor`, `test`, `docs`, …); the squash merge keeps the PR title.

**Seeing the UI.** There is no dev server or browser, and job ports are unused:
the UI is a native Iced window. Run the `ui_acceptance` fixtures (fake services
only; fixture scenarios in `docs/testing.md`, gallery behavior in `docs/features/gallery-ui.feature`) on a private Xvfb display,
screenshot it to `/tmp/factory-<job-id>-<agent>-*.png`, and read the PNGs. Never
launch the real `ui`, `login` or publisher.

```sh
nix develop -c cargo build --locked --release --example ui_acceptance --features desktop
timeout 180 nix develop -c xvfb-run -a -s '-screen 0 1440x900x24' bash -c '
  unset WAYLAND_DISPLAY  # else the window opens on the real desktop
  openbox --sm-disable >/dev/null 2>&1 &
  target/release/examples/ui_acceptance --scenario SCENARIO \
    --fixture-dir "$(mktemp -d /tmp/publicator-ui.XXXXXX)" >/dev/null 2>&1 & app=$!
  win=; for _ in $(seq 60); do win=$(xdotool search --onlyvisible --pid $app | head -1); [ -n "$win" ] && break; sleep 1; done
  [ -n "$win" ] || { echo "no window"; kill $app; exit 1; }
  sleep 4; import -window "$win" OUT.png
  kill $app' </dev/null
```

For a narrow window or a click, add `xdotool windowsize "$win" W H` or
`xdotool mousemove --window "$win" X Y click 1`, then another `import`. Never call
`import` without a window id: it then waits for a mouse click forever.
