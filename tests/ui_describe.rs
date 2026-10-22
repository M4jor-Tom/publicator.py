#![cfg(feature = "desktop")]

mod common;
use common::{Fixture, id, row};
use iced::Element;
use publicator::{
    application::Application,
    metadata::{Endpoints, MetadataClient, Source, VISION_MODELS, available_sources},
    ui::{App, Message, UiOptions, describe},
};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

async fn run_tasks(task: iced::Task<Message>) -> Vec<Message> {
    use iced::futures::StreamExt;
    use iced_runtime::Action;
    let mut actions = iced_runtime::task::into_stream(task).expect("dispatched task stream");
    let mut messages = Vec::new();
    while let Some(action) = actions.next().await {
        if let Action::Output(message) = action {
            messages.push(message);
        }
    }
    messages
}

async fn run_task(task: iced::Task<Message>) -> Message {
    use iced::futures::StreamExt;
    use iced_runtime::Action;
    let mut actions = iced_runtime::task::into_stream(task).expect("dispatched task stream");
    while let Some(action) = actions.next().await {
        if let Action::Output(message) = action {
            return message;
        }
    }
    panic!("dispatched task produced no completion message");
}

fn fixture_app(f: &Fixture, endpoint: String) -> (App, Arc<Application>) {
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
    let application = Arc::new(Application::new(store, client, || 1_700_000_000));
    let (mut app, _) = App::from_application(
        application.clone(),
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(5),
        },
    );
    let _ = app.update(Message::LoadFinished(Ok(f.store.read().unwrap())));
    (app, application)
}

fn marker_row(n: u64) -> serde_json::Value {
    let mut row = row(n, "SCHEDULED");
    row["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
    row["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
    row
}

fn fake_llmfit(f: &Fixture, level: &str, notes: &[&str]) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
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
    std::fs::write(
        &path,
        format!(
            "#!{}\ncat <<'EOF'\n{}\nEOF\n",
            shell.trim_end(),
            json!({"models":models})
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn failed_llmfit(f: &Fixture) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let shell = std::process::Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    let path = f.path("llmfit-failed");
    std::fs::write(
        &path,
        format!(
            "#!{}\nprintf 'fit unavailable' >&2\nexit 1\n",
            String::from_utf8(shell.stdout).unwrap().trim_end()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    let application = Arc::new(Application::new(store, client, || 1_700_000_000));
    let (mut app, _) = App::from_application(
        application.clone(),
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(5),
        },
    );
    let _ = app.update(Message::LoadFinished(Ok(f.store.read().unwrap())));
    (app, application)
}

fn ollama_reply(path: &str, body: serde_json::Value) -> common::http::Reply {
    common::http::Reply {
        path: path.into(),
        status: 200,
        body: body.to_string(),
    }
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
    std::fs::write(
        &llmfit,
        format!(
            "#!{}\ncat <<'EOF'\n{}\nEOF\n", shell.trim_end(),
            json!({"models":[{"name":VISION_MODELS[0].llmfit_name,"fit_level":"Too Tight","notes":["Needs 40 GB RAM"]}]})
        ),
    ).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&llmfit, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        std::process::Command::new(&llmfit)
            .output()
            .unwrap()
            .status
            .success()
    );
    let store = Arc::new(f.store.clone());
    let client = MetadataClient::new(reqwest::Client::new(), Endpoints::default(), llmfit);
    let application = Arc::new(Application::new(store, client, || 1_700_000_000));
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(5),
        },
    );
    let _ = app.update(Message::LoadFinished(Ok(f.store.read().unwrap())));
    let _ = app.update(Message::SelectSource(Source::Local));
    let task = app.update(Message::GenerateOne(id(1)));
    let fit_result = run_task(task).await;
    assert!(matches!(fit_result, Message::LocalFitFinished { .. }));
    let _ = app.update(fit_result);
    let status = app.local_fit_status().unwrap();
    assert_eq!(status.fit.reason, "Needs 40 GB RAM");
    assert!(!app.local_install_consent_pending());
}

#[tokio::test]
async fn installed_good_local_model_generates_without_pull_or_install_modal() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1)]));
    let model_name = VISION_MODELS[0].ollama_name.clone();
    let tags = json!({"models":[{"name":model_name}]});
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags),
        ollama_reply(
            "/api/chat",
            json!({"message":{"content":json!({"title":"Installed model","description":"Generated locally."}).to_string()}}),
        ),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits in memory"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let status = app.local_fit_status().unwrap();
    assert_eq!(status.fit.reason, "Fits in memory");
    assert!(status.available && status.installed);
    let checked = run_task(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(checked, Message::LocalFitFinished { .. }));
    assert!(!app.local_install_consent_pending());
    let generation = app.update(checked);
    let completed = run_task(generation).await;
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
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let tags = json!({"models":[]});
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags),
        ollama_reply("/api/pull", json!({"status":"success"})),
        ollama_reply(
            "/api/chat",
            json!({"message":{"content":json!({"title":"Installed after consent","description":"Generated locally."}).to_string()}}),
        ),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let checked = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    let _ = app.update(run_task(generation).await);
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
async fn installed_local_batch_keeps_identity_through_dirty_flush_and_admission() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1), marker_row(2)]));
    let model_name = VISION_MODELS[0].ollama_name.clone();
    let tags = json!({"models":[{"name":model_name}]});
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);

    // The Good/installed fit result immediately continues without consent. Add
    // a draft while status is pending so the batch identity must survive save.
    let fit_task = app.update(Message::GenerateMarked);
    let status = run_task(fit_task).await;
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
    let flush = app.update(status);
    let saved = run_task(flush).await;
    let generation = app.update(saved);

    // The generation Task has not been polled/admitted yet. Its expected batch
    // must still be the original two UUIDs, despite the dirty draft replacing
    // one marker during the required save.
    assert_eq!(f.read()[0]["title"], "Manual title");
    let after_flush = std::fs::read(&f.store.paths().json).unwrap();
    let finished = run_task(generation).await;
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
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1), marker_row(2)]));
    let model_name = VISION_MODELS[0].ollama_name.clone();
    let tags = json!({"models":[{"name":model_name}]});
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let status = run_task(app.update(Message::GenerateMarked)).await;
    let generation = app.update(status);
    f.store
        .mutate(|rows| {
            rows[0]["title"] = json!("Manual title");
            rows[0]["description"] = json!("Manual description");
            Ok(())
        })
        .unwrap();
    let saved = std::fs::read(&f.store.paths().json).unwrap();
    let finished = run_task(generation).await;
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
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1), marker_row(2)]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let tags = json!({"models": []});
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/tags", tags.clone()),
        ollama_reply("/api/pull", json!({"status":"success"})),
        ollama_reply(
            "/api/chat",
            json!({"message":{"content":json!({"title":"Should not run","description":"Changed batch."}).to_string()}}),
        ),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let checked = run_task(app.update(Message::GenerateMarked)).await;
    let _ = app.update(checked);
    assert!(app.local_install_consent_pending());

    // Confirmation creates an asynchronous Application task. Change eligibility
    // before polling it to exercise the admission boundary, not just UI state.
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
    let finished = run_task(confirm).await;
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
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", json!({"models":[]})),
        ollama_reply("/api/tags", json!({"models":[]})),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let checked = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    f.image("1.png", 4, 4);
    f.image("2.png", 4, 4);
    f.write(json!([marker_row(1), marker_row(2)]));
    let empty = json!({"models":[]});
    let mut replies = (0..5)
        .map(|_| ollama_reply("/api/tags", empty.clone()))
        .collect::<Vec<_>>();
    replies.push(ollama_reply("/api/pull", json!({"status":"success"})));
    replies.push(ollama_reply("/api/chat", json!({"message":{"content":json!({"title":"Only second","description":"Consent bound to target."}).to_string()}})));
    let server = common::http::Server::start(replies);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let first = run_task(app.update(Message::GenerateOne(id(1)))).await;
    let second = run_task(app.update(Message::GenerateOne(id(2)))).await;
    let _ = app.update(first);
    assert!(!app.local_install_consent_pending());
    let _ = app.update(second);
    assert!(app.local_install_consent_pending());
    let generation = app.update(Message::ConfirmLocalInstall);
    let _ = app.update(run_task(generation).await);
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
        let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
        let _ = app.update(selected);
        let checked = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let checked = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let checked = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    let empty = json!({"models":[]});
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", empty.clone()),
        ollama_reply("/api/tags", empty),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let consent_result = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", json!({"models":[]})),
        ollama_reply("/api/tags", json!({"models":[]})),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let stale = run_task(app.update(Message::GenerateOne(id(1)))).await;
    let current = run_task(app.update(Message::SelectModel(VISION_MODELS[1].clone()))).await;
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
    let server = common::http::Server::start(vec![
        ollama_reply("/api/tags", json!({"models":[]})),
        ollama_reply("/api/tags", json!({"models":[]})),
    ]);
    let (mut app, _) = local_fixture_app(
        &f,
        fake_llmfit(&f, "Good", &["Fits host"]),
        server.url().into(),
    );
    let selected = run_task(app.update(Message::SelectSource(Source::Local))).await;
    let _ = app.update(selected);
    let stale = run_task(app.update(Message::GenerateOne(id(1)))).await;
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
    let image = f.image("1.png", 4, 4);
    assert!(image.exists());
    f.write(json!([marker_row(1), row(2, "SCHEDULED")]));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 8192];
        let _ = stream.read(&mut request).unwrap();
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        let content =
            json!({"title":"Generated", "description":"Saved after provider completion."})
                .to_string();
        let body = json!({"choices":[{"message":{"content":content}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let (mut app, _) = fixture_app(&f, endpoint);
    let task = tokio::spawn(run_task(app.update(Message::GenerateOne(id(1)))));
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
    let save_other = run_task(app.update(Message::SaveDraft(id(2)))).await;
    assert!(matches!(
        save_other,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    let _ = app.update(save_other);
    assert_eq!(f.read()[1]["title"], "unrelated");
    release_tx.send(()).unwrap();
    let completion = task.await.unwrap();
    let _ = app.update(completion);
    server.join().unwrap();
    let saved = f.read();
    assert_eq!(saved[0]["title"], "Generated");
    assert_eq!(saved[0]["state"], "DESCRIBED");
    assert_eq!(saved[1]["title"], "unrelated");
    assert_eq!(saved[1]["state"], "SCHEDULED");
}

#[tokio::test]
async fn previews_are_requested_for_initial_visible_cards_then_later_scrolled_cards() {
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
    let store = Arc::new(f.store.clone());
    let client = MetadataClient::new(
        reqwest::Client::new(),
        Endpoints::default(),
        "llmfit".into(),
    );
    let application = Arc::new(Application::new(store, client, || 0));
    let (mut app, _) = App::from_application(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(1),
        },
    );
    let candidates =
        publicator::ui::pick::search_candidates(&f.store, 25, &["images".into()]).unwrap();
    assert_eq!(candidates.len(), 10);
    let task = app.update(Message::SearchFinished(Ok(candidates)));
    let messages = run_tasks(task).await;
    for message in messages {
        let _ = app.update(message);
    }
    let cache = f.path(".thumbs");
    let initial = std::fs::read_dir(&cache).unwrap().count();
    assert!(initial < 10);
    let task = app.update(Message::PreviewViewport {
        offset: 2300.0,
        height: 700.0,
    });
    let messages = run_tasks(task).await;
    for message in messages {
        let _ = app.update(message);
    }
    let after_scroll = std::fs::read_dir(&cache).unwrap().count();
    assert!(after_scroll > initial);
    assert!(
        run_tasks(app.update(Message::PreviewViewport {
            offset: 2300.0,
            height: 700.0
        }))
        .await
        .is_empty()
    );
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), after_scroll);
}

#[tokio::test]
async fn describe_previews_share_lazy_scroll_boundary() {
    let f = Fixture::new();
    let mut rows = Vec::new();
    for n in 1..=10 {
        let image = f.image(&format!("{n}.png"), 3 + n as u32, 3);
        assert!(image.exists());
        let mut publication = row(n, "SCHEDULED");
        let digest = openssl::sha::sha512(&std::fs::read(image).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        publication["files"][0]["sha512sum"] = json!(digest);
        rows.push(publication);
    }
    f.write(json!(rows));
    assert_eq!(f.store.read().unwrap().len(), 10);
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let messages = run_tasks(app.update(Message::DescribeViewport {
        offset: 0.0,
        height: 600.0,
    }))
    .await;
    for message in messages {
        let _ = app.update(message);
    }
    let cache = f.path(".thumbs");
    let initial = std::fs::read_dir(&cache).unwrap().count();
    assert!(initial < 10);
    let messages = run_tasks(app.update(Message::DescribeViewport {
        offset: 2400.0,
        height: 600.0,
    }))
    .await;
    for message in messages {
        let _ = app.update(message);
    }
    let after_scroll = std::fs::read_dir(&cache).unwrap().count();
    assert!(after_scroll > initial);
    assert!(
        run_tasks(app.update(Message::DescribeViewport {
            offset: 2400.0,
            height: 600.0,
        }))
        .await
        .is_empty()
    );
    assert_eq!(std::fs::read_dir(cache).unwrap().count(), after_scroll);
}

#[tokio::test]
async fn delayed_title_save_preserves_newer_same_and_other_field_edits() {
    use fs2::FileExt;
    use iced_runtime::{Action, futures::futures::StreamExt};

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
        let mut actions = iced_runtime::task::into_stream(task).expect("save task stream");
        let first_save = tokio::spawn(async move {
            loop {
                match actions.next().await {
                    Some(Action::Output(message)) => break message,
                    Some(_) => {}
                    None => panic!("first save task ended without completion"),
                }
            }
        });
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
        let second = run_task(second_task).await;
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
async fn dirty_draft_is_saved_before_single_generation_starts() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1)]));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 8192];
        let _ = stream.read(&mut request).unwrap();
        let content =
            json!({"title":"Generated", "description":"Generated description."}).to_string();
        let body = json!({"choices":[{"message":{"content":content}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let (mut app, _) = fixture_app(&f, endpoint);
    let _ = app.update(Message::Edited {
        uuid: id(1),
        field: publicator::ui::drafts::Field::Title,
        text: "Manual draft".into(),
    });
    let saved = run_task(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(
        saved,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    assert_eq!(f.read()[0]["title"], "Manual draft");
    let generation = app.update(saved);
    let completed = run_task(generation).await;
    let _ = app.update(completed);
    server.join().unwrap();
    assert_eq!(f.read()[0]["title"], "Generated");
}

#[tokio::test]
async fn failed_single_generation_can_be_retried_from_describe() {
    use common::http::{Reply, Server};

    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([marker_row(1)]));
    let server = Server::start(vec![
        Reply {
            path: "/chat/completions".into(),
            status: 500,
            body: json!({"error":{"message":"fixture failure"}}).to_string(),
        },
        common::http::Reply {
            path: "/models".into(),
            status: 200,
            body: json!({"data":[]}).to_string(),
        },
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 200,
            body: json!({"choices":[{"message":{"content":json!({"title":"Generated","description":"Generated description."}).to_string()}}]}).to_string(),
        },
    ]);
    let (mut app, _) = fixture_app(&f, server.url().to_owned());

    let failed = run_task(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(
        failed,
        Message::GenerationFinished { result: Err(_), .. }
    ));
    let _ = app.update(failed);
    assert_eq!(f.read()[0]["state"], "SCHEDULED");
    assert_eq!(f.read()[0]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(f.read()[0]["description"], "[[PUBLICATOR:TODO_GENERATE]]");

    let retried = run_task(app.update(Message::GenerateOne(id(1)))).await;
    assert!(matches!(
        retried,
        Message::GenerationFinished { result: Ok(_), .. }
    ));
    let _ = app.update(retried);
    assert_eq!(f.read()[0]["title"], "Generated");
    assert_eq!(f.read()[0]["description"], "Generated description.");
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
    use common::http::{Reply, Server};
    let f = Fixture::new();
    let mut rows = Vec::new();
    for n in 1..=3 {
        f.image(&format!("{n}.png"), 4, 4);
        rows.push(marker_row(n));
    }
    f.write(json!(rows));
    let replies = [3, 2, 1].into_iter().map(|n| Reply {
        path: "/chat/completions".into(), status: 200,
        body: json!({"choices":[{"message":{"content":json!({"title":format!("title-{n}"),"description":"Generated description."}).to_string()}}]}).to_string(),
    }).collect();
    let server = Server::start(replies);
    let (mut app, _) = fixture_app(&f, server.url().to_owned());
    let completion = run_task(app.update(Message::GenerateMarked)).await;
    assert!(matches!(
        &completion,
        Message::GenerationFinished { result: Ok(_), .. }
    ));
    let _ = app.update(completion);
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

#[test]
fn describe_view_constructs_from_loaded_state() {
    let f = Fixture::new();
    let (app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let _: Element<'_, Message> = describe::view(&app);
    assert_eq!(VISION_MODELS.len(), 2);
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
    let saved = run_task(app.update(Message::SaveDraft(uuid))).await;
    assert!(matches!(
        saved,
        Message::SaveFinished { result: Ok(()), .. }
    ));
    assert_eq!(
        fixture.read()[0]["description"],
        "first line\n雪 <script>alert('literal')</script>"
    );
}
