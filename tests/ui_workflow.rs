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
fn custom_date_ui_operation_persists_exact_wall_input_without_profile() {
    let (mut app, application, store, _root) = app_with_rows(json!([common::row(1, "PICKED")]));
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
    assert!(
        task.units() > 0,
        "Custom-date message must dispatch an async save"
    );
    schedule::edit_date(
        &application,
        &uuid,
        DateEdit::Custom("2027-01-01T19:30".into()),
        common::epoch("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&store.paths().json).unwrap()).unwrap();
    assert_eq!(saved[0]["scheduleIsManual"], true);
    assert!(saved[0].get("scheduleProfile").is_none());
    assert_eq!(
        saved[0]["apparitionTimestampIfDifferentThanSubmission"],
        common::epoch("2027-01-01T18:30:00Z")
    );
    let _ = app.update(Message::WorkflowFinished {
        message: "Saved schedule date".into(),
        result: Ok(()),
    });
}

#[test]
fn pick_dispatch_persists_candidate_before_refreshing_phase() {
    let (mut app, application, store, root) = app_with_rows(json!([]));
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
    assert!(
        task.units() > 0,
        "authorized Pick message must dispatch its operation"
    );
    let saved_uuid = pick::pick_candidate(
        &application,
        &candidates[0],
        common::epoch("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    let rows = store.read().unwrap();
    assert_eq!(rows[0]["uuid"], saved_uuid);
    assert_eq!(rows[0]["state"], "PICKED");
    let _ = app.update(Message::PickFinished(Ok(())));
    assert_eq!(app.current_tab(), publicator::ui::Tab::Schedule);
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
