#![cfg(feature = "desktop")]

mod common;
use common::{Fixture, id, row};
use iced::Element;
use publicator::{
    application::Application,
    metadata::{Endpoints, MetadataClient, Source, VISION_MODELS, available_sources},
    ui::{App, Message, Tab, UiOptions, describe},
};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

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
    let _ = app.update(Message::Navigate(Tab::Describe));
    (app, application)
}

fn marker_row(n: u64) -> serde_json::Value {
    let mut row = row(n, "SCHEDULED");
    row["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
    row["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
    row
}

#[test]
fn source_options_do_not_include_a_third_provider() {
    assert_eq!(available_sources(), &[Source::OpenRouter, Source::Local]);
}

#[test]
fn selecting_local_source_does_not_itself_confirm_install() {
    let f = Fixture::new();
    let (mut app, _) = fixture_app(&f, "http://127.0.0.1:1".into());
    let _ = app.update(Message::SelectSource(Source::Local));
    let task = app.update(Message::GenerateOne(id(1)));
    assert!(iced_runtime::task::into_stream(task).is_none());
    assert!(iced_runtime::task::into_stream(app.update(Message::CancelLocalInstall)).is_none());
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
