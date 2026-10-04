# Lessons

## 2026-09-18 — no feature branch for bounded work
Created `feature/da-publish-flags` for a two-flag CLI change; user said "Implement on master directly".
Rule: this repo has no `develop`; bounded changes (a flag, a small fix) go straight on `master`. Branch only when the user starts one.

## 2026-10-08 — batch generation is publication-driven
When asked to generate metadata for marked publications, I initially kept `generate-meta`'s `IMAGE...` interface in mind. The user clarified that it should scan `publications.json` and generate every marked entry, with no image arguments.
Rule: for this workflow, treat `publications.json` markers as the input contract; do not require image paths on the CLI.
Also required: vision model options are objects with string `ollama_name` and `openrouter_name` fields, not bare IDs.
Rule: preserve the dual-provider model mapping explicitly; never infer one provider's model identifier from the other's.
Install UX: the GUI may auto-install on local generation but must indicate the install; the CLI must ask `[Y/n]` unless `-y` is passed, with Enter defaulting to yes.
Rule: preserve this GUI/CLI consent difference and the CLI's yes-by-default prompt for Ollama downloads.
Workflow: user requested design and plan artifacts in `/tmp`, self-reviewed and approved before execution, with implementation delegated subagent-driven.
Rule: follow explicit artifact-location and self-approval instructions; don't add another user gate after the user directs execution.
