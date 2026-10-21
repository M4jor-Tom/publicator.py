#[cfg(feature = "desktop")]
mod common;

#[cfg(feature = "desktop")]
#[test]
fn selection_is_partial_when_only_one_pending_cell_is_checked() {
    use publicator::ui::publish::{SelectionState, selection_state};

    assert_eq!(selection_state(&[true, false]), SelectionState::Partial);
    assert_eq!(selection_state(&[]), SelectionState::None);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publish_checkbox_persists_registered_target_through_ui_task() {
    use iced_runtime::{Action, futures::futures::StreamExt};
    use publicator::{
        application::Application,
        config::{Config, Paths},
        metadata::{Endpoints, MetadataClient},
        ui::{App, Message, UiOptions},
    };
    use std::{sync::Arc, time::Duration};

    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let store = Arc::new(publicator::store::Store::new(paths, Config::default()).unwrap());
    let row = serde_json::json!({
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED",
        "title":"Title", "description":"Description", "submissionTimestamp":1700000000,
        "files":[{"basename":"art.png","sha512sum":"0".repeat(128)}], "apparitions":[]
    });
    std::fs::write(
        &store.paths().json,
        serde_json::to_vec(&vec![row.clone()]).unwrap(),
    )
    .unwrap();
    let application = Arc::new(Application::new(
        store.clone(),
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        ),
        || 0,
    ));
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    let _ = app.update(Message::LoadFinished(Ok(vec![row])));
    let task = app.update(Message::SelectCell {
        key: "00000000-0000-4000-8000-000000000001:deviantart".into(),
        selected: true,
    });
    let mut actions = iced_runtime::task::into_stream(task).expect("workflow task dispatched");
    let completion = loop {
        match actions.next().await {
            Some(Action::Output(message)) => break message,
            Some(_) => continue,
            None => panic!("workflow task produced no completion"),
        }
    };
    assert!(matches!(
        &completion,
        Message::WorkflowFinished { result: Ok(()), .. }
    ));
    let _ = app.update(completion);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(
        saved[0]["apparitions"],
        serde_json::json!([
            {"platformName":"deviantart","state":"unpublished"}
        ])
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publish_dispatch_rejects_a_stale_saved_cell_set_before_worker_start() {
    use iced_runtime::{Action, futures::futures::StreamExt};
    use publicator::{
        application::Application,
        config::{Config, Paths},
        metadata::{Endpoints, MetadataClient},
        publishing::Publisher,
        ui::{App, Message, UiOptions},
    };
    use std::{sync::Arc, time::Duration};

    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let store = Arc::new(publicator::store::Store::new(paths, Config::default()).unwrap());
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
        Application::new(
            store.clone(),
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints::default(),
                "llmfit".into(),
            ),
            || 0,
        )
        .with_publisher(Publisher::new(
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
    let _ = app.update(Message::LoadFinished(Ok(vec![row.clone()])));
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

    let task = app.update(Message::PublishSelected);
    let mut actions = iced_runtime::task::into_stream(task).expect("publish task dispatched");
    let completion = loop {
        match actions.next().await {
            Some(Action::Output(message)) => break message,
            Some(_) => continue,
            None => panic!("publish task produced no completion"),
        }
    };
    match &completion {
        Message::PublishFinished(completion) => {
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

#[cfg(feature = "desktop")]
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
    let events = publicator::calendar::timeline(&rows).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].url.as_deref(), Some("https://example.test/item"));
    assert_eq!(events[0].title, "<literal>");
}

#[cfg(feature = "desktop")]
async fn run_ui_task(task: iced::Task<publicator::ui::Message>) -> Vec<publicator::ui::Message> {
    use iced_runtime::{Action, futures::futures::StreamExt};
    let Some(mut actions) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    while let Some(action) = actions.next().await {
        if let Action::Output(message) = action {
            messages.push(message);
        }
    }
    messages
}

#[cfg(feature = "desktop")]
async fn dispatch_options_message(
    app: &mut publicator::ui::App,
    message: publicator::ui::Message,
) -> Vec<publicator::ui::Message> {
    let mut pending = run_ui_task(app.update(message)).await;
    let mut completed = Vec::new();
    while !pending.is_empty() {
        let mut next = Vec::new();
        for message in pending {
            next.extend(run_ui_task(app.update(message.clone())).await);
            completed.push(message);
        }
        pending = next;
    }
    completed
}

#[cfg(feature = "desktop")]
async fn dispatch_ui_task(
    app: &mut publicator::ui::App,
    task: iced::Task<publicator::ui::Message>,
) -> Vec<publicator::ui::Message> {
    let mut pending = run_ui_task(task).await;
    let mut completed = Vec::new();
    while !pending.is_empty() {
        let mut next = Vec::new();
        for message in pending {
            next.extend(run_ui_task(app.update(message.clone())).await);
            completed.push(message);
        }
        pending = next;
    }
    completed
}

#[cfg(feature = "desktop")]
async fn dispatch_ui_message(
    app: &mut publicator::ui::App,
    message: publicator::ui::Message,
) -> Vec<publicator::ui::Message> {
    let task = app.update(message);
    dispatch_ui_task(app, task).await
}

#[cfg(feature = "desktop")]
fn publishing_fixture() -> common::Fixture {
    use publicator::config::Config;
    let f = common::Fixture::with_config(Config {
        tags: Some("tags.txt".into()),
        ..Config::default()
    });
    std::fs::write(f.path("tags.txt"), "fake tag\n").unwrap();
    f
}

#[cfg(feature = "desktop")]
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

#[cfg(feature = "desktop")]
fn publishing_ui_app(
    f: &common::Fixture,
    worker: &std::path::Path,
) -> (
    publicator::ui::App,
    std::sync::Arc<publicator::store::Store>,
    iced::Task<publicator::ui::Message>,
) {
    use publicator::{
        application::Application,
        metadata::{Endpoints, MetadataClient},
        publishing::Publisher,
        ui::{App, UiOptions},
    };
    use std::{sync::Arc, time::Duration};
    let store = Arc::new(f.store.clone());
    let application = Arc::new(
        Application::new(
            store.clone(),
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints {
                    openrouter: "http://127.0.0.1:1".into(),
                    ollama: "http://127.0.0.1:1".into(),
                },
                "llmfit".into(),
            ),
            || 0,
        )
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

#[cfg(feature = "desktop")]
fn fake_publisher_worker(f: &common::Fixture, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let worker = f.path("worker.sh");
    std::fs::write(
        &worker,
        format!(
            r#"#!/bin/sh
[ "$1" = "--check-steps" ] && exit 0
{body}
"#
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&worker, permissions).unwrap();
    worker
}

#[cfg(feature = "desktop")]
fn options_app() -> (
    publicator::ui::App,
    std::sync::Arc<publicator::store::Store>,
    tempfile::TempDir,
) {
    use publicator::{
        application::Application,
        config::{Config, Paths},
        metadata::{Endpoints, MetadataClient},
        ui::{App, UiOptions},
    };
    use std::{sync::Arc, time::Duration};
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let config = Config {
        tiers: vec!["Gold".into(), "Silver".into()],
        galleries: vec!["Art".into(), "Featured".into()],
        ..Config::default()
    };
    let store = Arc::new(publicator::store::Store::new(paths, config).unwrap());
    let row = serde_json::json!({
        "uuid":"00000000-0000-4000-8000-000000000009", "state":"DESCRIBED",
        "title":"Title", "description":"Description", "submissionTimestamp":1700000000,
        "files":[{"basename":"art.png","sha512sum":"0".repeat(128)}],
        "apparitions":[{"platformName":"deviantart","state":"unpublished","priceIfNotFree":2.5,"tiers":["Gold"],"galleries":["Art"]}]
    });
    std::fs::write(
        &store.paths().json,
        serde_json::to_vec(&vec![row.clone()]).unwrap(),
    )
    .unwrap();
    let application = Arc::new(Application::new(
        store.clone(),
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        ),
        || 0,
    ));
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    let _ = app.update(publicator::ui::Message::LoadFinished(Ok(vec![row])));
    let _ = app.update(publicator::ui::Message::PublishLocksFinished(Ok(
        std::collections::HashSet::new(),
    )));
    (app, store, root)
}

#[cfg(feature = "desktop")]
fn scheduled_jump_app() -> (
    publicator::ui::App,
    std::sync::Arc<publicator::store::Store>,
    tempfile::TempDir,
) {
    use publicator::ui::Message;
    let (mut app, store, root) = options_app();
    store
        .mutate(|rows| {
            rows[0]["state"] = serde_json::json!("SCHEDULED");
            Ok(())
        })
        .unwrap();
    let _ = app.update(Message::LoadFinished(Ok(store.read().unwrap())));
    (app, store, root)
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn close_cancel_stops_after_current_receipt_without_dropping_publisher_task() {
    use iced_runtime::{Action, futures::futures::StreamExt};
    use publicator::ui::Message;
    use std::time::Duration;

    let f = publishing_fixture();
    f.write(serde_json::json!([
        pending_publication(&f, 32),
        pending_publication(&f, 33)
    ]));
    let started = f.path("cancel-worker-started");
    let release = f.path("cancel-worker-release");
    let calls = f.path("cancel-worker-calls");
    let body = format!(
        r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{{"protocol":1,"type":"ready"}}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  echo "$uuid" >> '{}'
  if [ ! -e '{}' ]; then touch '{}'; while [ ! -e '{}' ]; do sleep 0.01; done; fi
  printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid" ;;
close) exit 0 ;;
esac
done"#,
        calls.display(),
        started.display(),
        started.display(),
        release.display()
    );
    let worker = fake_publisher_worker(&f, &body);
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    dispatch_ui_task(&mut app, init).await;
    let task = app.update(Message::PublishSelected);
    let mut actions = iced_runtime::task::into_stream(task).expect("publish task started");
    tokio::time::timeout(Duration::from_secs(5), async {
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
    })
    .await
    .expect("first delivery reached the controlled worker gate");

    let _ = app.update(Message::CloseRequested);
    let _ = app.update(Message::CloseCancelPublish);
    std::fs::write(&release, "continue").unwrap();
    let completion = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Action::Output(message)) = actions.next().await {
                break message;
            }
        }
    })
    .await
    .expect("cancel waits until the first receipt and worker cleanup");
    assert!(matches!(completion, Message::PublishFinished(_)));
    let task = app.update(completion);
    let followup = dispatch_ui_task(&mut app, task).await;
    assert!(
        followup
            .iter()
            .any(|message| matches!(message, Message::PublishStatusesFinished(Ok(_))))
    );
    let calls = std::fs::read_to_string(calls).unwrap();
    assert_eq!(calls.lines().collect::<Vec<_>>(), [common::id(32)]);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(
        saved[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert_eq!(saved[1]["apparitions"][0]["state"], "unpublished");
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn close_saves_option_drafts_and_invalid_option_blocks_close() {
    use publicator::ui::Message;
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
    let refused = dispatch_options_message(&mut app, Message::CloseRequested).await;
    assert!(refused.iter().any(|message| matches!(
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
    let failed = dispatch_options_message(&mut app, Message::CloseRequested).await;
    assert!(failed.iter().any(|message| matches!(
        message,
        Message::OptionsSaved { result: Err(error), .. } if error.contains("finite nonnegative")
    )));
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);

    let _ = app.update(Message::OptionPriceChanged {
        uuid,
        value: "0".into(),
    });
    let saved = dispatch_options_message(&mut app, Message::CloseRequested).await;
    assert!(
        saved
            .iter()
            .any(|message| matches!(message, Message::OptionsSaved { result: Ok(()), .. }))
    );
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(persisted[0]["apparitions"][0]["priceIfNotFree"], 0.0);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn close_during_delayed_publish_waits_for_durable_receipt_and_cleanup() {
    use iced_runtime::{Action, futures::futures::StreamExt};
    use publicator::ui::Message;
    use std::time::Duration;

    let f = publishing_fixture();
    f.write(serde_json::json!([pending_publication(&f, 31)]));
    let started = f.path("worker-started");
    let release = f.path("worker-release");
    let body = format!(
        r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{{"protocol":1,"type":"ready"}}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  touch '{}'
  while [ ! -e '{}' ]; do sleep 0.01; done
  printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid" ;;
close) exit 0 ;;
esac
done"#,
        started.display(),
        release.display()
    );
    let worker = fake_publisher_worker(&f, &body);
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    dispatch_ui_task(&mut app, init).await;
    let task = app.update(Message::PublishSelected);
    let mut actions = iced_runtime::task::into_stream(task).expect("publish task started");
    tokio::time::timeout(Duration::from_secs(5), async {
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
    })
    .await
    .expect("fake worker reached the controlled in-flight delivery");

    assert!(
        run_ui_task(app.update(Message::CloseRequested))
            .await
            .is_empty()
    );
    std::fs::write(&release, "continue").unwrap();

    let completion = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Action::Output(message)) = actions.next().await {
                break message;
            }
        }
    })
    .await
    .expect("publisher returns its durable result after worker cleanup");
    assert!(matches!(completion, Message::PublishFinished(_)));
    let task = app.update(completion);
    let followup = dispatch_ui_task(&mut app, task).await;
    assert!(
        followup
            .iter()
            .any(|message| matches!(message, Message::PublishStatusesFinished(Ok(_))))
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(
        saved[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publish_options_ui_persists_plural_allow_lists_zero_and_clear() {
    use publicator::ui::Message;
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
    dispatch_options_message(&mut app, Message::SaveOptions(uuid.clone())).await;
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
    dispatch_options_message(&mut app, Message::SaveOptions(uuid)).await;
    let cleared: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert!(cleared[0]["apparitions"][0].get("priceIfNotFree").is_none());
    let _ = app.update(Message::LoadFinished(Ok(store.read().unwrap())));
    let _: iced::Element<'_, Message> = publicator::ui::publish::view(&app);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publish_flushes_valid_option_edits_before_admission() {
    use publicator::ui::Message;
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::OptionPriceChanged {
        uuid,
        value: "6.25".into(),
    });
    let messages = dispatch_options_message(&mut app, Message::PublishSelected).await;
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::PublishFinished(completion) if completion.result.is_err()))
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
    let messages = dispatch_options_message(&mut app, Message::PublishSelected).await;
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, Message::PublishFinished(_)))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn option_edits_flush_before_navigation() {
    use publicator::ui::{Message, Tab};
    let (mut app, store, _root) = options_app();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let _ = app.update(Message::OptionTierChanged {
        uuid,
        value: "Silver".into(),
        selected: true,
    });
    dispatch_options_message(&mut app, Message::Navigate(Tab::Calendar)).await;
    assert_eq!(app.current_tab(), Tab::Calendar);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(
        saved[0]["apparitions"][0]["tiers"],
        serde_json::json!(["Gold", "Silver"])
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn successful_cell_options_are_locked_and_unconfigured_values_do_not_write() {
    use publicator::ui::Message;
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
    let messages = dispatch_options_message(&mut app, Message::SaveOptions(uuid)).await;
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), saved_success);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn unresolved_or_busy_publish_options_cannot_be_saved() {
    use publicator::ui::Message;
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
    let blocked = dispatch_options_message(&mut app, Message::SaveOptions(uuid.clone())).await;
    assert!(
        blocked
            .iter()
            .any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);

    let publish_task = app.update(Message::PublishSelected);
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "4".into(),
    });
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    for message in run_ui_task(publish_task).await {
        let _ = app.update(message);
    }
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn active_publish_locks_option_edits() {
    use publicator::ui::Message;
    let (mut app, store, _root) = options_app();
    let before = std::fs::read(&store.paths().json).unwrap();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    let publish = app.update(Message::PublishSelected);
    let _ = app.update(Message::OptionPriceChanged {
        uuid: uuid.clone(),
        value: "8".into(),
    });
    let options = app.update(Message::SaveOptions(uuid));
    let rejected = run_ui_task(options).await;
    assert!(
        rejected
            .iter()
            .any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    for message in run_ui_task(publish).await {
        let _ = app.update(message);
    }
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn invalid_publish_option_input_keeps_saved_bytes_unchanged() {
    use publicator::ui::Message;
    let (mut app, store, _root) = options_app();
    let before = std::fs::read(&store.paths().json).unwrap();
    let uuid = "00000000-0000-4000-8000-000000000009".to_owned();
    for value in ["-1", "NaN", "inf", "not a number"] {
        let _ = app.update(Message::OptionPriceChanged {
            uuid: uuid.clone(),
            value: value.into(),
        });
        let messages = dispatch_options_message(&mut app, Message::SaveOptions(uuid.clone())).await;
        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Message::OptionsSaved { result: Err(_), .. }))
        );
        assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    }
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publish_failure_retry_after_restart_shows_real_cell_outcomes_and_skips_saved_success() {
    use publicator::{publishing::PublishCellState, ui::Message};
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
    dispatch_ui_task(&mut app, init).await;
    let first_messages = dispatch_ui_message(&mut app, Message::PublishSelected).await;
    let first_report = first_messages
        .iter()
        .find_map(|message| match message {
            Message::PublishFinished(completion) => completion.result.as_ref().ok(),
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
    dispatch_ui_task(&mut restarted, init).await;
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
    let retry = dispatch_ui_message(&mut restarted, Message::PublishSelected).await;
    let retry_report = retry
        .iter()
        .find_map(|message| match message {
            Message::PublishFinished(completion) => completion.result.as_ref().ok(),
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

#[cfg(feature = "desktop")]
#[tokio::test]
async fn unresolved_unknown_blocks_resend_until_confirmed_not_submitted() {
    use publicator::{
        publish_journal::ReconcileOutcome, publishing::PublishCellState, ui::Message,
    };
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
    dispatch_ui_task(&mut app, init).await;
    let messages = dispatch_ui_message(&mut app, Message::PublishSelected).await;
    assert!(messages.iter().any(|message| matches!(message,
        Message::PublishFinished(completion)
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
    let blocked = dispatch_ui_message(&mut app, Message::PublishSelected).await;
    assert!(
        !blocked
            .iter()
            .any(|message| matches!(message, Message::PublishFinished(_)))
    );
    assert_eq!(std::fs::read_to_string(&calls).unwrap(), before_resend);

    let before_invalid = std::fs::read(store.paths().json.as_path()).unwrap();
    let deselected = dispatch_ui_message(
        &mut app,
        Message::SelectCell {
            key: format!("{}:deviantart", attempt.uuid),
            selected: false,
        },
    )
    .await;
    assert!(
        !deselected
            .iter()
            .any(|message| matches!(message, Message::WorkflowFinished { .. }))
    );
    let deselect_all = dispatch_ui_message(&mut app, Message::SelectAll(false)).await;
    assert!(
        !deselect_all
            .iter()
            .any(|message| matches!(message, Message::WorkflowFinished { .. }))
    );
    assert_eq!(
        std::fs::read(store.paths().json.as_path()).unwrap(),
        before_invalid
    );
    let receipt = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        attempt.attempt_id
    ));
    let receipt_before = std::fs::read(&receipt).unwrap();
    let invalid = dispatch_ui_message(
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
            .iter()
            .any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
    );
    assert_eq!(std::fs::read(&receipt).unwrap(), receipt_before);
    assert_eq!(
        std::fs::read(store.paths().json.as_path()).unwrap(),
        before_invalid
    );

    let reconciled = dispatch_ui_message(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id,
            outcome: ReconcileOutcome::NotSubmitted,
        },
    )
    .await;
    assert!(
        reconciled
            .iter()
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
    let retried = dispatch_ui_message(&mut app, Message::PublishSelected).await;
    assert!(retried.iter().any(|message| matches!(message,
        Message::PublishFinished(completion)
            if completion.result.as_ref().is_ok_and(|report| report.published == 2))));
    assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 3);
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn scheduled_reconciliation_refuses_changed_metadata_and_keeps_newer_fields_and_receipt() {
    use publicator::{
        publish_journal::ReconcileOutcome, publishing::PublishCellState, ui::Message,
    };
    let f = publishing_fixture();
    f.write(serde_json::json!([pending_publication(&f, 1)]));
    let worker = fake_publisher_worker(
        &f,
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
    );
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    dispatch_ui_task(&mut app, init).await;
    dispatch_ui_message(&mut app, Message::PublishSelected).await;
    let attempt = app.publish_attempts()[0].clone();
    store
        .mutate(|rows| {
            rows[0]["title"] = serde_json::json!("Newer reviewed title");
            Ok(())
        })
        .unwrap();
    dispatch_ui_message(&mut app, Message::Refresh).await;
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
    let messages = dispatch_ui_message(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id.clone(),
            outcome: ReconcileOutcome::Scheduled {
                actual_url: Some("https://example.test/verified".into()),
            },
        },
    )
    .await;
    assert!(messages.iter().any(|message| matches!(message,
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

#[cfg(feature = "desktop")]
#[tokio::test]
async fn verified_scheduled_reconciliation_accepts_optional_actual_http_url() {
    use publicator::{
        publish_journal::ReconcileOutcome, publishing::PublishCellState, ui::Message,
    };
    let f = publishing_fixture();
    f.write(serde_json::json!([
        pending_publication(&f, 1),
        pending_publication(&f, 2)
    ]));
    let worker = fake_publisher_worker(
        &f,
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
    );
    let (mut app, store, init) = publishing_ui_app(&f, &worker);
    dispatch_ui_task(&mut app, init).await;
    dispatch_ui_message(&mut app, Message::PublishSelected).await;
    let attempt = app.publish_attempts()[0].clone();
    let receipt = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        attempt.attempt_id
    ));
    let before = std::fs::read(&receipt).unwrap();
    let invalid = dispatch_ui_message(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: attempt.attempt_id.clone(),
            outcome: ReconcileOutcome::Scheduled {
                actual_url: Some("https://".into()),
            },
        },
    )
    .await;
    assert!(invalid.iter().any(|message| matches!(message,
        Message::ReconcileFinished { result: Err(error), .. } if error.contains("valid HTTP(S) URL"))));
    assert_eq!(std::fs::read(&receipt).unwrap(), before);
    let valid = dispatch_ui_message(
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
        valid
            .iter()
            .any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
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

    dispatch_ui_message(&mut app, Message::PublishSelected).await;
    let second = app
        .publish_attempts()
        .iter()
        .find(|status| status.uuid == common::id(2))
        .unwrap()
        .clone();
    let without_url = dispatch_ui_message(
        &mut app,
        Message::ReconcileAttempt {
            attempt_id: second.attempt_id,
            outcome: ReconcileOutcome::Scheduled { actual_url: None },
        },
    )
    .await;
    assert!(
        without_url
            .iter()
            .any(|message| matches!(message, Message::ReconcileFinished { result: Ok(()), .. }))
    );
    let saved = store.read().unwrap();
    assert_eq!(
        saved[1]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert!(saved[1]["apparitions"][0].get("url").is_none());
}

#[cfg(feature = "desktop")]
#[test]
fn calendar_projection_keeps_every_apparition_and_includes_current_configured_zone_month() {
    use chrono::NaiveDate;
    use chrono_tz::Pacific::Honolulu;
    use publicator::{
        calendar::{months, timeline},
        config::Config,
        scheduling,
    };

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

#[cfg(feature = "desktop")]
#[test]
fn shared_prompt_panel_preserves_literal_text_and_labels_all_nearest_lineages() {
    use publicator::{
        prompts::{Lineage, PromptMatch, PromptVersion},
        ui::prompt,
    };

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
    let _: iced::Element<'_, publicator::ui::Message> = prompt::view(&exact);
}

#[cfg(feature = "desktop")]
fn capture_url_without_opening(url: &str) -> Result<(), String> {
    Err(format!("captured without browser: {url}"))
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn actual_url_message_dispatches_valid_http_url_and_refuses_invalid_full_url() {
    use publicator::{
        application::Application,
        config::{Config, Paths},
        metadata::{Endpoints, MetadataClient},
        ui::{App, Message, UiOptions},
    };
    use std::{sync::Arc, time::Duration};

    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let store = Arc::new(publicator::store::Store::new(paths, Config::default()).unwrap());
    let mut row = common::row(1, "APPEARED");
    row["apparitions"] = serde_json::json!([{
        "platformName":"deviantart", "state":"published_or_scheduled",
        "url":"https://example.test/actual?x=1"
    }]);
    std::fs::write(&store.paths().json, serde_json::to_vec(&vec![row]).unwrap()).unwrap();
    let application = Arc::new(Application::new(
        store,
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        ),
        || 0,
    ));
    let (mut app, init) = App::from_application_with_url_opener(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
        capture_url_without_opening,
    );
    dispatch_ui_task(&mut app, init).await;
    let valid = run_ui_task(app.update(Message::OpenActualUrl(
        "https://example.test/actual?x=1".into(),
    )))
    .await;
    assert!(valid.iter().any(|message| matches!(message,
        Message::UrlOpened { result: Err(error), .. } if error == "captured without browser: https://example.test/actual?x=1")));
    let unrecorded =
        run_ui_task(app.update(Message::OpenActualUrl("https://unrecorded.test/".into()))).await;
    assert!(unrecorded.iter().any(|message| matches!(message,
        Message::UrlOpened { result: Err(error), .. } if error == "URL is not stored on an apparition")));
    let invalid = run_ui_task(app.update(Message::OpenActualUrl("https://".into()))).await;
    assert!(invalid.iter().any(|message| matches!(message,
        Message::UrlOpened { result: Err(error), .. } if error.contains("valid HTTP(S) URL"))));
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn configured_prompt_archive_resolves_pick_and_calendar_off_thread_and_missing_art_is_recorded()
 {
    use publicator::{
        application::Application,
        config::{Config, PromptConfig, PromptGrammar},
        metadata::{Endpoints, MetadataClient},
        prompts::PromptMatch,
        ui::{App, Message, Tab, UiOptions},
    };
    use std::{process::Command, sync::Arc, time::Duration};

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
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
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
        assert!(
            Command::new("magick")
                .args(["-size", "4x3", "xc:white"])
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
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
    let store = Arc::new(f.store.clone());
    let application = Arc::new(Application::new(
        store,
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        ),
        || 0,
    ));
    let (mut app, init) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    dispatch_ui_task(&mut app, init).await;
    let candidates = [&exact_path, &nearest_path, &unknown_path]
        .into_iter()
        .map(|path| publicator::images::Candidate {
            path: path.to_path_buf(),
            sha512: publicator::images::file_sha512(path).unwrap(),
        })
        .collect();
    dispatch_ui_message(&mut app, Message::SearchFinished(Ok(candidates))).await;
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

    let calendar_messages = dispatch_ui_message(&mut app, Message::Navigate(Tab::Calendar)).await;
    assert!(calendar_messages.iter().any(|message| matches!(message,
        Message::CalendarPrepared(Ok(prepared))
            if prepared.artwork.get("missing.png").is_some_and(Option::is_none)
                && prepared.prompts.iter().any(|(path, prompt)| path == &exact_path
                    && matches!(prompt, PromptMatch::Exact(version) if version.text == "exact <prompt>\nline two"))
                && prepared.prompts.iter().any(|(path, prompt)| path.ends_with("missing.png")
                    && matches!(prompt, PromptMatch::Unknown(_)))
    )));
    assert!(app.calendar_prepared());
    assert!(app.calendar_art_path("missing.png").is_none());
    assert_eq!(std::fs::read_dir(f.path(".thumbs")).unwrap().count(), 1);
    assert!(matches!(
        app.prompt_match(&exact_path),
        Some(PromptMatch::Exact(_))
    ));
    let _: iced::Element<'_, Message> = publicator::ui::calendar::view(&app);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publication_jump_flushes_draft_then_reveals_target_phase_and_scroll_action() {
    use iced_runtime::{Action, futures::futures::StreamExt};
    use publicator::ui::{Message, Tab};

    let (mut app, store, _root) = scheduled_jump_app();
    let uuid = common::id(9);
    let before = app.current_tab();
    let _ = app.update(Message::Edited {
        uuid: uuid.clone(),
        field: publicator::ui::drafts::Field::Title,
        text: "Saved before jump".into(),
    });
    let task = app.update(Message::JumpToPublication(uuid.clone()));
    assert_eq!(
        app.current_tab(),
        before,
        "navigation waits for the dirty draft save"
    );
    let mut actions = iced_runtime::task::into_stream(task).expect("save task is returned");
    let completion = loop {
        match actions.next().await {
            Some(Action::Output(message @ Message::SaveFinished { result: Ok(()), .. })) => {
                break message;
            }
            Some(Action::Output(message @ Message::SaveFinished { result: Err(_), .. })) => {
                panic!("jump save failed: {message:?}")
            }
            Some(_) => {}
            None => panic!("draft save did not complete"),
        }
    };
    let followup = app.update(completion);
    let actions =
        iced_runtime::task::into_stream(followup).expect("target scroll operation is returned");
    assert!(
        actions
            .any(|action| async move { matches!(action, Action::Widget(_)) })
            .await
    );
    assert_eq!(app.current_tab(), Tab::Describe);
    assert_eq!(app.revealed_publication(), Some(uuid.as_str()));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(saved[0]["title"], "Saved before jump");
    let _: iced::Element<'_, Message> = publicator::ui::describe::view(&app);
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn publication_jump_maps_all_phases_to_the_next_action() {
    use iced_runtime::{Action, futures::futures::StreamExt};
    use publicator::ui::{Message, Tab};
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
        let _ = app.update(Message::LoadFinished(Ok(store.read().unwrap())));
        let task = app.update(Message::JumpToPublication(common::id(9)));
        assert_eq!(app.current_tab(), expected);
        let actions = iced_runtime::task::into_stream(task).expect("scroll action is returned");
        assert!(
            actions
                .any(|action| async move { matches!(action, Action::Widget(_)) })
                .await
        );
    }
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn failed_jump_flush_keeps_phase_and_draft_without_revealing_or_exiting() {
    use publicator::ui::{Message, Tab};
    let (mut app, store, _root) = scheduled_jump_app();
    let uuid = common::id(9);
    let before = std::fs::read(&store.paths().json).unwrap();
    let _ = app.update(Message::Edited {
        uuid: uuid.clone(),
        field: publicator::ui::drafts::Field::Title,
        text: "x".repeat(51),
    });
    let messages = dispatch_ui_message(&mut app, Message::JumpToPublication(uuid)).await;
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::SaveFinished { result: Err(_), .. })),
        "messages: {messages:?}"
    );
    assert_eq!(app.current_tab(), Tab::Pick);
    assert_eq!(app.revealed_publication(), None);
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    let retry = dispatch_ui_message(&mut app, Message::RetrySave).await;
    assert!(
        retry
            .iter()
            .any(|message| matches!(message, Message::SaveFinished { result: Err(_), .. }))
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
}
