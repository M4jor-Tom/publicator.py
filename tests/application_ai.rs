mod common;
use common::http::{Reply, Server};
use common::{Fixture, id, row};
use publicator::{
    application::Application,
    metadata::{Endpoints, GenerationRequest, MetadataClient, Source, VISION_MODELS},
};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

fn request(source: Source) -> GenerationRequest {
    GenerationRequest {
        source,
        model: VISION_MODELS[0].clone(),
        timeout: Duration::from_secs(2),
        install_missing: false,
    }
}
fn reply(path: &str, body: serde_json::Value) -> Reply {
    Reply {
        path: path.into(),
        status: 200,
        body: body.to_string(),
    }
}
fn content(title: &str) -> String {
    json!({"title":title,"description":"A complete generated description."}).to_string()
}
fn local_client(endpoint: String, llmfit: &std::path::Path) -> MetadataClient {
    MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: "http://127.0.0.1:1".into(),
            ollama: endpoint,
        },
        llmfit.to_path_buf(),
    )
}
fn client(endpoint: String) -> MetadataClient {
    MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: endpoint.clone(),
            ollama: endpoint,
        },
        "llmfit".into(),
    )
    .with_openrouter_key("fixture")
}

#[tokio::test]
async fn empty_local_marked_batch_is_noop_without_model_or_service_setup() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    let marker = f.path("llmfit-called");
    let tool = f.path("llmfit");
    std::fs::write(
        &tool,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf '%s' '{{\"models\":[]}}'\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let app = Application::new(
        Arc::new(f.store.clone()),
        local_client("http://127.0.0.1:1".into(), &tool),
        || 1_700_000_000,
    );
    let report = app.generate_marked(&request(Source::Local)).await.unwrap();
    assert_eq!(
        (report.succeeded, report.failed, report.results.len()),
        (0, 0, 0)
    );
    assert!(!marker.exists());
    assert_eq!(f.read()[0], row(1, "SCHEDULED"));
}

#[tokio::test]
async fn generation_lease_excludes_duplicate_and_same_uuid_edits_but_not_other_work() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([row(1, "SCHEDULED"), row(2, "SCHEDULED")]));
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
            json!({"title":"Generated title","description":"A generated description."}).to_string();
        let body = json!({"choices":[{"message":{"content":content}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let app = Arc::new(Application::new(
        Arc::new(f.store.clone()),
        client(endpoint),
        || 1_700_000_000,
    ));
    let task = {
        let app = Arc::clone(&app);
        tokio::spawn(async move { app.generate_one(&id(1), request(Source::OpenRouter)).await })
    };
    tokio::task::spawn_blocking(move || started_rx.recv_timeout(Duration::from_secs(3)).unwrap())
        .await
        .unwrap();
    assert!(app.mutate(Some(&id(1)), |_| Ok(())).is_err());
    assert!(
        app.generate_one(&id(1), request(Source::OpenRouter))
            .await
            .is_err()
    );
    assert!(app.begin_publishing().is_err());
    app.mutate(Some(&id(2)), |_| Ok(())).unwrap();
    release_tx.send(()).unwrap();
    assert!(task.await.unwrap().is_ok());
    server.join().unwrap();
    assert_eq!(f.read()[0]["title"], "Generated title");
}

#[tokio::test]
async fn successful_batches_save_in_provider_order() {
    for (source, expected) in [
        (Source::OpenRouter, vec![3, 2, 1]),
        (Source::Local, vec![1, 2, 3]),
    ] {
        let f = Fixture::new();
        let mut rows = Vec::new();
        for n in 1..=3 {
            f.image(&format!("{n}.png"), 4, 4);
            let mut p = row(n, "SCHEDULED");
            p["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
            p["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
            rows.push(p);
        }
        f.write(json!(rows));
        let replies = if source == Source::OpenRouter {
            expected
                .iter()
                .map(|n| {
                    reply(
                        "/chat/completions",
                        json!({"choices":[{"message":{"content":content(&format!("title-{n}"))}}]}),
                    )
                })
                .collect()
        } else {
            let mut replies = vec![reply(
                "/api/tags",
                json!({"models":[{"name":VISION_MODELS[0].ollama_name}]}),
            )];
            for n in &expected {
                replies.push(reply(
                    "/api/tags",
                    json!({"models":[{"name":VISION_MODELS[0].ollama_name}]}),
                ));
                replies.push(reply(
                    "/api/chat",
                    json!({"message":{"content":content(&format!("title-{n}"))}}),
                ));
            }
            replies
        };
        let server = Server::start(replies);
        let metadata = if source == Source::OpenRouter {
            client(server.url().to_owned())
        } else {
            let tool = f.path("llmfit");
            std::fs::write(&tool, format!("#!/bin/sh\nprintf '%s' '{{\"models\":[{{\"name\":\"{}\",\"fit_level\":\"Good\"}}]}}'\n", VISION_MODELS[0].llmfit_name)).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            local_client(server.url().to_owned(), &tool)
        };
        let app = Application::new(Arc::new(f.store.clone()), metadata, || 1_700_000_000);
        let report = app.generate_marked(&request(source)).await.unwrap();
        assert_eq!((report.succeeded, report.failed), (3, 0));
        assert_eq!(
            report
                .results
                .iter()
                .map(|r| r.uuid.clone())
                .collect::<Vec<_>>(),
            expected.iter().map(|n| id(*n)).collect::<Vec<_>>()
        );
        for n in 1..=3 {
            assert_eq!(f.read()[n - 1]["title"], format!("title-{n}"));
            assert_eq!(f.read()[n - 1]["state"], "DESCRIBED");
        }
    }
}

#[tokio::test]
async fn publishing_lease_blocks_edits_until_dropped() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    let app = Application::new(
        Arc::new(f.store.clone()),
        client("http://127.0.0.1:1".into()),
        || 1_700_000_000,
    );
    let lease = app.begin_publishing().unwrap();
    assert!(app.mutate(Some(&id(1)), |_| Ok(())).is_err());
    drop(lease);
    assert!(app.mutate(Some(&id(1)), |_| Ok(())).is_ok());
}

#[tokio::test]
async fn invalid_model_and_phase_do_not_write_generation_markers() {
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    let app = Application::new(
        Arc::new(f.store.clone()),
        client("http://127.0.0.1:1".into()),
        || 1_700_000_000,
    );
    let mut bad = request(Source::Local);
    bad.model.ollama_name = "unapproved".into();
    assert!(app.generate_one(&id(1), bad).await.is_err());
    assert_eq!(f.read()[0]["title"], "Title");
}

#[tokio::test]
async fn accepted_provider_failure_saves_both_markers() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([row(1, "SCHEDULED")]));
    let app = Application::new(
        Arc::new(f.store.clone()),
        client("http://127.0.0.1:1".into()),
        || 1_700_000_000,
    );
    let result = app.generate_one(&id(1), request(Source::OpenRouter)).await;
    assert!(result.is_err());
    assert_eq!(f.read()[0]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(f.read()[0]["description"], "[[PUBLICATOR:TODO_GENERATE]]");
}

#[tokio::test]
async fn openrouter_batch_reverses_persisted_order_and_keeps_marker_on_failure() {
    let f = Fixture::new();
    let mut rows = vec![];
    for n in 1..=3 {
        f.image(&format!("{n}.png"), 4, 4);
        let mut p = row(n, "SCHEDULED");
        p["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        rows.push(p);
    }
    f.write(json!(rows));
    let app = Application::new(
        Arc::new(f.store.clone()),
        client("http://127.0.0.1:1".into()),
        || 1_700_000_000,
    );
    let report = app
        .generate_marked(&request(Source::OpenRouter))
        .await
        .unwrap();
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.uuid.as_str())
            .collect::<Vec<_>>(),
        vec![id(3).as_str(), id(2).as_str(), id(1).as_str()]
    );
    assert_eq!((report.succeeded, report.failed), (0, 3));
    assert!(
        f.read()
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["description"] == "[[PUBLICATOR:TODO_GENERATE]]")
    );
}

#[tokio::test]
async fn batch_keeps_first_successful_safe_model_as_preference() {
    let f = Fixture::new();
    let mut rows = Vec::new();
    for n in 1..=3 {
        f.image(&format!("{n}.png"), 4, 4);
        let mut p = row(n, "SCHEDULED");
        p["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        p["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        rows.push(p);
    }
    f.write(json!(rows));
    let catalog = json!({"data":[
        {"id":"vendor/model-b:free","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0"}},
        {"id":"google/model-c:free","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0"}}
    ]});
    let success = |title: &str| {
        reply(
            "/chat/completions",
            json!({"choices":[{"message":{"content":content(title)}}]}),
        )
    };
    let failed = || Reply {
        path: "/chat/completions".into(),
        status: 500,
        body: json!({"error":{"message":"temporary"}}).to_string(),
    };
    let server = Server::start(vec![
        failed(),
        reply("/models", catalog.clone()),
        success("one"),
        failed(),
        success("two"),
        success("three"),
    ]);
    let app = Application::new(
        Arc::new(f.store.clone()),
        client(server.url().to_owned()),
        || 1_700_000_000,
    );
    let report = app
        .generate_marked(&request(Source::OpenRouter))
        .await
        .unwrap();
    assert_eq!((report.succeeded, report.failed), (3, 0));
    let models = server
        .requests()
        .iter()
        .map(|r| r["body"]["model"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        models,
        vec![
            Some("google/gemma-4-26b-a4b-it:free".into()),
            None,
            Some("vendor/model-b:free".into()),
            Some("vendor/model-b:free".into()),
            Some("google/model-c:free".into()),
            Some("vendor/model-b:free".into()),
        ]
    );
}
