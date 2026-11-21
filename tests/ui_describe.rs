#![cfg(feature = "desktop")]

mod common;
use common::{
    Fixture,
    http::{Reply, completion},
    id, row, thumbs, with_markers,
};
use iced::Element;
use publicator::{
    application::Application,
    application::BatchEvent,
    metadata::{Endpoints, MetadataClient, Source, VISION_MODELS, available_sources},
    ui::{App, GenerationState, Message, Tab, UiOptions, describe},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

/// The placeholder sensor's report that the preview `key` is (`shown`) or is not on screen.
fn shown(key: impl Into<String>, shown: bool) -> Message {
    Message::PreviewShown {
        key: key.into(),
        shown,
    }
}

fn fixture_app(f: &Fixture, endpoint: String) -> (App, Arc<Application>) {
    fixture_app_with_model(f, endpoint, None)
}

fn fixture_app_with_model(
    f: &Fixture,
    endpoint: String,
    openrouter_model: Option<publicator::metadata::ModelOption>,
) -> (App, Arc<Application>) {
    let store = Arc::new(f.store.clone());
    let client = MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: endpoint,
            ollama: "http://127.0.0.1:1".into(),
        },
        "llmfit".into(),
    )
    .with_openrouter_key("fixture");
    let application = Arc::new(Application::new(store, client));
    let (mut app, _) = App::from_application(
        application.clone(),
        UiOptions {
            openrouter_model,
            ai_timeout: Duration::from_secs(5),
        },
    );
    let _ = app.update(Message::LoadFinished(0, Ok(f.store.read().unwrap().into())));
    (app, application)
}

async fn select_local(app: &mut App) {
    let selected = common::ui::first_output(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
}

fn edit_manual_metadata(app: &mut App) {
    for (field, text) in [
        (publicator::ui::drafts::Field::Title, "Manual title"),
        (
            publicator::ui::drafts::Field::Description,
            "Manual description",
        ),
    ] {
        let _ = app.update(Message::Edited {
            uuid: id(1),
            field,
            text: text.into(),
        });
    }
}

fn marker_row(n: u64) -> serde_json::Value {
    with_markers(row(n, "SCHEDULED"))
}

fn fake_llmfit(f: &Fixture, level: &str, notes: &[&str]) -> std::path::PathBuf {
    let shell = std::process::Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    let shell = String::from_utf8(shell.stdout).unwrap();
    let models = VISION_MODELS
        .iter()
        .map(|model| json!({"name":model.llmfit_name,"fit_level":level,"notes":notes}))
        .collect::<Vec<_>>();
    let path = f.path("llmfit");
    common::executable::write_executable(
        &path,
        &format!(
            "#!{}\ncat <<'EOF'\n{}\nEOF\n",
            shell.trim_end(),
            json!({"models":models})
        ),
    );
    path
}

fn failed_llmfit(f: &Fixture) -> std::path::PathBuf {
    let shell = std::process::Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    let path = f.path("llmfit-failed");
    common::executable::write_executable(
        &path,
        &format!(
            "#!{}\nprintf 'fit unavailable' >&2\nexit 1\n",
            String::from_utf8(shell.stdout).unwrap().trim_end()
        ),
    );
    path
}

fn local_fixture_app(
    f: &Fixture,
    llmfit: std::path::PathBuf,
    ollama: String,
) -> (App, Arc<Application>) {
    let store = Arc::new(f.store.clone());
    let client = MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: "http://127.0.0.1:1".into(),
            ollama,
        },
        llmfit,
    );
    let application = Arc::new(Application::new(store, client));
    let (mut app, _) = App::from_application(
        application.clone(),
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(5),
        },
    );
    let _ = app.update(Message::LoadFinished(0, Ok(f.store.read().unwrap().into())));
    (app, application)
}

fn good_local_status_app(f: &Fixture, installed: bool) -> (App, common::http::Server) {
    let models = if installed {
        json!([{"name": VISION_MODELS[0].ollama_name}])
    } else {
        json!([])
    };
    let server = common::http::Server::start(
        (0..2)
            .map(|_| Reply::new("/api/tags", 200, json!({"models":models})))
            .collect(),
    );
    let (app, _) = local_fixture_app(
        f,
        fake_llmfit(f, "Good", &["Fits host"]),
        server.url().into(),
    );
    (app, server)
}

#[test]
fn source_options_do_not_include_a_third_provider() {
    assert_eq!(available_sources(), &[Source::OpenRouter, Source::Local]);
}

#[tokio::test]
async fn local_generation_checks_and_exposes_fit_before_install_consent() {
    let f = Fixture::new();
    let llmfit = f.path("llmfit");
    let shell = std::process::Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    let shell = String::from_utf8(shell.stdout).unwrap();
    common::executable::write_executable(
        &llmfit,
        &format!(
            "#!{}\ncat <<'EOF'\n{}\nEOF\n",
            shell.trim_end(),
            json!({"models":[{"name":VISION_MODELS[0].llmfit_name,"fit_level":"Too Tight","notes":["Needs 40 GB RAM"]}]})
        ),
    );
    assert!(
        std::process::Command::new(&llmfit)
            .output()
            .unwrap()
            .status
            .success()
    );
    let store = Arc::new(f.store.clone());
    let client = MetadataClient::new(reqwest::Client::new(), Endpoints::default(), llmfit);
    let application = Arc::new(Application::new(store, client));
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(5),
        },
    );
    let _ = app.update(Message::LoadFinished(0, Ok(f.store.read().unwrap().into())));
    let _ = app.update(Message::SelectSource(Source::Local));
    let task = app.update(Message::GenerateOne(id(1)));
    let fit_result = common::ui::first_output(task).await;
    assert!(matches!(fit_result, Message::LocalFitFinished { .. }));
    let _ = app.update(fit_result);
    let status = app.local_fit_status().unwrap();
    assert_eq!(status.fit.reason, "Needs 40 GB RAM");
    assert!(!app.local_install_consent_pending());
}

#[tokio::test]
async fn installed_good_local_model_generates_without_pull_or_install_modal() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1)]));
    let model_name = VISION_MODELS[0].ollama_name.clone();
    let tags = json!({"models":[{"name":model_name}]});
    let server = common::http::Server::start(vec![
        Reply::new("/api/tags", 200, tags.clone()),
        Reply::new("/api/tags", 200, tags.clone()),
        Reply::new("/api/tags", 200, tags.clone()),
        Reply::new("/api/tags", 200, tags),
        Reply::new(
            "/api/chat",
            200,
            json!({"message":{"content":json!({"title":"Installed model","description":"Generated locally."}).to_string()}}),
        ),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits in memory"]),
        server.url().into(),
    );
    select_local(&mut app).await;
    let status = app.local_fit_status().unwrap();
    assert_eq!(status.fit.reason, "Fits in memory");
    assert!(status.available && status.installed);
    let checked = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(checked, Message::LocalFitFinished { .. }));
    assert!(!app.local_install_consent_pending());
    let generation = app.update(checked);
    let completed = common::ui::first_output(generation).await;
    let _ = app.update(completed);
    let calls = server.requests();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["path"] == "/api/tags")
            .count(),
        4
    );
    assert!(!calls.iter().any(|call| call["path"] == "/api/pull"));
    assert_eq!(calls.last().unwrap()["path"], "/api/chat");
    assert_eq!(f.read()[0]["title"], "Installed model");
    assert_eq!(f.read()[0]["state"], "DESCRIBED");
}

#[tokio::test]
async fn missing_good_model_requires_confirmation_and_only_then_pulls() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let tags = json!({"models":[]});
    let server = common::http::Server::start(vec![
        Reply::new("/api/tags", 200, tags.clone()),
        Reply::new("/api/tags", 200, tags.clone()),
        Reply::new("/api/tags", 200, tags.clone()),
        Reply::new("/api/tags", 200, tags),
        Reply::new("/api/pull", 200, json!({"status":"success"})),
        Reply::new(
            "/api/chat",
            200,
            json!({"message":{"content":json!({"title":"Installed after consent","description":"Generated locally."}).to_string()}}),
        ),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    select_local(&mut app).await;
    let checked = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let _ = app.update(checked);
    assert!(app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    assert!(
        !server
            .requests()
            .iter()
            .any(|call| call["path"] == "/api/pull")
    );
    let generation = app.update(Message::ConfirmLocalInstall);
    let _ = app.update(common::ui::first_output(generation).await);
    let calls = server.requests();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["path"] == "/api/tags")
            .count(),
        4
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["path"] == "/api/pull")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["path"] == "/api/chat")
            .count(),
        1
    );
    assert_eq!(f.read()[0]["title"], "Installed after consent");
    assert_eq!(f.read()[0]["state"], "DESCRIBED");
}

#[tokio::test]
async fn dirty_local_batch_requires_consent_and_refusal_never_pulls() {
    let f = Fixture::new();
    f.write(json!([marker_row(1), marker_row(2)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let server =
        common::http::Server::start(vec![Reply::new("/api/tags", 200, json!({"models": []}))]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let _pending_status = app.update(Message::SelectSource(Source::Local));
    edit_manual_metadata(&mut app);

    let fit_result = common::ui::first_output(app.update(Message::GenerateMarked)).await;
    assert!(matches!(fit_result, Message::LocalFitFinished { .. }));
    let _ = app.update(fit_result);
    let status = app.local_fit_status().unwrap();
    assert!(status.fit.fit && status.available && !status.installed);
    assert!(app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|call| call["path"] == "/api/tags")
            .count(),
        1
    );
    assert!(
        !server
            .requests()
            .iter()
            .any(|call| call["path"] == "/api/pull")
    );

    let _ = app.update(Message::CancelLocalInstall);
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    assert!(
        !server
            .requests()
            .iter()
            .any(|call| call["path"] == "/api/pull")
    );
    assert!(
        !server
            .requests()
            .iter()
            .any(|call| call["path"] == "/api/chat")
    );
}

#[tokio::test]
async fn installed_local_batch_keeps_identity_through_dirty_flush_and_admission() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), marker_row(2)]));
    let (mut app, server) = good_local_status_app(&f, true);
    select_local(&mut app).await;

    // The Good/installed fit result immediately continues without consent. Add
    // a draft while status is pending so the batch identity must survive save.
    let fit_task = app.update(Message::GenerateMarked);
    let status = common::ui::first_output(fit_task).await;
    edit_manual_metadata(&mut app);
    let flush = app.update(status);
    let saved = common::ui::first_output(flush).await;
    let generation = app.update(saved);

    // The generation Task has not been polled/admitted yet. Its expected batch
    // must still be the original two UUIDs, despite the dirty draft replacing
    // one marker during the required save.
    assert_eq!(f.read()[0]["title"], "Manual title");
    let after_flush = std::fs::read(&f.store.paths().json).unwrap();
    let finished = common::ui::first_output(generation).await;
    assert!(
        matches!(
            finished,
            Message::GenerationFinished {
                result: Err(ref error),
                ..
            } if error.contains("marked batch changed")
        ),
        "unexpected completion: {finished:?}"
    );
    let _ = app.update(finished);
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), after_flush);
    assert_eq!(f.read()[1]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert!(!app.local_install_consent_pending());
    let calls = server.requests();
    assert!(!calls.iter().any(|call| call["path"] == "/api/pull"));
    assert!(!calls.iter().any(|call| call["path"] == "/api/chat"));
}

#[tokio::test]
async fn installed_local_batch_rejects_membership_changed_before_admission() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), marker_row(2)]));
    let (mut app, server) = good_local_status_app(&f, true);
    select_local(&mut app).await;
    let status = common::ui::first_output(app.update(Message::GenerateMarked)).await;
    let generation = app.update(status);
    f.store
        .mutate(|rows| {
            rows[0]["title"] = json!("Manual title");
            rows[0]["description"] = json!("Manual description");
            Ok(())
        })
        .unwrap();
    let saved = std::fs::read(&f.store.paths().json).unwrap();
    let finished = common::ui::first_output(generation).await;
    assert!(matches!(
        finished,
        Message::GenerationFinished {
            result: Err(ref error),
            ..
        } if error.contains("marked batch changed")
    ));
    let _ = app.update(finished);
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), saved);
    assert!(
        !server
            .requests()
            .iter()
            .any(|call| call["path"] == "/api/chat")
    );
    assert!(
        !server
            .requests()
            .iter()
            .any(|call| call["path"] == "/api/pull")
    );
}

#[tokio::test]
async fn batch_local_install_consent_rejects_a_changed_eligible_set() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), marker_row(2)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let tags = json!({"models": []});
    let server = common::http::Server::start(vec![Reply::new("/api/tags", 200, tags.clone())]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let _pending_status = app.update(Message::SelectSource(Source::Local));
    edit_manual_metadata(&mut app);
    let checked = common::ui::first_output(app.update(Message::GenerateMarked)).await;
    let _ = app.update(checked);
    assert!(app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);

    // The consent continuation still carries the original set through the
    // dirty-draft flush and checks it at actual Application admission.
    let confirm = app.update(Message::ConfirmLocalInstall);
    f.store
        .mutate(|rows| {
            rows[0]["title"] = json!("Manual title");
            rows[0]["description"] = json!("Manual description");
            Ok(())
        })
        .unwrap();
    let saved = std::fs::read(&f.store.paths().json).unwrap();
    assert_ne!(saved, before);
    assert_eq!(f.read()[0]["title"], "Manual title");
    let saved_message = common::ui::first_output(confirm).await;
    assert!(matches!(
        saved_message,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    let generation = app.update(saved_message);
    let finished = common::ui::first_output(generation).await;
    assert!(matches!(
        finished,
        Message::GenerationFinished {
            result: Err(ref error),
            ..
        } if error.contains("marked batch changed")
    ));
    let _ = app.update(finished);
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), saved);
    assert_eq!(f.read()[0]["title"], "Manual title");
    let calls = server.requests();
    assert!(!calls.iter().any(|call| call["path"] == "/api/pull"));
    assert!(!calls.iter().any(|call| call["path"] == "/api/chat"));
}

#[tokio::test]
async fn cancel_missing_model_consent_keeps_saved_bytes_and_never_pulls_or_chats() {
    let f = Fixture::new();
    f.write(json!([marker_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let (mut app, server) = good_local_status_app(&f, false);
    select_local(&mut app).await;
    let checked = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let _ = app.update(checked);
    assert!(app.local_install_consent_pending());
    let _ = app.update(Message::CancelLocalInstall);
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    let calls = server.requests();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call["path"] == "/api/tags"));
}

#[tokio::test]
async fn older_target_fit_result_cannot_authorize_newer_target_consent() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), f.marked_row(2)]));
    let empty = json!({"models":[]});
    let mut replies = (0..5)
        .map(|_| Reply::new("/api/tags", 200, empty.clone()))
        .collect::<Vec<_>>();
    replies.push(Reply::new("/api/pull", 200, json!({"status":"success"})));
    replies.push(Reply::new("/api/chat", 200, json!({"message":{"content":json!({"title":"Only second","description":"Consent bound to target."}).to_string()}})));
    let server = common::http::Server::start(replies);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    select_local(&mut app).await;
    let first = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let second = common::ui::first_output(app.update(Message::GenerateOne(id(2)))).await;
    let _ = app.update(first);
    assert!(!app.local_install_consent_pending());
    let _ = app.update(second);
    assert!(app.local_install_consent_pending());
    let generation = app.update(Message::ConfirmLocalInstall);
    let _ = app.update(common::ui::first_output(generation).await);
    assert_eq!(f.read()[0]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(f.read()[1]["title"], "Only second");
    assert_eq!(f.read()[1]["state"], "DESCRIBED");
}

#[tokio::test]
async fn poor_unknown_and_failed_fit_results_never_prompt_or_mutate() {
    for (level, expected_reason) in [
        ("Too Tight", "Too tight for host"),
        ("Mystery", "llmfit fit_level: Mystery"),
    ] {
        let f = Fixture::new();
        f.write(json!([marker_row(1)]));
        let before = std::fs::read(&f.store.paths().json).unwrap();
        let server = common::http::Server::start(vec![]);
        let (mut app, _) = local_fixture_app(
            &f,
            fake_llmfit(
                &f,
                level,
                if level == "Mystery" {
                    &[]
                } else {
                    &["Too tight for host"]
                },
            ),
            server.url().into(),
        );
        let selected =
            common::ui::first_output(app.update(Message::SelectSource(Source::Local))).await;
        let _ = app.update(selected);
        let checked = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
        let _ = app.update(checked);
        assert_eq!(app.local_fit_status().unwrap().fit.reason, expected_reason);
        assert!(!app.local_install_consent_pending());
        assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
        assert!(server.requests().is_empty());
    }

    let f = Fixture::new();
    f.write(json!([marker_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let server = common::http::Server::start(vec![]);
    let (mut app, _) = local_fixture_app(&f, failed_llmfit(&f), server.url().into());
    select_local(&mut app).await;
    let checked = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let _ = app.update(checked);
    assert!(
        app.local_fit_status()
            .unwrap()
            .fit
            .reason
            .contains("fit unavailable")
    );
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn good_fit_with_unavailable_ollama_never_prompts_or_mutates() {
    let f = Fixture::new();
    f.write(json!([marker_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        "http://127.0.0.1:1".into(),
    );
    select_local(&mut app).await;
    let checked = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let _ = app.update(checked);
    let status = app.local_fit_status().unwrap();
    assert!(status.fit.fit);
    assert!(!status.available);
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
}

#[tokio::test]
async fn cancelled_or_source_stale_fit_result_cannot_reopen_install_consent() {
    let f = Fixture::new();
    f.write(json!([marker_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let (mut app, server) = good_local_status_app(&f, false);
    select_local(&mut app).await;
    let consent_result = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let stale_copy = consent_result.clone();
    let _ = app.update(consent_result);
    assert!(app.local_install_consent_pending());
    let _ = app.update(Message::CancelLocalInstall);
    let _ = app.update(stale_copy);
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    assert!(
        server
            .requests()
            .iter()
            .all(|call| call["path"] == "/api/tags")
    );
}

#[tokio::test]
async fn model_change_invalidates_in_flight_local_fit_before_consent() {
    let f = Fixture::new();
    f.write(json!([marker_row(1)]));
    let (mut app, _server) = good_local_status_app(&f, false);
    select_local(&mut app).await;
    let stale = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let current =
        common::ui::first_output(app.update(Message::SelectModel(VISION_MODELS[1].clone()))).await;
    let _ = app.update(stale);
    assert!(!app.local_install_consent_pending());
    let _ = app.update(current);
    assert!(!app.local_install_consent_pending());
}

#[tokio::test]
async fn source_change_invalidates_in_flight_local_fit_before_consent() {
    let f = Fixture::new();
    f.write(json!([marker_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let (mut app, _server) = good_local_status_app(&f, false);
    select_local(&mut app).await;
    let stale = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    let _ = app.update(Message::SelectSource(Source::OpenRouter));
    let _ = app.update(stale);
    assert!(!app.local_install_consent_pending());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
}

#[test]
fn selecting_local_source_does_not_itself_confirm_install() {
    let f = Fixture::new();
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let _ = app.update(Message::SelectSource(Source::Local));
    let task = app.update(Message::GenerateOne(id(1)));
    assert!(iced_runtime::task::into_stream(task).is_some());
    assert!(!app.local_install_consent_pending());
    let _ = app.update(Message::CancelLocalInstall);
    assert!(!app.local_install_consent_pending());
}

#[tokio::test]
async fn describe_dispatch_holds_only_target_until_provider_result_is_persisted() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), row(2, "SCHEDULED")]));
    let (server, started_rx, release_tx) =
        common::http::Server::start_paused(completion("Generated"));
    let (mut app, _) = fixture_app(&f, server.url().to_owned());
    let task = tokio::spawn(common::ui::first_output(
        app.update(Message::GenerateOne(id(1))),
    ));
    tokio::task::spawn_blocking(move || started_rx.recv_timeout(Duration::from_secs(3)).unwrap())
        .await
        .unwrap();
    let _ = app.update(Message::Edited {
        uuid: id(1),
        field: publicator::ui::drafts::Field::Title,
        text: "blocked".into(),
    });
    let _ = app.update(Message::Edited {
        uuid: id(2),
        field: publicator::ui::drafts::Field::Title,
        text: "unrelated".into(),
    });
    let save_other = common::ui::first_output(app.update(Message::SaveDraft(id(2)))).await;
    assert!(matches!(
        save_other,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    let _ = app.update(save_other);
    assert_eq!(f.read()[1]["title"], "unrelated");
    release_tx.send(()).unwrap();
    let completion = task.await.unwrap();
    let _ = app.update(completion);
    server.finish();
    let saved = f.read();
    assert_eq!(saved[0]["title"], "Generated");
    assert_eq!(saved[0]["state"], "DESCRIBED");
    assert_eq!(saved[1]["title"], "unrelated");
    assert_eq!(saved[1]["state"], "SCHEDULED");
}

/// What loads follows the placeholders actually on screen, not estimated row heights:
/// the last card of a long list loads as soon as it is shown, and nothing else does.
#[tokio::test]
async fn pick_previews_load_exactly_the_shown_placeholders_anywhere_in_the_list() {
    let config = publicator::config::Config {
        publicable: vec!["images".into()],
        ..publicator::config::Config::default()
    };
    let f = Fixture::with_config(config);
    let dir = f.path("images");
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..10 {
        let image = f.image(&format!("source-{n}.png"), 3 + n, 3);
        std::fs::copy(image, dir.join(format!("{n}.png"))).unwrap();
    }
    let (mut app, _) = App::from_application(
        Arc::new(common::offline_app(Arc::new(f.store.clone()))),
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    let _ = app.update(Message::LoadFinished(0, Ok(f.store.read().unwrap().into())));
    let candidates =
        publicator::ui::pick::search_candidates(&f.store, 25, &["images".into()]).unwrap();
    assert_eq!(candidates.len(), 10);
    let task = app.update(Message::SearchFinished {
        request_id: 0,
        result: Ok(candidates.clone()),
    });
    common::ui::settle(&mut app, task).await;
    assert!(thumbs(&f).is_empty(), "nothing is on screen yet");
    let thumb = |candidate: &publicator::images::Candidate| {
        publicator::images::thumb_name(&candidate.path, &candidate.sha512).unwrap()
    };
    let key = |candidate: &publicator::images::Candidate| {
        publicator::ui::pick::preview_key(&candidate.path).into_owned()
    };
    let (last, hidden) = (&candidates[9], &candidates[4]);
    let _ = app.update(shown(key(hidden), true));
    let _ = app.update(shown(key(hidden), false));
    let task = app.update(shown(key(last), true));
    common::ui::settle(&mut app, task).await;
    assert_eq!(thumbs(&f), [thumb(last)]);
    assert!(
        common::ui::run(app.update(shown(key(last), true)))
            .await
            .messages
            .is_empty(),
        "a loaded preview is not prepared again"
    );
}

#[tokio::test]
async fn row_previews_load_the_shown_rows_of_the_active_tab() {
    let f = Fixture::new();
    let mut rows = Vec::new();
    let mut digests = Vec::new();
    for n in 1..=10 {
        let image = f.image(&format!("{n}.png"), 3 + n as u32, 3);
        let mut publication = row(n, "SCHEDULED");
        let digest = publicator::images::file_sha512(&image).unwrap();
        publication["files"][0]["sha512sum"] = json!(digest);
        digests.push(publicator::images::thumb_name(&image, &digest).unwrap());
        rows.push(publication);
    }
    f.write(json!(rows));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let task = app.update(Message::Navigate(Tab::Describe));
    common::ui::settle(&mut app, task).await;
    assert!(thumbs(&f).is_empty(), "nothing is on screen yet");
    let task = app.update(shown(id(10), true));
    common::ui::settle(&mut app, task).await;
    assert_eq!(thumbs(&f), [digests[9].clone()]);
    assert!(
        common::ui::run(app.update(shown(id(10), true)))
            .await
            .messages
            .is_empty()
    );
}

#[tokio::test]
async fn row_preview_loads_for_a_hash_recorded_as_file_sha512sum() {
    let f = Fixture::new();
    let image = f.image("1.png", 4, 3);
    let digest = publicator::images::file_sha512(&image).unwrap();
    let mut publication = row(1, "SCHEDULED");
    publication["files"][0] = json!({"basename": "1.png", "fileSha512sum": digest});
    f.write(json!([publication]));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let task = app.update(Message::Navigate(Tab::Describe));
    common::ui::settle(&mut app, task).await;
    let task = app.update(shown(id(1), true));
    common::ui::settle(&mut app, task).await;
    assert_eq!(
        thumbs(&f),
        [publicator::images::thumb_name(&image, &digest).unwrap()]
    );
}

#[tokio::test]
async fn row_without_recorded_hash_reports_an_error_not_an_unverified_preview() {
    let f = Fixture::new();
    f.image("1.png", 4, 3);
    let mut publication = row(1, "SCHEDULED");
    publication["files"][0] = json!({"basename": "1.png"});
    f.write(json!([publication]));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let task = app.update(Message::Navigate(Tab::Describe));
    common::ui::settle(&mut app, task).await;
    let effects = common::ui::send(&mut app, shown(id(1), true)).await;
    assert!(
        effects.any(|message| matches!(message,
            Message::RowPreviewFinished { uuid, result: Err(error) }
                if *uuid == id(1) && error.contains("artwork hash is missing"))),
        "{:?}",
        effects.messages
    );
    assert!(thumbs(&f).is_empty(), "no preview of unverified bytes");
}

#[tokio::test]
async fn describe_rows_sharing_a_basename_each_preview_their_recorded_file() {
    let f = Fixture::new();
    let (rows, _, mut expected) = f.namesake_rows("SCHEDULED");
    expected.sort();
    f.write(json!(rows));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let task = app.update(Message::Navigate(Tab::Describe));
    common::ui::settle(&mut app, task).await;
    for n in [1, 2] {
        let effects = common::ui::send(&mut app, shown(id(n), true)).await;
        assert!(
            effects.any(|message| matches!(message,
                Message::RowPreviewFinished { uuid, result: Ok(_) } if *uuid == id(n))),
            "{:?}",
            effects.messages
        );
    }
    assert_eq!(thumbs(&f), expected);
}

#[tokio::test]
async fn delayed_title_save_preserves_newer_same_and_other_field_edits() {
    use fs2::FileExt;

    async fn run_case(
        newer_field: publicator::ui::drafts::Field,
        newer_text: &str,
        expected_title: &str,
        expected_description: &str,
    ) {
        let f = Fixture::new();
        let mut publication = row(1, "SCHEDULED");
        publication["title"] = json!("Stored title");
        publication["description"] = json!("Stored description");
        f.write(json!([publication]));
        let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
        let _ = app.update(Message::Edited {
            uuid: id(1),
            field: publicator::ui::drafts::Field::Title,
            text: "First edit".into(),
        });

        let mut lock_path = f.store.paths().json.clone();
        lock_path.as_mut_os_string().push(".lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .unwrap();
        lock.lock_exclusive().unwrap();
        let task = app.update(Message::SaveDraft(id(1)));
        let first_save = tokio::spawn(common::ui::first_output(task));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!first_save.is_finished());
        let _ = app.update(Message::Edited {
            uuid: id(1),
            field: newer_field,
            text: newer_text.into(),
        });
        lock.unlock().unwrap();

        let first = first_save.await.unwrap();
        assert!(matches!(
            first,
            Message::SaveFinished { result: Ok(()), .. }
        ));
        let intermediate = f.read();
        assert_eq!(intermediate[0]["title"], "First edit");
        assert_eq!(intermediate[0]["description"], "Stored description");
        let second_task = app.update(first);
        let second = common::ui::first_output(second_task).await;
        assert!(matches!(
            second,
            Message::SaveFinished { result: Ok(()), .. }
        ));
        let _ = app.update(second);
        let saved = f.read();
        assert_eq!(saved[0]["title"], expected_title);
        assert_eq!(saved[0]["description"], expected_description);
    }

    run_case(
        publicator::ui::drafts::Field::Description,
        "Newer description",
        "First edit",
        "Newer description",
    )
    .await;
    run_case(
        publicator::ui::drafts::Field::Title,
        "Newer title",
        "Newer title",
        "Stored description",
    )
    .await;
}

#[tokio::test]
async fn repeated_explicit_save_lands_the_newest_revision_last() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED"), row(2, "SCHEDULED")]));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let edit = |app: &mut App, n, text: &str| {
        let _ = app.update(Message::Edited {
            uuid: id(n),
            field: publicator::ui::drafts::Field::Title,
            text: text.into(),
        });
    };
    edit(&mut app, 1, "Revision A");
    let held_a = app.update(Message::SaveDraft(id(1)));
    edit(&mut app, 1, "Revision B");
    edit(&mut app, 2, "Unrelated");
    let unrelated = common::ui::send(&mut app, Message::SaveDraft(id(2))).await;
    assert!(
        unrelated.any(|message| matches!(message, Message::SaveFinished { result: Ok(()), .. })),
        "an unrelated record stays saveable while A is in flight: {unrelated:?}"
    );
    assert_eq!(f.read()[1]["title"], "Unrelated");

    // A second explicit save of the same draft would land before the held A.
    common::ui::send(&mut app, Message::SaveDraft(id(1))).await;
    common::ui::settle(&mut app, held_a).await;
    assert_eq!(f.read()[0]["title"], "Revision B");
    let refresh = common::ui::send(&mut app, Message::Refresh).await;
    assert!(
        refresh.any(|message| matches!(message, Message::LoadFinished(_, Ok(_)))),
        "the newest revision is clean, so Refresh is admitted: {refresh:?}"
    );
}

#[tokio::test]
async fn dirty_draft_is_saved_before_single_generation_starts() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1)]));
    let server = common::http::Server::start(vec![completion("Generated")]);
    let (mut app, _) = fixture_app(&f, server.url().to_owned());
    let _ = app.update(Message::Edited {
        uuid: id(1),
        field: publicator::ui::drafts::Field::Title,
        text: "Manual draft".into(),
    });
    let saved = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(
        saved,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    assert_eq!(f.read()[0]["title"], "Manual draft");
    let generation = app.update(saved);
    let completed = common::ui::first_output(generation).await;
    let _ = app.update(completed);
    server.finish();
    assert_eq!(f.read()[0]["title"], "Generated");
}

#[tokio::test]
async fn failed_single_generation_can_be_retried_from_describe() {
    use common::http::{Reply, Server};

    let f = Fixture::new();
    f.write(json!([f.marked_row(1)]));
    let server = Server::start(vec![
        Reply::new(
            "/chat/completions",
            500,
            json!({"error":{"message":"fixture failure"}}),
        ),
        Reply::new("/models", 200, json!({"data":[]})),
        completion("Generated"),
    ]);
    let (mut app, _) = fixture_app(&f, server.url().to_owned());

    let started = app.update(Message::GenerateOne(id(1)));
    assert_eq!(
        app.generation_state(&id(1)),
        Some(&GenerationState::Generating)
    );
    let failed = common::ui::first_output(started).await;
    assert!(matches!(
        failed,
        Message::GenerationFinished { result: Err(_), .. }
    ));
    let _ = app.update(failed);
    assert!(matches!(
        app.generation_state(&id(1)),
        Some(GenerationState::Failed(_))
    ));
    assert_eq!(f.read()[0]["state"], "SCHEDULED");
    assert_eq!(f.read()[0]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(f.read()[0]["description"], "[[PUBLICATOR:TODO_GENERATE]]");

    let retried = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(
        retried,
        Message::GenerationFinished { result: Ok(_), .. }
    ));
    let _ = app.update(retried);
    assert_eq!(
        app.generation_state(&id(1)),
        Some(&GenerationState::Generated)
    );
    assert_eq!(f.read()[0]["title"], "Generated");
    assert_eq!(
        f.read()[0]["description"],
        "A complete generated description."
    );
    assert_eq!(f.read()[0]["state"], "DESCRIBED");
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request["path"] == "/chat/completions")
            .count(),
        2
    );
}

#[tokio::test]
async fn batch_dispatch_uses_reverse_order_and_persists_each_record() {
    use common::http::Server;
    let f = Fixture::new();
    let rows: Vec<_> = (1..=3).map(|n| f.marked_row(n)).collect();
    f.write(json!(rows));
    let replies = [3, 2, 1]
        .into_iter()
        .map(|n| completion(&format!("title-{n}")))
        .collect();
    let server = Server::start(replies);
    let (mut app, _) = fixture_app(&f, server.url().to_owned());
    let effects = common::ui::send(&mut app, Message::GenerateMarked).await;
    assert!(
        effects.any(|message| matches!(message, Message::GenerationFinished { result: Ok(_), .. }))
    );
    assert_eq!(
        describe::batch_summary(&app).as_deref(),
        Some("Generated 3 of 3")
    );
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|request| request["body"]["model"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["google/gemma-4-26b-a4b-it:free"; 3]
    );
    for n in 1..=3 {
        assert_eq!(f.read()[n - 1]["title"], format!("title-{n}"));
        assert_eq!(f.read()[n - 1]["state"], "DESCRIBED");
    }
}

#[tokio::test]
async fn startup_model_option_selects_the_model_sent_to_the_provider() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1)]));
    let server = common::http::Server::start(vec![completion("Generated")]);
    let (mut app, _) =
        fixture_app_with_model(&f, server.url().to_owned(), Some(VISION_MODELS[1].clone()));
    let completion = common::ui::first_output(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(
        completion,
        Message::GenerationFinished { result: Ok(_), .. }
    ));
    let requests = server.requests();
    server.finish();
    assert_eq!(requests[0]["body"]["model"], "google/gemma-4-31b-it:free");
}

#[test]
fn describe_view_constructs_from_loaded_state() {
    let f = Fixture::new();
    let (app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let _: Element<'_, Message> = describe::view(&app);
    assert_eq!(VISION_MODELS.len(), 2);
}

#[tokio::test]
async fn describe_layout_is_saved_and_survives_restart_navigation_and_refresh() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    assert_eq!(app.describe_layout(), describe::DescribeLayout::Grid);
    let _ = app.update(Message::SelectDescribeLayout(
        describe::DescribeLayout::Rows,
    ));
    assert_eq!(app.describe_layout(), describe::DescribeLayout::Rows);
    assert_eq!(app.status_message(), None);
    assert_eq!(
        std::fs::read_to_string(f.path(".publicator/ui.json")).unwrap(),
        r#"{"describe_layout":"rows"}"#
    );
    let (mut restarted, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    assert_eq!(restarted.describe_layout(), describe::DescribeLayout::Rows);
    let _: Element<'_, Message> = describe::view(&restarted);
    common::ui::send(&mut restarted, Message::Navigate(Tab::Publish)).await;
    common::ui::send(&mut restarted, Message::Navigate(Tab::Describe)).await;
    common::ui::send(&mut restarted, Message::Refresh).await;
    assert_eq!(restarted.describe_layout(), describe::DescribeLayout::Rows);
}

#[tokio::test]
async fn native_multiline_description_editor_keeps_unicode_and_literal_markup() {
    let fixture = Fixture::new();
    let mut scheduled = row(900, "SCHEDULED");
    scheduled["description"] = json!("");
    fixture.write(json!([scheduled]));
    let (mut app, _) = fixture_app(&fixture, "http://127.0.0.1:1".into());
    let uuid = id(900);
    let _ = app.update(Message::DescriptionAction {
        uuid: uuid.clone(),
        action: iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Paste(
            std::sync::Arc::new("first line\n雪 <script>alert('literal')</script>".into()),
        )),
    });
    let saved = common::ui::first_output(app.update(Message::SaveDraft(uuid))).await;
    assert!(matches!(
        saved,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    assert_eq!(
        fixture.read()[0]["description"],
        "first line\n雪 <script>alert('literal')</script>"
    );
}

/// The generation states of cards 1..=3, in id order, with any failure's error elided.
fn card_states(app: &App) -> Vec<&'static str> {
    (1..=3)
        .map(|n| match app.generation_state(&id(n)) {
            None => "-",
            Some(GenerationState::Queued) => "queued",
            Some(GenerationState::Generating) => "generating",
            Some(GenerationState::Generated) => "generated",
            Some(GenerationState::Failed(error)) => {
                assert!(!error.is_empty());
                "failed"
            }
        })
        .collect()
}

#[tokio::test]
async fn batch_reports_each_card_and_the_summary_in_processing_order() {
    let f = Fixture::new();
    let rows: Vec<_> = (1..=3).map(|n| f.marked_row(n)).collect();
    f.write(json!(rows));
    // OpenRouter runs 3, 2, 1; record 2's model and rescue catalogue both fail.
    let failure = |path: &str| Reply::new(path, 500, "fixture failure");
    let server = common::http::Server::start(vec![
        completion("title-3"),
        failure("/chat/completions"),
        failure("/models"),
        completion("title-1"),
    ]);
    let (mut app, _) = fixture_app(&f, server.url().to_owned());
    let task = app.update(Message::GenerateMarked);
    assert_eq!(card_states(&app), ["queued"; 3]);
    assert_eq!(
        describe::batch_summary(&app).as_deref(),
        Some("Preparing 3…")
    );
    let mut actions = iced_runtime::task::into_stream(task).unwrap();
    let mut step = async |app: &mut App| {
        let message = common::ui::next_output(&mut actions).await;
        let event = match &message {
            Message::GenerationProgress(event) => Some(event.clone()),
            _ => None,
        };
        common::ui::send(app, message).await;
        event
    };
    assert_eq!(
        step(&mut app).await,
        Some(BatchEvent::Admitted(vec![id(3), id(2), id(1)]))
    );
    assert_eq!(card_states(&app), ["queued"; 3]);
    let expected = [
        (
            3,
            ["queued", "queued", "generating"],
            "Generating 1 of 3 · 0 done · 0 failed",
        ),
        (
            3,
            ["queued", "queued", "generated"],
            "Generating 1 of 3 · 1 done · 0 failed",
        ),
        (
            2,
            ["queued", "generating", "generated"],
            "Generating 2 of 3 · 1 done · 0 failed",
        ),
        (
            2,
            ["queued", "failed", "generated"],
            "Generating 2 of 3 · 1 done · 1 failed",
        ),
        (
            1,
            ["generating", "failed", "generated"],
            "Generating 3 of 3 · 1 done · 1 failed",
        ),
        (
            1,
            ["generated", "failed", "generated"],
            "Generating 3 of 3 · 2 done · 1 failed",
        ),
    ];
    for (index, (n, states, summary)) in expected.into_iter().enumerate() {
        let event = step(&mut app).await;
        if index % 2 == 0 {
            assert_eq!(event, Some(BatchEvent::Started(id(n))));
        } else {
            assert!(
                matches!(&event, Some(BatchEvent::Finished(result)) if result.uuid == id(n)),
                "unexpected event {event:?}"
            );
        }
        assert_eq!(card_states(&app), states);
        assert_eq!(describe::batch_summary(&app).as_deref(), Some(summary));
        if index == 2 {
            assert_eq!(
                f.read()[2]["title"],
                "title-3",
                "persisted before the next start"
            );
        }
    }
    assert_eq!(step(&mut app).await, None, "the run's end");
    assert_eq!(card_states(&app), ["generated", "failed", "generated"]);
    assert_eq!(
        describe::batch_summary(&app).as_deref(),
        Some("Generated 2 of 3 · 1 failed")
    );
    server.finish();
    assert_eq!(f.read()[1]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    // A successful Refresh clears every finished state and the summary.
    common::ui::send(&mut app, Message::Refresh).await;
    assert_eq!(card_states(&app), ["-"; 3]);
    assert_eq!(describe::batch_summary(&app), None);
}

#[tokio::test]
async fn busy_card_refuses_every_action_with_a_message() {
    let blocked = [
        Message::GenerateOne(id(1)),
        Message::ApproveMetadata(id(1)),
        Message::ReturnToSchedule(id(1)),
        Message::SaveDraft(id(1)),
        Message::Edited {
            uuid: id(1),
            field: publicator::ui::drafts::Field::Title,
            text: "blocked".into(),
        },
        Message::DescriptionAction {
            uuid: id(1),
            action: iced::widget::text_editor::Action::Edit(
                iced::widget::text_editor::Edit::Insert('x'),
            ),
        },
        Message::AskDeletePublication(id(1)),
    ];
    for message in blocked {
        let f = Fixture::new();
        f.write(json!([f.marked_row(1), marker_row(2)]));
        let before = std::fs::read(&f.store.paths().json).unwrap();
        let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
        // The batch task is never polled: its cards stay Queued.
        let _held = app.update(Message::GenerateMarked);
        assert_eq!(app.generation_state(&id(1)), Some(&GenerationState::Queued));
        let what = format!("{message:?}");
        let effects = common::ui::run(app.update(message)).await;
        assert!(effects.messages.is_empty(), "{what} started a task");
        assert_eq!(
            app.status_message(),
            Some("Wait for this image's generation to finish"),
            "{what}"
        );
        // No delete banner is pending, and no draft reached the store.
        common::ui::run(app.update(Message::ConfirmDeletePublication)).await;
        common::ui::run(app.update(Message::SaveDraft(id(1)))).await;
        assert_eq!(app.generation_state(&id(1)), Some(&GenerationState::Queued));
        assert_eq!(
            std::fs::read(&f.store.paths().json).unwrap(),
            before,
            "{what}"
        );
    }
}

#[tokio::test]
async fn generate_marked_during_a_running_batch_starts_nothing() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), marker_row(2)]));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let _held = app.update(Message::GenerateMarked);
    let effects = common::ui::run(app.update(Message::GenerateMarked)).await;
    assert!(effects.messages.is_empty());
    assert_eq!(
        app.status_message(),
        Some("A batch generation is already running")
    );
    assert_eq!(
        describe::batch_summary(&app).as_deref(),
        Some("Preparing 2…")
    );
}

#[tokio::test]
async fn batch_level_failure_clears_its_queued_cards_and_reports_the_error() {
    let f = Fixture::new();
    f.write(json!([f.marked_row(1), marker_row(2)]));
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let task = app.update(Message::GenerateMarked);
    assert_eq!(app.generation_state(&id(2)), Some(&GenerationState::Queued));
    // Admission reads an invalid document and fails the whole batch.
    std::fs::write(&f.store.paths().json, "not json").unwrap();
    let finished = common::ui::first_output(task).await;
    assert!(matches!(
        finished,
        Message::GenerationFinished { result: Err(_), .. }
    ));
    let _ = app.update(finished);
    assert_eq!(app.generation_state(&id(1)), None);
    assert_eq!(app.generation_state(&id(2)), None);
    assert!(
        describe::batch_summary(&app)
            .is_some_and(|summary| summary.starts_with("Generation failed: ")),
        "{:?}",
        describe::batch_summary(&app)
    );
}
