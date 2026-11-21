#![cfg(feature = "desktop")]

mod common;

use chrono::NaiveDate;
use chrono_tz::Pacific::Honolulu;
use iced_runtime::{Action, futures::futures::StreamExt};
use publicator::{
    calendar::{months, timeline},
    config::{Config, Paths, PromptConfig, PromptGrammar},
    prompts::{Lineage, PromptMatch, PromptVersion},
    publish_journal::ReconcileOutcome,
    publishing::{PublishCellState, Publisher},
    scheduling,
    store::Store,
    ui::{
        App, Message, Tab, UiOptions,
        drafts::Field,
        prompt,
        publish::{SelectionState, selection_state},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// A UI over `f`'s store with unreachable AI providers, and the task starting its first load.
fn offline_ui(f: &common::Fixture) -> (App, iced::Task<Message>) {
    App::from_application(
        Arc::new(common::offline_app(Arc::new(f.store.clone()))),
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    )
}

fn loaded_default_app(f: &common::Fixture, row: serde_json::Value) -> App {
    f.write(serde_json::json!([row.clone()]));
    let (mut app, _) = offline_ui(f);
    let _ = app.update(Message::LoadFinished(0, Ok(vec![row].into())));
    app
}

#[test]
fn selection_is_partial_when_only_one_pending_cell_is_checked() {
    assert_eq!(selection_state(&[true, false]), SelectionState::Partial);
    assert_eq!(selection_state(&[]), SelectionState::None);
}

#[tokio::test]
async fn publish_checkbox_persists_registered_target_through_ui_task() {
    let f = common::Fixture::new();
    let store = Arc::new(f.store.clone());
    let row = serde_json::json!({
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED",
        "title":"Title", "description":"Description", "submissionTimestamp":1700000000,
        "files":[{"basename":"art.png","sha512sum":"0".repeat(128)}], "apparitions":[]
    });
    let mut app = loaded_default_app(&f, row);
    let task = app.update(Message::SelectCell {
        key: "00000000-0000-4000-8000-000000000001:deviantart".into(),
        selected: true,
    });
    common::ui::finish_workflow(&mut app, task).await;
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(
        saved[0]["apparitions"],
        serde_json::json!([
            {"platformName":"deviantart","state":"unpublished"}
        ])
    );
}

#[tokio::test]
async fn publish_dispatch_rejects_a_stale_saved_cell_set_before_worker_start() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let store = Arc::new(Store::new(paths, Config::default()).unwrap());
    let mut row = serde_json::json!({
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED",
        "title":"Title", "description":"Description", "submissionTimestamp":1700000000,
        "files":[{"basename":"missing.png","sha512sum":"0".repeat(128)}],
        "apparitions":[{"platformName":"deviantart","state":"unpublished"}]
    });
    std::fs::write(
        &store.paths().json,
        serde_json::to_vec(&vec![row.clone()]).unwrap(),
    )
    .unwrap();
    let application = Arc::new(
        common::offline_app(store.clone()).with_publisher(Publisher::new(
            root.path().join("no-worker"),
            root.path().join("no-skill"),
        )),
    );
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    let _ = app.update(Message::LoadFinished(0, Ok(vec![row.clone()].into())));
    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::new(),
    )));
    row["apparitions"] = serde_json::json!([]);
    store
        .mutate(|rows| {
            rows[0] = row;
            Ok(())
        })
        .unwrap();

    let completion = common::ui::first_output(app.update(Message::PublishSelected)).await;
    match &completion {
        Message::PublishFinished(_, completion) => {
            assert!(
                completion
                    .result
                    .as_ref()
                    .unwrap_err()
                    .contains("selected publishing cells changed")
            )
        }
        other => panic!("expected stale selection rejection, got {other:?}"),
    }
    let refresh = app.update(completion);
    assert!(iced_runtime::task::into_stream(refresh).is_some());
    assert!(!root.path().join("no-worker").exists());
}

#[test]
fn calendar_timeline_keeps_each_apparition_and_its_actual_url() {
    let rows = vec![serde_json::json!({
        "uuid":"00000000-0000-4000-8000-000000000001",
        "title":"<literal>","description":"", "submissionTimestamp":1700000000,
        "files":[{"basename":"absent.png"}],
        "apparitions":[
            {"platformName":"deviantart","state":"unpublished"},
            {"platformName":"historic","state":"published_or_scheduled","url":"https://example.test/item"}
        ]
    })];
    let events = timeline(&rows).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].url.as_deref(), Some("https://example.test/item"));
    assert!(events.iter().all(|event| event.title == "<literal>"));
}

fn publishing_fixture() -> common::Fixture {
    let f = common::Fixture::with_config(Config {
        tags: Some("tags.txt".into()),
        ..Config::default()
    });
    std::fs::write(f.path("tags.txt"), "fake tag\n").unwrap();
    f
}

fn pending_publication(f: &common::Fixture, n: u64) -> serde_json::Value {
    let image = f.image(&format!("{n}.png"), 4, 3);
    let mut row = common::row(n, "DESCRIBED");
    row["files"][0]["sha512sum"] =
        serde_json::json!(publicator::images::file_sha512(&image).unwrap());
    row["apparitionTimestampIfDifferentThanSubmission"] = serde_json::json!(1_700_002_800);
    row["apparitions"] = serde_json::json!([
        {"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}
    ]);
    row
}

fn publishing_ui_app(f: &common::Fixture, worker: &Path) -> (App, Arc<Store>, iced::Task<Message>) {
    let store = Arc::new(f.store.clone());
    let application = Arc::new(
        common::offline_app(store.clone())
            .with_publisher(Publisher::new(worker.to_owned(), f.path("skill.md"))),
    );
    let (app, init) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    (app, store, init)
}

fn fake_publisher_worker(f: &common::Fixture, body: &str) -> PathBuf {
    let worker = f.path("worker.sh");
    common::executable::write_executable(
        &worker,
        &format!(
            r#"#!/bin/sh
[ "$1" = "--check-steps" ] && exit 0
{body}
"#
        ),
    );
    worker
}

fn options_app() -> (App, Arc<Store>, tempfile::TempDir) {
    let config = Config {
        tiers: vec!["Gold".into(), "Silver".into()],
        galleries: vec!["Art".into(), "Featured".into()],
        ..Config::default()
    };
    let f = common::Fixture::with_config(config);
    let store = Arc::new(f.store.clone());
    // Real artwork bytes: a publish from this app can fail only for the reason a test names.
    let mut row = f.art_row(9, "DESCRIBED");
    row["apparitions"] = serde_json::json!([{"platformName":"deviantart","state":"unpublished",
        "priceIfNotFree":2.5,"tiers":["Gold"],"galleries":["Art"]}]);
    let mut app = loaded_default_app(&f, row);
    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::new(),
    )));
    (app, store, f.root)
}

fn scheduled_jump_app() -> (App, Arc<Store>, tempfile::TempDir) {
    let (mut app, store, root) = options_app();
    store
        .mutate(|rows| {
            rows[0]["state"] = serde_json::json!("SCHEDULED");
            Ok(())
        })
        .unwrap();
    let _ = app.update(Message::LoadFinished(0, Ok(store.read().unwrap().into())));
    (app, store, root)
}

/// Holds the first submit in a recorded same-group child until `release` exists or the test
/// process is gone: a killed test runner runs no destructor, so the hold bounds itself.
fn held_publisher_worker(
    f: &common::Fixture,
    calls: &Path,
    started: &Path,
    release: &Path,
) -> PathBuf {
    let body = format!(
        r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{{"protocol":1,"type":"ready"}}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  echo "$uuid" >> '{}'
  if [ ! -e '{started}' ]; then
    (while [ ! -e '{release}' ] && kill -0 $PPID 2>/dev/null; do sleep 0.01; done) &
    echo "$$ $!" > '{started}.tmp'; mv '{started}.tmp' '{started}'; wait $!
  fi
  printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid" ;;
close) exit 0 ;;
esac
done"#,
        calls.display(),
        started = started.display(),
        release = release.display()
    );
    fake_publisher_worker(f, &body)
}

async fn wait_for_worker_gate<S>(actions: &mut S, started: &Path)
where
    S: iced::futures::Stream<Item = iced_runtime::Action<Message>> + Unpin,
{
    loop {
        tokio::select! {
            _ = async {
                while !started.exists() {
                    tokio::task::yield_now().await;
                }
            } => break,
            action = actions.next() => match action {
                Some(Action::Output(message)) => panic!("publish ended before the gate: {message:?}"),
                Some(_) => {},
                None => panic!("publish task ended before the fake worker gate"),
            }
        }
    }
}

/// A publish whose first delivery is held inside the fake worker until `release_publish`.
/// `started` records "<worker pid = group> <hold pid>"; dropping kills that group's survivors.
struct HeldPublish {
    fixture: common::Fixture,
    app: App,
    store: Arc<Store>,
    actions: iced_runtime::futures::BoxStream<iced_runtime::Action<Message>>,
    calls: PathBuf,
    started: PathBuf,
    release: PathBuf,
}

impl Drop for HeldPublish {
    fn drop(&mut self) {
        drop(common::GroupWatchdog(&self.started));
    }
}

/// Publishes the pending publications `numbers`, beside the `extra` rows, and holds delivery.
async fn held_publish(numbers: &[u64], extra: &[serde_json::Value]) -> HeldPublish {
    let f = publishing_fixture();
    let mut rows: Vec<_> = numbers
        .iter()
        .map(|&n| pending_publication(&f, n))
        .collect();
    rows.extend_from_slice(extra);
    f.write(serde_json::json!(rows));
    let (calls, started, release) = (
        f.path("worker-calls"),
        f.path("worker-started"),
        f.path("worker-release"),
    );
    let worker = held_publisher_worker(&f, &calls, &started, &release);
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    common::ui::settle(&mut app, init).await;
    let task = app.update(Message::PublishSelected);
    let mut actions = iced_runtime::task::into_stream(task).expect("publish task started");
    tokio::time::timeout(
        Duration::from_secs(5),
        wait_for_worker_gate(&mut actions, &started),
    )
    .await
    .expect("first delivery reached the controlled worker gate");
    HeldPublish {
        fixture: f,
        app,
        store,
        actions,
        calls,
        started,
        release,
    }
}

/// Releases the held delivery; the publisher answers only after its durable receipt and cleanup.
async fn release_publish(held: &mut HeldPublish) -> Message {
    std::fs::write(&held.release, "continue").unwrap();
    let completion = tokio::time::timeout(
        Duration::from_secs(5),
        common::ui::next_output(&mut held.actions),
    )
    .await
    .expect("publisher returns its durable result after worker cleanup");
    assert!(matches!(completion, Message::PublishFinished(..)));
    let (group, _) = common::gate_pids(&held.started).expect("worker gate");
    assert!(
        common::eventually(Duration::from_secs(5), || {
            common::live_process_group_members(group).is_empty()
        }),
        "worker group survived its publish: {:?}",
        common::live_process_group_members(group)
    );
    completion
}

fn sent_uuids(held: &HeldPublish) -> Vec<String> {
    std::fs::read_to_string(&held.calls)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn deviantart_state(held: &HeldPublish, index: usize) -> serde_json::Value {
    held.store.read().unwrap()[index]["apparitions"][0]["state"].clone()
}

fn held_outcome(held: &HeldPublish, n: u64) -> PublishCellState {
    held.app.publish_cell_outcome(&common::id(n)).unwrap().state
}

fn receipt_count(held: &HeldPublish) -> usize {
    std::fs::read_dir(held.fixture.path(".publicator/publish"))
        .unwrap()
        .count()
}

#[tokio::test]
async fn close_cancel_stops_after_current_receipt_without_dropping_publisher_task() {
    let mut held = held_publish(&[32, 33], &[]).await;
    let close = common::ui::run(held.app.update(Message::CloseRequested)).await;
    assert_eq!(close.exits, 0, "close must wait for the active delivery");
    let _ = held.app.update(Message::CloseCancelPublish);
    let completion = release_publish(&mut held).await;
    let followup = common::ui::send(&mut held.app, completion).await;
    assert!(followup.any(|message| matches!(message, Message::PublishStatusesFinished(Ok(_)))));
    assert_eq!(followup.exits, 1, "close completes after cleanup");
    assert_eq!(sent_uuids(&held), [common::id(32)]);
    assert_eq!(deviantart_state(&held, 0), "published_or_scheduled");
    assert_eq!(deviantart_state(&held, 1), "unpublished");
}

#[tokio::test]
async fn duplicate_publish_request_and_completion_change_nothing_through_the_reducer() {
    let mut held = held_publish(&[32, 33], &[]).await;
    let refused = common::ui::run(held.app.update(Message::PublishSelected)).await;
    assert_eq!(
        (refused.messages.len(), refused.exits),
        (0, 0),
        "a second Publish during the held delivery starts nothing: {refused:?}"
    );
    let completion = release_publish(&mut held).await;
    common::ui::send(&mut held.app, completion.clone()).await;
    assert_eq!(sent_uuids(&held), [common::id(32), common::id(33)]);
    let (disk, receipts, before) = (
        std::fs::read(&held.store.paths().json).unwrap(),
        receipt_count(&held),
        [32, 33].map(|n| held_outcome(&held, n)),
    );
    let again = common::ui::run(held.app.update(completion.clone())).await;
    assert_eq!((again.messages.len(), again.exits), (0, 0), "{again:?}");
    assert_eq!(std::fs::read(&held.store.paths().json).unwrap(), disk);
    let after = [32, 33].map(|n| held_outcome(&held, n));
    assert_eq!((receipt_count(&held), after), (receipts, before));

    // A newer run holds its own lease: the old completion can neither end it nor allow a close.
    let newer = pending_publication(&held.fixture, 34);
    held.store
        .mutate(|rows| {
            rows.push(newer);
            Ok(())
        })
        .unwrap();
    common::ui::send(&mut held.app, Message::Refresh).await;
    let newer_run = held.app.update(Message::PublishSelected);
    for message in [completion, Message::CloseRequested] {
        let effects = common::ui::send(&mut held.app, message).await;
        assert_eq!(
            (effects.messages.len(), effects.exits),
            (0, 0),
            "{effects:?}"
        );
    }
    let newer_completion = common::ui::first_output(newer_run).await;
    let closed = common::ui::send(&mut held.app, newer_completion).await;
    assert_eq!(
        closed.exits, 1,
        "close follows the newer run's own completion"
    );
    assert_eq!(
        sent_uuids(&held),
        [common::id(32), common::id(33), common::id(34)]
    );
}

#[tokio::test]
async fn close_saves_option_drafts_and_invalid_option_blocks_close() {
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let before = std::fs::read(&store.paths().json).unwrap();
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "5".into(),
    });
    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::from([uuid.clone()]),
    )));
    let refused = common::ui::send(&mut app, Message::CloseRequested).await;
    assert!(refused.any(|message| matches!(
        message,
        Message::OptionsSaved { result: Err(error), .. } if error.contains("Unresolved publish attempt")
    )));
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);

    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::new(),
    )));
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "NaN".into(),
    });
    let failed = common::ui::send(&mut app, Message::CloseRequested).await;
    assert!(failed.any(|message| matches!(
        message,
        Message::OptionsSaved { result: Err(error), .. } if error.contains("finite nonnegative")
    )));
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);

    let _ = app.update(Message::OptionPriceChanged {
        uuid,
        value: "0".into(),
    });
    let saved = common::ui::send(&mut app, Message::CloseRequested).await;
    assert!(saved.any(|message| matches!(message, Message::OptionsSaved { result: Ok(()), .. })));
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(persisted[0]["apparitions"][0]["priceIfNotFree"], 0.0);
}

#[tokio::test]
async fn close_during_delayed_publish_waits_for_durable_receipt_and_cleanup() {
    let mut held = held_publish(&[31], &[]).await;
    let close = common::ui::run(held.app.update(Message::CloseRequested)).await;
    assert_eq!((close.messages.len(), close.exits), (0, 0));
    let completion = release_publish(&mut held).await;
    let followup = common::ui::send(&mut held.app, completion).await;
    assert!(
        followup.any(|message| matches!(message, Message::PublishStatusesFinished(Ok(_)))),
        "publisher status refresh failed after cleanup: {followup:?}"
    );
    assert_eq!(
        followup.exits, 1,
        "the requested close happens after cleanup"
    );
    assert_eq!(deviantart_state(&held, 0), "published_or_scheduled");
}

/// Discard-and-close, or close then Refresh, during a held delivery: no exit and no
/// reload (the publisher lock is held) until the durable outcome; then exactly one exit.
async fn close_route_waits_for_publisher(close: Message, dirty: bool) {
    let scheduled = common::id(5);
    let mut held = held_publish(&[31], &[common::row(5, "SCHEDULED")]).await;
    if dirty {
        let _ = held.app.update(Message::Edited {
            uuid: scheduled.clone(),
            field: Field::Title,
            text: "Edited during publish".into(),
        });
    }
    let flushes = dirty && matches!(close, Message::CloseRequested);
    for message in [close, Message::Refresh] {
        let effects = common::ui::send(&mut held.app, message).await;
        assert_eq!(effects.exits, 0, "exited during publish: {effects:?}");
        assert!(
            !effects.any(|message| matches!(
                message,
                Message::LoadFinished(..) | Message::PublishStatusesFinished(..)
            )),
            "reloaded while the publisher lock is held: {effects:?}"
        );
    }
    let completion = release_publish(&mut held).await;
    let followup = common::ui::send(&mut held.app, completion).await;
    assert_eq!(
        followup.exits, 1,
        "close after the durable outcome: {followup:?}"
    );
    assert_eq!(deviantart_state(&held, 0), "published_or_scheduled");
    let title = &held.store.read().unwrap()[1]["title"];
    assert_eq!(
        title,
        if flushes {
            "Edited during publish"
        } else {
            "Title"
        }
    );
}

#[tokio::test]
async fn discard_or_refresh_during_publish_never_exits_before_the_durable_outcome() {
    for dirty in [false, true] {
        close_route_waits_for_publisher(Message::ConfirmDiscard, dirty).await;
        close_route_waits_for_publisher(Message::CloseRequested, dirty).await;
    }
}

#[tokio::test]
async fn close_exits_once_only_after_the_final_draft_flush_succeeds() {
    assert_eq!(
        common::ui::run(iced::exit()).await.exits,
        1,
        "an immediate exit is observed, never mistaken for waiting"
    );
    let (mut app, store, _root) = scheduled_jump_app();
    let before = std::fs::read(&store.paths().json).unwrap();
    for (title, saves) in [("x".repeat(51), false), ("Saved before close".into(), true)] {
        let _ = app.update(Message::Edited {
            uuid: common::id(9),
            field: Field::Title,
            text: title,
        });
        let save = common::ui::run(app.update(Message::CloseRequested)).await;
        assert_eq!(save.exits, 0, "no exit while the final save is pending");
        let [finished @ Message::SaveFinished { result, .. }] = &save.messages[..] else {
            panic!("close starts exactly one save: {save:?}");
        };
        assert_eq!(result.is_ok(), saves);
        let after = common::ui::run(app.update(finished.clone())).await;
        assert_eq!(after.exits, usize::from(saves), "{after:?}");
    }
    assert_ne!(std::fs::read(&store.paths().json).unwrap(), before);
    assert_eq!(store.read().unwrap()[0]["title"], "Saved before close");
}

#[tokio::test]
async fn publish_options_ui_persists_plural_allow_lists_zero_and_clear() {
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "0".into(),
    });
    let _ = app.update(Message::OptionTierChanged {
        uuid: uuid.clone(),
        value: "Silver".into(),
        selected: true,
    });
    let _ = app.update(Message::OptionGalleryChanged {
        uuid: uuid.clone(),
        value: "Featured".into(),
        selected: true,
    });
    let first_save_messages = common::ui::send(&mut app, Message::SaveOptions(uuid.clone())).await;
    assert!(
        first_save_messages
            .any(|message| matches!(message, Message::OptionsSaved { result: Ok(()), .. }))
    );
    assert!(first_save_messages.any(|message| matches!(message, Message::LoadFinished(_, Ok(_)))));
    assert!(
        first_save_messages
            .any(|message| matches!(message, Message::PublishStatusesFinished(Ok(_))))
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(saved[0]["apparitions"][0]["priceIfNotFree"], 0.0);
    assert_eq!(
        saved[0]["apparitions"][0]["tiers"],
        serde_json::json!(["Gold", "Silver"])
    );
    assert_eq!(
        saved[0]["apparitions"][0]["galleries"],
        serde_json::json!(["Art", "Featured"])
    );

    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: String::new(),
    });
    let clear_messages = common::ui::send(&mut app, Message::SaveOptions(uuid)).await;
    assert!(
        clear_messages
            .any(|message| matches!(message, Message::OptionsSaved { result: Ok(()), .. })),
        "cleared option edit was not admitted; task messages: {clear_messages:?}"
    );
    let cleared_bytes = std::fs::read(&store.paths().json).unwrap();
    let cleared: serde_json::Value = serde_json::from_slice(&cleared_bytes).unwrap();
    assert!(
        cleared[0]["apparitions"][0].get("priceIfNotFree").is_none(),
        "clear task messages: {clear_messages:?}; persisted bytes: {}",
        String::from_utf8_lossy(&cleared_bytes)
    );
    let _: iced::Element<'_, Message> = publicator::ui::publish::view(&app);
}

#[tokio::test]
async fn publish_flushes_valid_option_edits_before_admission() {
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::OptionPriceChanged {
        uuid,
        value: "6.25".into(),
    });
    let messages = common::ui::send(&mut app, Message::PublishSelected).await;
    // Admitted after the flush, then refused by this app's one deliberate gap: no publisher.
    let refused = Err("invalid state: publisher is not configured".to_owned());
    assert!(
        messages
            .any(|message| matches!(message, Message::PublishFinished(_, completion) if completion.result == refused)),
        "{messages:?}"
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(saved[0]["apparitions"][0]["priceIfNotFree"], 6.25);

    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::OptionPriceChanged {
        uuid,
        value: "NaN".into(),
    });
    let before = std::fs::read(&store.paths().json).unwrap();
    let messages = common::ui::send(&mut app, Message::PublishSelected).await;
    assert!(
        messages.any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert!(!messages.any(|message| matches!(message, Message::PublishFinished(..))));
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
}

#[tokio::test]
async fn option_edits_flush_before_navigation() {
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::OptionTierChanged {
        uuid,
        value: "Silver".into(),
        selected: true,
    });
    common::ui::send(&mut app, Message::Navigate(Tab::Calendar)).await;
    assert_eq!(app.current_tab(), Tab::Calendar);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(
        saved[0]["apparitions"][0]["tiers"],
        serde_json::json!(["Gold", "Silver"])
    );
}

#[tokio::test]
async fn successful_cell_options_are_locked_and_unconfigured_values_do_not_write() {
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let before = std::fs::read(&store.paths().json).unwrap();
    let _ = app.update(Message::OptionTierChanged {
        uuid: uuid.clone(),
        value: "Not configured".into(),
        selected: true,
    });
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);

    store
        .mutate(|rows| {
            rows[0]["state"] = serde_json::json!("APPEARED");
            rows[0]["apparitions"][0]["state"] = serde_json::json!("published_or_scheduled");
            Ok(())
        })
        .unwrap();
    let saved_success = std::fs::read(&store.paths().json).unwrap();
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "0".into(),
    });
    let messages = common::ui::send(&mut app, Message::SaveOptions(uuid)).await;
    assert!(
        messages.any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), saved_success);
}

#[tokio::test]
async fn unresolved_or_busy_publish_options_cannot_be_saved() {
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let before = std::fs::read(&store.paths().json).unwrap();
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "3".into(),
    });
    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::from([uuid.clone()]),
    )));
    let blocked = common::ui::send(&mut app, Message::SaveOptions(uuid.clone())).await;
    assert!(blocked.any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. })));
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);

    let publish_task = app.update(Message::PublishSelected);
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "4".into(),
    });
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    for message in common::ui::run(publish_task).await.messages {
        let _ = app.update(message);
    }
}

#[tokio::test]
async fn active_publish_locks_option_edits() {
    let (mut app, store, _root) = options_app();
    let before = std::fs::read(&store.paths().json).unwrap();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let publish = app.update(Message::PublishSelected);
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "8".into(),
    });
    let options = app.update(Message::SaveOptions(uuid));
    let rejected = common::ui::run(options).await;
    assert!(
        rejected.any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    for message in common::ui::run(publish).await.messages {
        let _ = app.update(message);
    }
}

#[tokio::test]
async fn return_to_describe_rolls_back_only_an_unlocked_idle_row() {
    let (mut app, store, _root) = options_app();
    let before = std::fs::read(&store.paths().json).unwrap();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::PublishLocksFinished(Ok([uuid.clone()].into())));
    let _ = common::ui::send(&mut app, Message::ReturnToDescribe(uuid.clone())).await;
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::new(),
    )));
    let publish = app.update(Message::PublishSelected);
    let _ = common::ui::run(app.update(Message::ReturnToDescribe(uuid.clone()))).await;
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    for message in common::ui::run(publish).await.messages {
        let _ = app.update(message);
    }
    let _ = app.update(Message::LoadFinished(0, Ok(store.read().unwrap().into())));
    let _ = app.update(Message::PublishLocksFinished(Ok(
        std::collections::HashSet::new(),
    )));
    let task = app.update(Message::ReturnToDescribe(uuid));
    common::ui::finish_workflow(&mut app, task).await;
    let row = &store.read().unwrap()[0];
    assert_eq!(row["state"], "SCHEDULED");
    let original: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(row["title"], original[0]["title"]);
    assert_eq!(row["description"], original[0]["description"]);
    assert_eq!(app.status_message(), Some("Returned to Describe"));
}

#[tokio::test]
async fn invalid_publish_option_input_keeps_saved_bytes_unchanged() {
    let (mut app, store, _root) = options_app();
    let before = std::fs::read(&store.paths().json).unwrap();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    for value in ["-1", "NaN", "inf", "not a number"] {
        let _ = app.update(Message::OptionPriceChanged {
            uuid: uuid.clone(),
            value: value.into(),
        });
        let messages = common::ui::send(&mut app, Message::SaveOptions(uuid.clone())).await;
        assert!(
            messages.any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
        );
        assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    }
}

#[tokio::test]
async fn publish_failure_retry_after_restart_shows_real_cell_outcomes_and_skips_saved_success() {
    let f = publishing_fixture();
    let mut first = pending_publication(&f, 1);
    first["apparitions"] = serde_json::json!([
        {"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]},
        {"platformName":"historical-platform","state":"unpublished"}
    ]);
    let second = pending_publication(&f, 2);
    let third = pending_publication(&f, 3);
    f.write(serde_json::json!([first, second, third]));
    let calls = f.path("calls.log");
    let worker = fake_publisher_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{{"protocol":1,"type":"ready"}}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  count=$(grep -Fxc "$uuid" '{}' || true)
  echo "$uuid" >> '{}'
  if [ "$uuid" = '{}' ] && [ "$count" -eq 0 ]; then
    printf '{{"protocol":1,"type":"failed","attempt_id":"%s","uuid":"%s","error":"safe failure"}}\n' "$attempt" "$uuid"
  else
    printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid"
  fi ;;
close) exit 0 ;;
esac
done"#,
            calls.display(),
            calls.display(),
            common::id(2),
        ),
    );
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    common::ui::settle(&mut app, init).await;
    let first_messages = common::ui::send(&mut app, Message::PublishSelected).await;
    let first_report = first_messages
        .messages
        .iter()
        .find_map(|message| match message {
            Message::PublishFinished(_, completion) => completion.result.as_ref().ok(),
            _ => None,
        })
        .expect("Publisher outcome returned through App::update");
    assert_eq!(first_report.outcomes.len(), 3);
    assert_eq!(first_report.outcomes[0].state, PublishCellState::Success);
    assert_eq!(first_report.outcomes[1].state, PublishCellState::SafeFailed);
    assert_eq!(
        first_report.outcomes[2].state,
        PublishCellState::Unattempted
    );
    assert_eq!(
        app.publish_cell_outcome(&common::id(1)).unwrap().state,
        PublishCellState::Success
    );
    assert_eq!(
        app.publish_cell_outcome(&common::id(2)).unwrap().state,
        PublishCellState::SafeFailed
    );
    let pending = app.publish_cell_outcome(&common::id(3)).unwrap();
    assert_eq!(pending.state, PublishCellState::Unattempted);
    assert!(pending.retryable());
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][1]["state"],
        "unpublished"
    );

    let (mut restarted, _store, init) = publishing_ui_app(&f, &worker);
    common::ui::settle(&mut restarted, init).await;
    assert_eq!(
        restarted
            .publish_cell_outcome(&common::id(1))
            .unwrap()
            .state,
        PublishCellState::Success
    );
    assert_eq!(
        restarted
            .publish_cell_outcome(&common::id(2))
            .unwrap()
            .state,
        PublishCellState::SafeFailed
    );
    assert_eq!(
        restarted
            .publish_cell_outcome(&common::id(3))
            .unwrap()
            .state,
        PublishCellState::Unattempted
    );
    assert!(
        restarted
            .publish_cell_outcome(&common::id(2))
            .unwrap()
            .retryable()
    );
    let retry = common::ui::send(&mut restarted, Message::PublishSelected).await;
    let retry_report = retry
        .messages
        .iter()
        .find_map(|message| match message {
            Message::PublishFinished(_, completion) => completion.result.as_ref().ok(),
            _ => None,
        })
        .expect("retry result returned through App::update");
    assert_eq!(retry_report.outcomes.len(), 2);
    assert_eq!(retry_report.outcomes[0].state, PublishCellState::Success);
    assert_eq!(retry_report.outcomes[1].state, PublishCellState::Success);
    let sent = std::fs::read_to_string(calls)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(
        sent.iter().filter(|uuid| **uuid == common::id(1)).count(),
        1
    );
    assert_eq!(
        sent.iter().filter(|uuid| **uuid == common::id(2)).count(),
        2
    );
    assert_eq!(
        sent.iter().filter(|uuid| **uuid == common::id(3)).count(),
        1
    );
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][1]["state"],
        "unpublished"
    );
}

#[tokio::test]
async fn unresolved_unknown_blocks_resend_until_confirmed_not_submitted() {
    let f = publishing_fixture();
    f.write(serde_json::json!([
        pending_publication(&f, 1),
        pending_publication(&f, 2)
    ]));
    let calls = f.path("calls.log");
    let worker = fake_publisher_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{{"protocol":1,"type":"ready"}}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  echo "$uuid" >> '{}'
  if [ "$(wc -l < '{}' )" -eq 1 ]; then
    printf '{{"protocol":1,"type":"unknown","attempt_id":"%s","uuid":"%s","error":"confirmation lost"}}\n' "$attempt" "$uuid"
  else
    printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid"
  fi ;;
close) exit 0 ;;
esac
done"#,
            calls.display(),
            calls.display(),
        ),
    );
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    common::ui::settle(&mut app, init).await;
    let messages = common::ui::send(&mut app, Message::PublishSelected).await;
    assert!(messages.any(|message| matches!(message,
        Message::PublishFinished(_, completion)
            if completion.result.as_ref().is_ok_and(|report|
                report.outcomes[0].state == PublishCellState::Unknown
                    && report.outcomes[1].state == PublishCellState::Unattempted))));
    let attempt = app
        .publish_attempts()
        .iter()
        .find(|attempt| attempt.uuid == common::id(1))
        .unwrap()
        .clone();
    assert!(matches!(
        attempt.state,
        publicator::publish_journal::AttemptState::Unknown { .. }
    ));
    assert!(
        !app.publish_cell_outcome(&common::id(1))
            .unwrap()
            .retryable()
    );
    let before_resend = std::fs::read_to_string(&calls).unwrap();
    let blocked = common::ui::send(&mut app, Message::PublishSelected).await;
    assert!(!blocked.any(|message| matches!(message, Message::PublishFinished(..))));
    assert_eq!(std::fs::read_to_string(&calls).unwrap(), before_resend);

    let before_invalid = std::fs::read(store.paths().json.as_path()).unwrap();
    let deselected = common::ui::send(
        &mut app,
        Message::SelectCell {
            key: format!("{}:deviantart", attempt.uuid),
            selected: false,
        },
    )
    .await;
    assert!(!deselected.any(|message| matches!(message, Message::WorkflowFinished { .. })));
    let deselect_all = common::ui::send(&mut app, Message::SelectAll(false)).await;
    assert!(!deselect_all.any(|message| matches!(message, Message::WorkflowFinished { .. })));
    assert_eq!(
        std::fs::read(store.paths().json.as_path()).unwrap(),
        before_invalid
    );
    let receipt = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        attempt.attempt_id
    ));
    let receipt_before = std::fs::read(&receipt).unwrap();
    let invalid = common::ui::send(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id.clone(),
            outcome: ReconcileOutcome::Scheduled {
                actual_url: Some("javascript:alert(1)".into()),
            },
        },
    )
    .await;
    assert!(
        !invalid
            .any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
    );
    assert_eq!(std::fs::read(&receipt).unwrap(), receipt_before);
    assert_eq!(
        std::fs::read(store.paths().json.as_path()).unwrap(),
        before_invalid
    );

    let reconciled = common::ui::send(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id,
            outcome: ReconcileOutcome::NotSubmitted,
        },
    )
    .await;
    assert!(
        reconciled
            .any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
    );
    assert_eq!(
        app.publish_cell_outcome(&common::id(1)).unwrap().state,
        PublishCellState::ConfirmedNotSubmitted
    );
    assert!(
        app.publish_cell_outcome(&common::id(1))
            .unwrap()
            .retryable()
    );
    let retried = common::ui::send(&mut app, Message::PublishSelected).await;
    assert!(retried.any(|message| matches!(message,
        Message::PublishFinished(_, completion)
            if completion.result.as_ref().is_ok_and(|report| report.published == 2))));
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 3);
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
}

fn unknown_publisher_worker(f: &common::Fixture) -> PathBuf {
    fake_publisher_worker(
        f,
        r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{"protocol":1,"type":"ready"}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  printf '{"protocol":1,"type":"unknown","attempt_id":"%s","uuid":"%s","error":"confirmation lost"}\n' "$attempt" "$uuid"
  ;;
close) exit 0 ;;
esac
done"#,
    )
}

#[tokio::test]
async fn scheduled_reconciliation_refuses_changed_metadata_and_keeps_newer_fields_and_receipt() {
    let f = publishing_fixture();
    f.write(serde_json::json!([pending_publication(&f, 1)]));
    let worker = unknown_publisher_worker(&f);
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    common::ui::settle(&mut app, init).await;
    let published = common::ui::send(&mut app, Message::PublishSelected).await;
    assert!(
        !app.publish_attempts().is_empty(),
        "unknown worker must leave an attempt for reconciliation: {published:?}"
    );
    let attempt = app.publish_attempts()[0].clone();
    store
        .mutate(|rows| {
            rows[0]["title"] = serde_json::json!("Newer reviewed title");
            Ok(())
        })
        .unwrap();
    common::ui::send(&mut app, Message::Refresh).await;
    let current = app
        .publish_attempts()
        .iter()
        .find(|status| status.attempt_id == attempt.attempt_id)
        .unwrap();
    assert_eq!(current.current_matches, Some(false));
    let receipt = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        attempt.attempt_id
    ));
    let receipt_before = std::fs::read(&receipt).unwrap();
    let newer_bytes = std::fs::read(store.paths().json.as_path()).unwrap();
    let messages = common::ui::send(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id.clone(),
            outcome: ReconcileOutcome::Scheduled {
                actual_url: Some("https://example.test/verified".into()),
            },
        },
    )
    .await;
    assert!(messages.any(|message| matches!(message,
        Message::ReconcileFinished { result: Err(error), .. } if error.contains("approved content/options changed"))));
    assert_eq!(std::fs::read(&receipt).unwrap(), receipt_before);
    assert_eq!(
        std::fs::read(store.paths().json.as_path()).unwrap(),
        newer_bytes
    );
    assert_eq!(store.read().unwrap()[0]["title"], "Newer reviewed title");
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["state"],
        "unpublished"
    );
    assert_eq!(
        app.publish_cell_outcome(&common::id(1)).unwrap().state,
        PublishCellState::Unknown
    );
}

#[tokio::test]
async fn verified_scheduled_reconciliation_accepts_optional_actual_http_url() {
    let f = publishing_fixture();
    f.write(serde_json::json!([
        pending_publication(&f, 1),
        pending_publication(&f, 2)
    ]));
    let worker = unknown_publisher_worker(&f);
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    common::ui::settle(&mut app, init).await;
    common::ui::send(&mut app, Message::PublishSelected).await;
    let attempt = app.publish_attempts()[0].clone();
    let receipt = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        attempt.attempt_id
    ));
    let before = std::fs::read(&receipt).unwrap();
    let invalid = common::ui::send(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id.clone(),
            outcome: ReconcileOutcome::Scheduled {
                actual_url: Some("https://".into()),
            },
        },
    )
    .await;
    assert!(invalid.any(|message| matches!(message,
        Message::ReconcileFinished { result: Err(error), .. } if error.contains("valid HTTP(S) URL"))));
    assert_eq!(std::fs::read(&receipt).unwrap(), before);
    let valid = common::ui::send(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id,
            outcome: ReconcileOutcome::Scheduled {
                actual_url: Some("https://example.test/verified".into()),
            },
        },
    )
    .await;
    assert!(
        valid.any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
    );
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["url"],
        "https://example.test/verified"
    );
    assert_eq!(
        app.publish_cell_outcome(&common::id(1)).unwrap().state,
        PublishCellState::Success
    );

    common::ui::send(&mut app, Message::PublishSelected).await;
    let second = app
        .publish_attempts()
        .iter()
        .find(|status| status.uuid == common::id(2))
        .unwrap()
        .clone();
    let without_url = common::ui::send(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: second.attempt_id,
            outcome: ReconcileOutcome::Scheduled { actual_url: None },
        },
    )
    .await;
    assert!(
        without_url
            .any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
    );
    let saved = store.read().unwrap();
    assert_eq!(
        saved[1]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert!(saved[1]["apparitions"][0].get("url").is_none());
}

#[test]
fn calendar_projection_keeps_every_apparition_and_includes_current_configured_zone_month() {
    let rows = vec![serde_json::json!({
        "uuid": common::id(1), "state":"APPEARED", "title":"Shared title",
        "description":"Description", "submissionTimestamp":common::epoch("2024-02-01T01:30:00Z"),
        "files":[{"basename":"art.png"}],
        "apparitions":[
            {"platformName":"deviantart","state":"published_or_scheduled","url":"https://example.test/actual"},
            {"platformName":"historical-pixiv","state":"unpublished"}
        ]
    })];
    let events = timeline(&rows).unwrap();
    let mut config = Config::default();
    config.schedule.insert(
        "timezone".into(),
        toml::Value::String("Pacific/Honolulu".into()),
    );
    let zone = scheduling::timezone(&config).unwrap();
    assert_eq!(zone, Honolulu);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].title, "Shared title");
    assert_eq!(events[0].timestamp, common::epoch("2024-02-01T01:30:00Z"));
    assert_eq!(
        events[0].url.as_deref(),
        Some("https://example.test/actual")
    );
    assert_eq!(events[1].platform, "historical-pixiv");
    assert_eq!(
        months(&events, Honolulu, common::epoch("2024-02-01T02:00:00Z")).unwrap(),
        vec![NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()]
    );
}

#[test]
fn shared_prompt_panel_preserves_literal_text_and_labels_all_nearest_lineages() {
    let literal = "<literal> [[PUBLICATOR:TODO_GENERATE]]\n雪";
    let exact = PromptMatch::Exact(PromptVersion {
        digest: "exact-digest".into(),
        text: literal.into(),
        paths: vec!["one.txt".into()],
        committed: 1,
    });
    assert_eq!(prompt::plain_text(&exact), vec!["Archived prompt", literal]);
    let nearest = PromptMatch::Nearest(vec![
        Lineage {
            path: "a/first.txt".into(),
            versions: vec![PromptVersion {
                digest: "first".into(),
                text: "first <candidate>".into(),
                paths: vec!["a/first.txt".into()],
                committed: 1,
            }],
        },
        Lineage {
            path: "b/second.txt".into(),
            versions: vec![PromptVersion {
                digest: "second".into(),
                text: "second [[PUBLICATOR:TODO_GENERATE]]".into(),
                paths: vec!["b/second.txt".into()],
                committed: 2,
            }],
        },
    ]);
    let lines = prompt::plain_text(&nearest);
    assert!(lines[0].contains("NOT archived"));
    assert!(lines[0].contains("not authoritative"));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("a/first.txt") && line.contains("first <candidate>"))
    );
    assert!(lines.iter().any(|line| line.contains("b/second.txt")
        && line.contains("second [[PUBLICATOR:TODO_GENERATE]]")));
    assert_eq!(
        prompt::plain_text(&PromptMatch::Unknown("no grammar match <x>".into())),
        vec!["Prompt unknown: no grammar match <x>"]
    );
    let _: iced::Element<'_, Message> = prompt::view(&exact);
}

fn capture_url_without_opening(url: &str) -> Result<(), String> {
    Err(format!("captured without browser: {url}"))
}

#[tokio::test]
async fn actual_url_message_dispatches_valid_http_url_and_refuses_invalid_full_url() {
    let f = common::Fixture::new();
    let store = Arc::new(f.store.clone());
    let mut row = common::row(1, "APPEARED");
    row["apparitions"] = serde_json::json!([{
        "platformName":"deviantart", "state":"published_or_scheduled",
        "url":"https://example.test/actual?x=1"
    }]);
    std::fs::write(&store.paths().json, serde_json::to_vec(&vec![row]).unwrap()).unwrap();
    let application = Arc::new(common::offline_app(store));
    let (mut app, init) = App::from_application_with_url_opener(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
        capture_url_without_opening,
    );
    common::ui::settle(&mut app, init).await;
    let valid = common::ui::run(app.update(Message::OpenActualUrl(
        "https://example.test/actual?x=1".into(),
    )))
    .await;
    assert!(valid.any(|message| matches!(message,
        Message::UrlOpened { result: Err(error), .. } if error == "captured without browser: https://example.test/actual?x=1")));
    let unrecorded =
        common::ui::run(app.update(Message::OpenActualUrl("https://unrecorded.test/".into())))
            .await;
    assert!(unrecorded.any(|message| matches!(message,
        Message::UrlOpened { result: Err(error), .. } if error == "URL is not stored on an apparition")));
    let invalid = common::ui::run(app.update(Message::OpenActualUrl("https://".into()))).await;
    assert!(invalid.any(|message| matches!(message,
        Message::UrlOpened { result: Err(error), .. } if error.contains("valid HTTP(S) URL"))));
}

#[tokio::test]
async fn configured_prompt_archive_resolves_pick_and_calendar_off_thread_and_missing_art_is_recorded()
 {
    let config = PromptConfig {
        repo: "prompt-archive".into(),
        filename: r"(?P<lineage>[^_]+)_(?P<version>[0-9a-f]{40})\.png".into(),
        version_hash: "sha1".into(),
    };
    let f = common::Fixture::with_config(Config {
        publicable: vec!["art".into()],
        prompts: Some(config.clone()),
        ..Config::default()
    });
    let repo = f.path("prompt-archive");
    std::fs::create_dir_all(repo.join("archive")).unwrap();
    let git = |args: &[&str]| common::git::git(&repo, args);
    git(&["init", "-q"]);
    std::fs::write(repo.join("archive/lineage"), "exact <prompt>\nline two").unwrap();
    git(&["add", "--", "archive/lineage"]);
    git(&["commit", "-qm", "archive prompt"]);
    let exact_digest = PromptGrammar::compile(&config)
        .unwrap()
        .digest(b"exact <prompt>\nline two")
        .unwrap();
    let art = f.path("art");
    std::fs::create_dir_all(&art).unwrap();
    let exact_path = art.join(format!("lineage_{exact_digest}.png"));
    let nearest_path = art.join(format!("lineage_{}.png", "f".repeat(40)));
    let unknown_path = art.join(format!("unknown_{}.png", "e".repeat(40)));
    for path in [&exact_path, &nearest_path, &unknown_path] {
        common::solid_image(path, "white", 4, 3);
    }
    let exact_basename = exact_path.file_name().unwrap().to_str().unwrap().to_owned();
    f.write(serde_json::json!([
        {"uuid":common::id(1),"state":"APPEARED","title":"Historical title","description":"Description","submissionTimestamp":1700000000,
         "files":[{"basename":exact_basename,"sha512sum":publicator::images::file_sha512(&exact_path).unwrap()}],
         "apparitions":[{"platformName":"deviantart","state":"published_or_scheduled","url":"https://example.test/actual"},{"platformName":"historic","state":"unpublished"}]},
        {"uuid":common::id(2),"state":"APPEARED","title":"Missing art title","description":"Description","submissionTimestamp":1700000000,
         "files":[{"basename":"missing.png","sha512sum":"0".repeat(128)}],
         "apparitions":[{"platformName":"old-platform","state":"unpublished"}]}
    ]));
    let application = Arc::new(common::offline_app(Arc::new(f.store.clone())));
    let (mut app, init) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    common::ui::settle(&mut app, init).await;
    let candidates = [&exact_path, &nearest_path, &unknown_path]
        .into_iter()
        .map(|path| publicator::images::Candidate {
            path: path.to_path_buf(),
            sha512: publicator::images::file_sha512(path).unwrap(),
        })
        .collect();
    common::ui::send(
        &mut app,
        Message::SearchFinished {
            request_id: 0,
            result: Ok(candidates),
        },
    )
    .await;
    assert!(
        matches!(app.prompt_match(&exact_path), Some(PromptMatch::Exact(version)) if version.text == "exact <prompt>\nline two")
    );
    assert!(
        matches!(app.prompt_match(&nearest_path), Some(PromptMatch::Nearest(candidates)) if candidates.iter().any(|candidate| candidate.path == "archive/lineage"))
    );
    assert!(matches!(
        app.prompt_match(&unknown_path),
        Some(PromptMatch::Unknown(_))
    ));
    let _: iced::Element<'_, Message> = publicator::ui::pick::view(&app);
    // Only an on-screen placeholder loads its preview: the exact candidate's.
    common::ui::send(
        &mut app,
        Message::PreviewShown {
            key: publicator::ui::pick::preview_key(&exact_path).into(),
            shown: true,
        },
    )
    .await;

    let calendar_messages = common::ui::send(&mut app, Message::Navigate(Tab::Calendar)).await;
    assert!(calendar_messages.any(|message| matches!(message,
        Message::CalendarPrepared(_, Ok(prepared))
            if prepared.artwork.get(&common::id(2)).is_some_and(Result::is_err)
                && prepared.prompts.iter().any(|(path, prompt)| path == &exact_path
                    && matches!(prompt, PromptMatch::Exact(version) if version.text == "exact <prompt>\nline two"))
                && prepared.prompts.iter().any(|(path, prompt)| path.ends_with("missing.png")
                    && matches!(prompt, PromptMatch::Unknown(_)))
    )));
    assert!(app.calendar_prepared());
    assert!(app.calendar_art(&common::id(2)).is_none());
    assert_eq!(std::fs::read_dir(f.path(".thumbs")).unwrap().count(), 1);
    assert!(matches!(
        app.prompt_match(&exact_path),
        Some(PromptMatch::Exact(_))
    ));
    let _: iced::Element<'_, Message> = publicator::ui::calendar::view(&app);
}

/// `rows` loaded with each one's first Calendar event expanded, then the Calendar opened and
/// settled; returns what opening it produced.
async fn expanded_calendar(
    f: &common::Fixture,
    rows: &[serde_json::Value],
) -> (App, common::ui::Effects) {
    f.write(serde_json::json!(rows));
    let (mut app, init) = offline_ui(f);
    common::ui::settle(&mut app, init).await;
    for row in rows {
        let uuid = row["uuid"].as_str().unwrap();
        let _ = app.update(Message::ToggleCalendarEvent(format!("{uuid}:0")));
    }
    let effects = common::ui::send(&mut app, Message::Navigate(Tab::Calendar)).await;
    assert!(app.calendar_prepared());
    (app, effects)
}

#[tokio::test]
async fn calendar_rows_sharing_a_basename_get_their_own_artwork_and_preview() {
    let f = common::Fixture::new();
    let (mut rows, images, mut expected) = f.namesake_rows("DESCRIBED");
    for publication in &mut rows {
        publication["apparitions"] =
            serde_json::json!([{"platformName":"deviantart","state":"unpublished"}]);
    }
    let (app, effects) = expanded_calendar(&f, &rows).await;
    for (n, (image, thumb)) in [1, 2].into_iter().zip(images.iter().zip(&expected)) {
        assert_eq!(
            app.calendar_art(&common::id(n)).map(|(path, _)| path),
            Some(image.as_path())
        );
        assert!(
            effects.any(|message| matches!(message,
                Message::CalendarPreviewFinished { uuid, result: Ok(path), .. }
                    if *uuid == common::id(n) && path.ends_with(thumb))),
            "{:?}",
            effects.messages
        );
    }
    expected.sort();
    assert_eq!(common::thumbs(&f), expected);
    let _: iced::Element<'_, Message> = publicator::ui::calendar::view(&app);
}

#[tokio::test]
async fn calendar_row_without_recorded_hash_shows_an_error_not_an_unverified_preview() {
    let f = common::Fixture::new();
    common::solid_image(&f.path("art.png"), "white", 4, 3);
    let mut publication = common::row(1, "APPEARED");
    publication["files"][0] = serde_json::json!({"basename": "art.png"});
    publication["apparitions"] =
        serde_json::json!([{"platformName":"deviantart","state":"published_or_scheduled"}]);
    let (app, effects) = expanded_calendar(&f, &[publication]).await;
    assert!(effects.any(|message| matches!(message,
        Message::CalendarPrepared(_, Ok(prepared))
            if matches!(&prepared.artwork[&common::id(1)], Err(error) if error.contains("artwork hash is missing")))));
    assert_eq!(app.calendar_art(&common::id(1)), None);
    assert!(
        common::thumbs(&f).is_empty(),
        "no preview of unverified bytes"
    );
    let _: iced::Element<'_, Message> = publicator::ui::calendar::view(&app);
}

#[tokio::test]
async fn publication_jump_flushes_draft_then_reveals_target_phase_and_scroll_action() {
    let (mut app, store, _root) = scheduled_jump_app();
    let uuid = common::id(9);
    let before = app.current_tab();
    let _ = app.update(Message::Edited {
        uuid: uuid.clone(),
        field: Field::Title,
        text: "Saved before jump".into(),
    });
    let task = app.update(Message::JumpToPublication(uuid.clone()));
    assert_eq!(
        app.current_tab(),
        before,
        "navigation waits for the dirty draft save"
    );
    let completion = common::ui::first_output(task).await;
    assert!(
        matches!(completion, Message::SaveFinished { result: Ok(()), .. }),
        "jump save failed: {completion:?}"
    );
    let followup = common::ui::run(app.update(completion)).await;
    assert!(followup.widgets > 0, "target scroll operation is returned");
    assert_eq!(app.current_tab(), Tab::Describe);
    assert_eq!(app.revealed_publication(), Some(uuid.as_str()));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(saved[0]["title"], "Saved before jump");
    let _: iced::Element<'_, Message> = publicator::ui::describe::view(&app);
}

#[tokio::test]
async fn publication_jump_maps_all_phases_to_the_next_action() {
    let (mut app, store, _root) = options_app();
    for (phase, expected) in [
        ("PICKED", Tab::Schedule),
        ("SCHEDULED", Tab::Describe),
        ("DESCRIBED", Tab::Publish),
        ("APPEARED", Tab::Publish),
    ] {
        store
            .mutate(|rows| {
                rows[0]["state"] = serde_json::json!(phase);
                if phase == "APPEARED" {
                    rows[0]["apparitions"][0]["state"] =
                        serde_json::json!("published_or_scheduled");
                }
                Ok(())
            })
            .unwrap();
        let _ = app.update(Message::LoadFinished(0, Ok(store.read().unwrap().into())));
        let task = app.update(Message::JumpToPublication(common::id(9)));
        assert_eq!(app.current_tab(), expected);
        assert!(
            common::ui::run(task).await.widgets > 0,
            "scroll action is returned"
        );
    }
}

#[tokio::test]
async fn failed_jump_flush_keeps_phase_and_draft_without_revealing_or_exiting() {
    let (mut app, store, _root) = scheduled_jump_app();
    let uuid = common::id(9);
    let before = std::fs::read(&store.paths().json).unwrap();
    let _ = app.update(Message::Edited {
        uuid: uuid.clone(),
        field: Field::Title,
        text: "x".repeat(51),
    });
    let messages = common::ui::send(&mut app, Message::JumpToPublication(uuid)).await;
    assert!(
        messages.any(|message| matches!(message, Message::SaveFinished { result: Err(_), .. })),
        "messages: {messages:?}"
    );
    assert_eq!((messages.exits, messages.widgets), (0, 0));
    assert_eq!(app.current_tab(), Tab::Pick);
    assert_eq!(app.revealed_publication(), None);
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    let retry = common::ui::send(&mut app, Message::RetrySave).await;
    assert!(retry.any(|message| matches!(message, Message::SaveFinished { result: Err(_), .. })));
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
}
