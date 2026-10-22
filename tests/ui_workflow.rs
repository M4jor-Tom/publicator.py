#![cfg(feature = "desktop")]

mod common;

use iced::Element;
use publicator::{
    application::Application,
    config::{Config, Paths},
    metadata::{Endpoints, MetadataClient},
    ui::{App, Message, UiOptions, pick, schedule},
    workflow::{self, DateEdit},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

async fn run_task(task: iced::Task<Message>) -> Message {
    use iced_runtime::{Action, futures::futures::StreamExt};

    let mut actions = iced_runtime::task::into_stream(task).expect("dispatched task stream");
    while let Some(action) = actions.next().await {
        if let Action::Output(message) = action {
            return message;
        }
    }
    panic!("dispatched task produced no completion message");
}

fn app_with_rows(
    rows: serde_json::Value,
) -> (
    App,
    Arc<Application>,
    Arc<publicator::store::Store>,
    tempfile::TempDir,
) {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let config = Config {
        publicable: vec!["images".into()],
        ..Config::default()
    };
    let store = Arc::new(publicator::store::Store::new(paths, config).unwrap());
    std::fs::write(&store.paths().json, serde_json::to_vec(&rows).unwrap()).unwrap();
    let client = MetadataClient::new(
        reqwest::Client::new(),
        Endpoints::default(),
        "llmfit".into(),
    );
    let application = Arc::new(Application::new(store.clone(), client, || 1_798_761_600));
    let (mut app, _) = App::from_application(
        application.clone(),
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(300),
        },
    );
    let _ = app.update(Message::LoadFinished(Ok(
        serde_json::from_value(rows).unwrap()
    )));
    (app, application, store, root)
}

#[test]
fn both_native_views_are_constructible_from_loaded_application_state() {
    let (app, _, _, _) = app_with_rows(json!([]));
    let _: Element<'_, Message> = pick::view(&app);
    let _: Element<'_, Message> = schedule::view(&app);
}

#[test]
fn search_uses_default_limit_and_all_configured_paths() {
    let (app, _, _, _) = app_with_rows(json!([]));
    assert_eq!(app.search_limit(), 25);
    assert_eq!(app.search_paths(), vec!["images".to_owned()]);
}

#[test]
fn default_search_selects_every_configured_publicable_directory() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let config = Config {
        publicable: vec!["picked".into(), "other".into()],
        ..Config::default()
    };
    let store = Arc::new(publicator::store::Store::new(paths, config).unwrap());
    let application = Arc::new(Application::new(
        store,
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        ),
        || 1_798_761_600,
    ));
    let (app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(300),
        },
    );
    assert_eq!(app.search_limit(), 25);
    assert_eq!(app.search_paths(), ["picked", "other"]);
}

#[test]
fn schedule_view_does_not_rewrite_custom_manual_date_without_profile() {
    let mut publication = common::row(1, "PICKED");
    publication["apparitionTimestampIfDifferentThanSubmission"] =
        json!(common::epoch("2027-01-15T09:00:00Z"));
    publication["scheduleIsManual"] = json!(true);
    let (app, _, store, _root) = app_with_rows(json!([publication]));
    let before = std::fs::read(&store.paths().json).unwrap();

    let _: Element<'_, Message> = schedule::view(&app);

    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
    let saved: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(saved[0]["scheduleIsManual"], true);
    assert!(saved[0].get("scheduleProfile").is_none());
}

#[tokio::test]
async fn custom_date_ui_operation_persists_exact_wall_input_without_profile() {
    let (mut app, _application, store, _root) = app_with_rows(json!([common::row(1, "PICKED")]));
    let uuid = common::id(1);
    let _ = app.update(Message::CustomDate {
        uuid: uuid.clone(),
        value: "2027-01-01".into(),
    });
    let _ = app.update(Message::CustomTime {
        uuid: uuid.clone(),
        value: "19:30".into(),
    });
    let task = app.update(Message::SaveCustomDate(uuid.clone()));
    let completion = run_task(task).await;
    assert!(matches!(
        &completion,
        Message::WorkflowFinished { result: Ok(()), .. }
    ));
    let _ = app.update(completion);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(saved[0]["scheduleIsManual"], true);
    assert!(saved[0].get("scheduleProfile").is_none());
    assert_eq!(
        saved[0]["apparitionTimestampIfDifferentThanSubmission"],
        common::epoch("2027-01-01T18:30:00Z")
    );
}

#[tokio::test]
async fn suggested_date_ui_action_persists_the_automatic_schedule() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(root.path()), None).unwrap();
    let config = Config {
        publicable: vec!["images".into()],
        schedule: toml::from_str(
            r#"[[profiles]]
name = 'default'
day = 'tuesday'
hour = 20
per_slot = 1"#,
        )
        .unwrap(),
        ..Config::default()
    };
    let store = Arc::new(publicator::store::Store::new(paths, config).unwrap());
    let row = common::row(1, "PICKED");
    std::fs::write(
        &store.paths().json,
        serde_json::to_vec(&json!([row])).unwrap(),
    )
    .unwrap();
    let application = Arc::new(Application::new(
        store.clone(),
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        ),
        || 1_798_761_600,
    ));
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(300),
        },
    );
    let _ = app.update(Message::LoadFinished(Ok(store.read().unwrap())));
    let completion = run_task(app.update(Message::SuggestedDate(common::id(1)))).await;
    assert!(matches!(
        completion,
        Message::WorkflowFinished { result: Ok(()), .. }
    ));
    let _ = app.update(completion);
    let saved = store.read().unwrap();
    assert_eq!(saved[0]["scheduleProfile"], "default");
    assert_eq!(saved[0]["scheduleIsManual"], false);
    assert!(
        saved[0]["apparitionTimestampIfDifferentThanSubmission"]
            .as_i64()
            .is_some()
    );
}

#[tokio::test]
async fn pick_dispatch_persists_candidate_before_refreshing_phase() {
    let (mut app, _application, store, root) = app_with_rows(json!([]));
    let image_dir = root.path().join("images");
    std::fs::create_dir_all(&image_dir).unwrap();
    let image_fixture = common::Fixture::new();
    let path = image_fixture.image("source.png", 2, 2);
    let candidate_path = image_dir.join("candidate.png");
    std::fs::copy(path, &candidate_path).unwrap();
    let candidates = pick::search_candidates(&store, 25, &["images".into()]).unwrap();
    assert_eq!(candidates.len(), 1);
    let _ = app.update(Message::SearchFinished(Ok(candidates.clone())));
    let task = app.update(Message::Pick(candidates[0].clone()));
    let completion = run_task(task).await;
    assert!(matches!(&completion, Message::PickFinished(Ok(()))));
    let _ = app.update(completion);
    let rows = store.read().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "PICKED");
    assert_eq!(app.current_tab(), publicator::ui::Tab::Schedule);
}

#[test]
fn empty_search_replaces_pick_authorization_for_previous_candidates() {
    let (mut app, _application, store, root) = app_with_rows(json!([]));
    let dir = root.path().join("images");
    std::fs::create_dir_all(&dir).unwrap();
    let source_fixture = common::Fixture::new();
    let source = source_fixture.image("source.png", 2, 2);
    let path = dir.join("candidate.png");
    std::fs::copy(source, &path).unwrap();
    let old = pick::search_candidates(&store, 25, &["images".into()])
        .unwrap()
        .remove(0);
    let _ = app.update(Message::SearchFinished(Ok(vec![old.clone()])));
    let _ = app.update(Message::SearchFinished(Ok(Vec::new())));
    let _ = app.update(Message::Pick(old));
    assert!(store.read().unwrap().is_empty());
    assert!(path.exists());
}

#[test]
fn stale_candidate_authorization_cannot_delete_file() {
    let (_app, application, store, root) = app_with_rows(json!([]));
    let dir = root.path().join("images");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("candidate.png");
    let image_fixture = common::Fixture::new();
    let image = image_fixture.image("source.png", 2, 2);
    std::fs::copy(image, &path).unwrap();
    let candidate = pick::search_candidates(&store, 25, &["images".into()])
        .unwrap()
        .remove(0);
    assert!(pick::delete_candidate(&application, &[], &candidate).is_err());
    assert!(path.exists());
    std::fs::write(&path, b"edited after picking").unwrap();
    let edited = std::fs::read(&path).unwrap();
    assert!(
        pick::delete_candidate(&application, std::slice::from_ref(&candidate), &candidate).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), edited);
}

#[test]
fn confirmed_deletion_uses_current_candidate_authorization() {
    let (_app, application, store, root) = app_with_rows(json!([]));
    let dir = root.path().join("images");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = common::Fixture::new();
    let image = fixture.image("source.png", 2, 2);
    let path = dir.join("candidate.png");
    std::fs::copy(image, &path).unwrap();
    let authorized = pick::search_candidates(&store, 25, &["images".into()]).unwrap();
    assert_eq!(authorized.len(), 1);
    pick::delete_candidate(&application, &authorized, &authorized[0]).unwrap();
    assert!(!path.exists());
}

#[test]
fn unavailable_suggestion_cannot_be_approved_and_frozen_schedule_cannot_be_edited() {
    let (_app, _application, store, _root) = app_with_rows(json!([
        common::row(1, "PICKED"),
        common::row(2, "SCHEDULED")
    ]));
    let before = std::fs::read(&store.paths().json).unwrap();
    assert!(
        workflow::approve_date(
            &store,
            &common::id(1),
            common::epoch("2026-12-01T00:00:00Z")
        )
        .is_err()
    );
    assert!(
        workflow::edit_date(
            &store,
            &common::id(2),
            DateEdit::Custom("2027-01-01T19:30".into()),
            common::epoch("2026-12-01T00:00:00Z")
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&store.paths().json).unwrap(), before);
}

#[test]
fn zero_search_limit_returns_no_candidates() {
    let (_app, _, store, root) = app_with_rows(json!([]));
    let configured = store.config().publicable.clone();
    for path in &configured {
        std::fs::create_dir_all(root.path().join(path)).unwrap();
    }
    assert!(
        pick::search_candidates(&store, 0, &configured)
            .unwrap()
            .is_empty()
    );
}
