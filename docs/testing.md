# Behaviour-to-gate map

Which runnable check guards each retained safety transition. **Automated** rows run in
`nix run .#quality` (Rust suites, `pytest tests/python`), `nix run .#native-acceptance` and `nix flake check`
(`checks.x86_64-linux.package-smoke`). **Supervised** rows need a human and real services; no
test stands in for them. `tests/docs.rs` fails if a test named here no longer exists.

This map is not a coverage claim. The suite is **not** fully tested: test counts, all-targets
compilation and a green gate say nothing about unlisted paths (see [Unmeasured](#unmeasured)).

## Publishing and metadata transitions

| Transition | Check | Kind |
|---|---|---|
| Admission: invalid or unsupported work never starts a worker | `tests/publishing.rs::unsupported_artwork_fails_preflight_before_worker_start`, `tests/publishing.rs::marked_or_unphased_pending_targets_never_start_the_worker`, `tests/publishing.rs::step_check_failure_prevents_worker_session_start`, `tests/publishing.rs::another_process_publisher_lock_prevents_worker_start` | automated |
| Admission: UI selection and options are validated and flushed first | `tests/ui_publish_calendar.rs::publish_dispatch_rejects_a_stale_saved_cell_set_before_worker_start`, `tests/ui_publish_calendar.rs::publish_flushes_valid_option_edits_before_admission`, `tests/application_ai.rs::generation_lease_excludes_duplicate_and_same_uuid_edits_but_not_other_work` | automated |
| Admission: worker rejects bad jobs before touching the session | `tests/python/test_worker.py::test_worker_rejects_bad_submit_before_session_mutation`, `tests/python/test_worker.py::test_worker_refuses_same_name_foreign_root_and_forged_open_root` | automated |
| Dispatch: selection, flags and data root reach the worker | `tests/cli.rs::default_publish_is_first_pending_and_all_uuid_remain_explicit_batches`, `tests/cli.rs::headless_flag_reaches_the_publish_worker_open_command`, `tests/cli.rs::browser_headless_config_reaches_the_publish_worker_open_command`, `tests/publishing.rs::ui_publish_takes_headless_from_config`, `src/publishing.rs::worker_launch_binds_store_data_root_outside_ndjson`, `tests/python/test_worker.py::test_worker_subprocess_preserves_json_line_framing` | automated |
| Dispatch: one delivery at a time per target | `tests/ui_publish_calendar.rs::duplicate_publish_request_and_completion_change_nothing_through_the_reducer`, `tests/ui_describe.rs::describe_dispatch_holds_only_target_until_provider_result_is_persisted` | automated |
| Dispatch: default development and packaged worker launch | `tests/cli.rs::checkout_check_steps_runs_the_default_worker_without_overrides`, `tests/packaging.rs::cli_uses_embedded_schema_and_injected_assets_outside_the_checkout`, flake check `package-smoke` | automated |
| Before commitment: the intent is durable before any send | `src/publish_journal.rs::intent_directory_creation_syncs_each_parent_before_the_attempt_directory`, `src/publish_journal.rs::failed_parent_sync_prevents_confirming_new_intent_directories`, `src/publish_journal.rs::approved_edit_after_hash_check_is_rejected_inside_the_fresh_store_mutation` | automated |
| After commitment: each result is saved before the next send | `tests/publishing.rs::publisher_persists_success_before_next_and_retry_skips_completed_delivery`, `tests/python/test_worker.py::test_worker_stops_after_terminal_submission_error` | automated |
| Persistence uncertainty: unknown or unapplied outcomes keep their receipt and are never resent | `tests/publishing.rs::unknown_worker_outcome_is_not_resent_on_retry`, `tests/publishing.rs::unusable_worker_terminals_are_unknown_and_never_resent`, `tests/publishing.rs::publisher_store_failure_after_terminal_keeps_unapplied_receipt`, `src/publishing.rs::terminal_read_timeout_keeps_attempt_unknown_and_blocks_retry`, `src/publish_journal.rs::store_pre_rename_failure_keeps_scheduled_receipt_for_recovery`, `src/publish_journal.rs::store_post_rename_durability_failure_keeps_receipt_and_recovers_idempotently`, `tests/store.rs::failed_staging_preserves_original_after_mutation_callback` | automated |
| Refresh: UI state is rebuilt from what was persisted | `tests/ui_workflow.rs::pick_dispatch_persists_candidate_before_refreshing_phase`, `tests/ui_drafts.rs::failed_save_keeps_dirty_for_explicit_retry_without_tick_spam` | automated |
| Close: waits for the durable receipt, saves drafts, reaps workers | `tests/ui_publish_calendar.rs::close_during_delayed_publish_waits_for_durable_receipt_and_cleanup`, `tests/ui_publish_calendar.rs::close_cancel_stops_after_current_receipt_without_dropping_publisher_task`, `tests/ui_publish_calendar.rs::close_saves_option_drafts_and_invalid_option_blocks_close`, `tests/publishing.rs::cancellation_finishes_current_artwork_and_leaves_later_target_unattempted`, `tests/publishing.rs::shutdown_terminates_worker_descendants`, `src/publishing.rs::cleanup_force_kills_worker_that_ignores_termination` | automated |
| Restart: saved successes are replayed once and skipped | `tests/ui_publish_calendar.rs::publish_failure_retry_after_restart_shows_real_cell_outcomes_and_skips_saved_success`, `tests/publish_journal.rs::terminal_receipt_survives_a_store_read_failure_for_later_replay`, `tests/publish_journal.rs::recorded_store_success_is_replayed_once_even_with_historical_pending_platform`, `tests/publishing.rs::second_publish_after_success_is_a_zero_work_noop` | automated |
| Reconciliation: only an explicit human outcome releases an unknown attempt | `tests/publish_journal.rs::human_scheduled_reconciliation_resolves_unknown_and_is_idempotent`, `tests/publish_journal.rs::pending_attempt_blocks_automatic_retry_and_explicit_not_submitted_releases_it`, `tests/publish_journal.rs::reconciliation_refuses_a_held_publisher_lock_without_rewriting_receipt`, `tests/ui_publish_calendar.rs::unresolved_unknown_blocks_resend_until_confirmed_not_submitted`, `tests/ui_publish_calendar.rs::scheduled_reconciliation_refuses_changed_metadata_and_keeps_newer_fields_and_receipt`, `tests/cli.rs::reconcile_and_submission_options_cannot_be_mixed` | automated |
| Deletion: a publication entry is removed only while undelivered and not named by an unresolved attempt | `tests/workflow_dates.rs::deleting_an_undelivered_publication_returns_its_artwork_to_pick`, `tests/workflow_dates.rs::delivered_publication_history_cannot_be_deleted`, `tests/publish_journal.rs::unresolved_attempt_blocks_deleting_its_publication_until_reconciled`, `tests/cli.rs::delete_publication_removes_only_the_named_undelivered_entry`, `tests/ui_workflow.rs::confirmed_tile_deletion_removes_the_publication_and_its_draft` | automated |
| Manual fallback hands over only the recorded artwork bytes | `tests/cli.rs::echo_first_prints_the_recorded_file_among_same_named_files`, `tests/cli.rs::echo_first_refuses_artwork_whose_bytes_changed` | automated |
| Real DeviantArt login (`nix run .#login`, human Firefox; PerimeterX blocks automation) | none | supervised |
| Real publish against live DeviantArt (`nix run .#da-publish`): live DOM, remote draft/schedule effect | none | supervised |
| Manual recovery and reconciliation evidence per `.claude/skills/publish-deviantart/SKILL.md` | none | supervised |
| Native window replay (`nix run .#native-acceptance [STORY ...]` runs `scripts/native-ui-replay.py` on a private Xvfb against `examples/ui_acceptance.rs` fakes): focus, keyboard, scroll, visible images, long names kept inside cards, persisted edits, close routes, tile deletion, Describe layout | replay stories; `scripts/native-ui-smoke.sh` stays the supervised human tool | automated: the post-commit gate and CI's `native-acceptance` job |

## Development sandbox

Run `nix develop -c ui-sandbox [SCENARIO]` to launch the native UI with fake AI and publishing services. The default is `mixed`; other fixtures are `gallery`, `calendar`, `empty`, `history`, `publish`, `prompt-search`, `prompt-search-missing-archive` and `reopen`. Each launch prints a fresh temporary data directory and preserves it for inspection. This dev-shell command is not a production package. By contrast, `scripts/dummy-data.sh` creates disposable data but does **not** replace real services.

For the publishing step check without Nix wrappers, run `nix develop -c cargo run --bin publicator -- check-steps` from the checkout. The default launcher is `python/publicator-da-worker` and the manual/recovery contract is `.claude/skills/publish-deviantart/SKILL.md`; `PUBLICATOR_WORKER` and `PUBLICATOR_SKILL` override those defaults. Use the step check and offline fixtures for validation, never a real login or submission.

## Maintenance limits

- Prompt archive rebuilds buffer Git blobs and retain the previous index until the swap. The accepted size is about 100 MB of object content (about 258 MiB peak RSS in the debug audit at 100 MiB of blobs). Beyond that, stream the objects rather than raising the ceiling.
- If migrating from the old thumbnail fallback, remove only the data directory's `.thumbs/` to rebuild oversized cached originals. Never remove `.publicator/` staging or receipts to clear a failure, or unlink the persistent store lock.

## Unmeasured

- Branch and line coverage are not measured, and no coverage tool is part of the gate or dev
  shell. An ad hoc run (for example `cargo llvm-cov` or `pytest --cov` from a temporary shell)
  is diagnostic only; never gate on its numbers.
- Live DeviantArt behaviour: FakeSession and routed local HTML do not prove remote effects.
- Abrupt process death (SIGKILL, power loss) mid-write beyond the injected pre/post-rename and
  fsync failures; real filesystem durability.
- Real provider services (OpenRouter, Ollama model installs): tests use loopback fixtures only.
- The packaged desktop UI beyond `publicator-ui --help` and the URL-opener smoke; the `gpu`
  feature.
