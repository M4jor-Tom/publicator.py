# Roadmap

What exists is documented per feature in `docs/features/` (index: `docs/README.md`).
This file lists only what is not built.

- **Platform-independent scheduling.** Trigger `da-publish --all --headless` from an
  external scheduler (e.g. a GitHub Actions cron) instead of paying for DeviantArt's
  premium scheduling. Groundwork done: the two flags and their tests
  (`src/publicator/apps/da_publish.py`, `tests/test_da_publish.py`). Not started: the
  trigger itself, and the fact that the PerimeterX login profile (`.deviantart-login/`)
  must live on the runner.
