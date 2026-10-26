mod common;
use common::http::{Reply, Server};
use common::{Fixture, id, row};
use publicator::{
    application::Application,
    config::{Config, Paths},
    metadata::{Endpoints, GenerationRequest, MetadataClient, Source, VISION_MODELS},
    store::Store,
};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
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
async fn marked_batch_uses_configuration_from_data_dir_for_external_json() {
    let f = Fixture::new();
    f.image("art.png", 4, 4);
    std::fs::write(
        f.path("publicator.toml"),
        "[deviantart]\ngalleries = ['Allowed']\n",
    )
    .unwrap();
    let config = Config::load(f.root.path()).unwrap();
    let external = tempfile::tempdir().unwrap();
    let json_path = external.path().join("rows.json");

    let make_row = |gallery: &str| {
        let mut publication = row(1, "SCHEDULED");
        publication["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        publication["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        publication["files"][0]["basename"] = json!("art.png");
        publication["files"][0]["sha512sum"] =
            json!(publicator::images::file_sha512(&f.path("art.png")).unwrap());
        publication["apparitions"] = json!([{
            "platformName":"deviantart", "state":"unpublished", "galleries":[gallery]
        }]);
        publication
    };

    let allowed_paths = Paths::resolve(Some(f.root.path()), Some(&json_path)).unwrap();
    let allowed_store = Store::new(allowed_paths, config.clone()).unwrap();
    std::fs::write(&json_path, json!([make_row("Allowed")]).to_string()).unwrap();
    let server = Server::start(vec![reply(
        "/chat/completions",
        json!({"choices":[{"message":{"content":content("Allowed title")}}]}),
    )]);
    let allowed_app = Application::new(Arc::new(allowed_store), client(server.url().to_owned()));
    let report = allowed_app
        .generate_marked(&request(Source::OpenRouter))
        .await
        .unwrap();
    assert_eq!((report.succeeded, report.failed), (1, 0));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&json_path).unwrap()).unwrap()
            [0]["title"],
        "Allowed title"
    );
    assert_eq!(server.requests().len(), 1);

    std::fs::write(&json_path, json!([make_row("Not allowed")]).to_string()).unwrap();
    let invalid_paths = Paths::resolve(Some(f.root.path()), Some(&json_path)).unwrap();
    let invalid_store = Store::new(invalid_paths, config).unwrap();
    let invalid_app =
        Application::new(Arc::new(invalid_store), client("http://127.0.0.1:1".into()));
    let before = std::fs::read(&json_path).unwrap();
    let error = invalid_app
        .generate_marked(&request(Source::OpenRouter))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Not allowed"));
    assert_eq!(std::fs::read(&json_path).unwrap(), before);
}

#[tokio::test]
async fn empty_local_marked_batch_is_noop_without_model_or_service_setup() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    let marker = f.path("llmfit-called");
    let tool = f.path("llmfit");
    common::executable::write_executable(
        &tool,
        &format!(
            "#!/bin/sh\ntouch '{}'\nprintf '%s' '{{\"models\":[]}}'\n",
            marker.display()
        ),
    );
    let app = Application::new(
        Arc::new(f.store.clone()),
        local_client("http://127.0.0.1:1".into(), &tool),
    );
    let report = app.generate_marked(&request(Source::Local)).await.unwrap();
    assert_eq!(
        (report.succeeded, report.failed, report.results.len()),
        (0, 0, 0)
    );
    assert!(!marker.exists());
    assert_eq!(f.read()[0], row(1, "SCHEDULED"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marked_batch_admission_uses_eligibility_after_a_waiting_edit() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
    publication["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
    f.write(json!([publication]));
    let app = Arc::new(Application::new(
        Arc::new(f.store.clone()),
        client("http://127.0.0.1:1".into()),
    ));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mutation_app = Arc::clone(&app);
    let mutation = tokio::task::spawn_blocking(move || {
        mutation_app.mutate(Some(&id(1)), |store| {
            store.mutate(|rows| {
                rows[0]["title"] = json!("Manual title");
                rows[0]["description"] = json!("Manual description");
                Ok(())
            })?;
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let batch_app = Arc::clone(&app);
    let (started_tx, started_rx) = mpsc::channel();
    let batch = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        batch_app
            .generate_marked(&request(Source::OpenRouter))
            .await
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_tx.send(()).unwrap();
    mutation.await.unwrap().unwrap();

    let report = batch.await.unwrap().unwrap();
    assert_eq!(
        (report.succeeded, report.failed, report.results.len()),
        (0, 0, 0)
    );
    assert_eq!(f.read()[0]["title"], "Manual title");
    assert_eq!(f.read()[0]["description"], "Manual description");
}

#[tokio::test]
async fn generation_lease_excludes_duplicate_and_same_uuid_edits_but_not_other_work() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([row(1, "SCHEDULED"), row(2, "SCHEDULED")]));
    let (server, started_rx, release_tx) = common::http::Server::start_paused(common::http::Reply {
        path: "/chat/completions".into(),
        status: 200,
        body: json!({"choices":[{"message":{"content":json!({"title":"Generated title","description":"A generated description."}).to_string()}}]}).to_string(),
    });
    let app = Arc::new(Application::new(
        Arc::new(f.store.clone()),
        client(server.url().to_owned()),
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
    server.finish();
    assert_eq!(f.read()[0]["title"], "Generated title");
}

#[tokio::test]
async fn openrouter_batch_continues_after_failure_and_ignores_inexact_marker() {
    let f = Fixture::new();
    let mut rows = Vec::new();
    for n in 1..=3 {
        f.image(&format!("{n}.png"), 4, 4);
        let mut publication = row(n, "SCHEDULED");
        publication["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        publication["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        rows.push(publication);
    }
    rows[2]["title"] = json!("[[PUBLICATOR:TODO_GENERATE]] ");
    rows[2]["description"] = json!("[[PUBLICATOR:TODO_GENERATE]] ");
    f.write(json!(rows));
    let server = Server::start(vec![
        Reply {
            path: "/chat/completions".into(),
            status: 500,
            body: json!({"error":{"message":"fixture failure"}}).to_string(),
        },
        reply("/models", json!({"data":[]})),
        reply(
            "/chat/completions",
            json!({"choices":[{"message":{"content":content("Generated")}}]}),
        ),
    ]);
    let app = Application::new(Arc::new(f.store.clone()), client(server.url().to_owned()));

    let report = app
        .generate_marked(&request(Source::OpenRouter))
        .await
        .unwrap();
    assert_eq!((report.succeeded, report.failed), (1, 1));
    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.uuid.as_str())
            .collect::<Vec<_>>(),
        [id(2).as_str(), id(1).as_str()]
    );
    let saved = f.read();
    assert_eq!(saved[0]["title"], "Generated");
    assert_eq!(saved[0]["description"], "A complete generated description.");
    assert_eq!(saved[1]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(saved[1]["description"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(saved[2]["title"], "[[PUBLICATOR:TODO_GENERATE]] ");
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["path"] == "/chat/completions")
            .count(),
        2
    );
}

#[tokio::test]
async fn batch_persists_each_success_before_continuing_after_failure() {
    let f = Fixture::new();
    let mut rows = Vec::new();
    for n in 1..=3 {
        f.image(&format!("{n}.png"), 4, 4);
        let mut publication = row(n, "SCHEDULED");
        publication["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        publication["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        rows.push(publication);
    }
    f.write(json!(rows));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let database = f.store.paths().json.clone();
    let (observed_tx, observed_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let respond = |status: u16, body: String| {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 16_384];
            let _ = stream.read(&mut request).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        };
        respond(
            200,
            json!({"choices":[{"message":{"content":content("title-3")}}]}).to_string(),
        );
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 16_384];
        let _ = stream.read(&mut request).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(database).unwrap()).unwrap();
        observed_tx
            .send(
                saved[2]["title"].as_str() == Some("title-3")
                    && saved[2]["description"].as_str()
                        == Some("A complete generated description."),
            )
            .unwrap();
        let failure = json!({"error":{"message":"fixture failure"}}).to_string();
        write!(
            stream,
            "HTTP/1.1 500 Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{failure}",
            failure.len()
        )
        .unwrap();
        respond(200, json!({"data":[]}).to_string());
        respond(
            200,
            json!({"choices":[{"message":{"content":content("title-1")}}]}).to_string(),
        );
    });
    let app = Application::new(Arc::new(f.store.clone()), client(endpoint));
    let report = app
        .generate_marked(&request(Source::OpenRouter))
        .await
        .unwrap();
    server.join().unwrap();
    assert!(observed_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    assert_eq!((report.succeeded, report.failed), (2, 1));
    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.uuid.as_str())
            .collect::<Vec<_>>(),
        [id(3).as_str(), id(2).as_str(), id(1).as_str()]
    );
    let saved = f.read();
    assert_eq!(saved[0]["title"], "title-1");
    assert_eq!(saved[0]["description"], "A complete generated description.");
    assert_eq!(saved[1]["title"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(saved[1]["description"], "[[PUBLICATOR:TODO_GENERATE]]");
    assert_eq!(saved[2]["title"], "title-3");
    assert_eq!(saved[2]["description"], "A complete generated description.");
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
            common::executable::write_executable(
                &tool,
                &format!(
                    "#!/bin/sh\nprintf '%s' '{{\"models\":[{{\"name\":\"{}\",\"fit_level\":\"Good\"}}]}}'\n",
                    VISION_MODELS[0].llmfit_name
                ),
            );
            local_client(server.url().to_owned(), &tool)
        };
        let app = Application::new(Arc::new(f.store.clone()), metadata);
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
    );
    let lease = app.begin_publishing().unwrap();
    assert!(app.mutate(Some(&id(1)), |_| Ok(())).is_err());
    drop(lease);
    assert!(app.mutate(Some(&id(1)), |_| Ok(())).is_ok());
}

#[tokio::test]
async fn invalid_phase_model_and_database_do_not_write_or_call_provider() {
    use std::io::ErrorKind;

    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([row(1, "PICKED")]));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (thread_calls, thread_stop) = (Arc::clone(&calls), Arc::clone(&stop));
    let server = thread::spawn(move || {
        while !thread_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((_, _)) => {
                    thread_calls.fetch_add(1, Ordering::Relaxed);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("counter listener failed: {error}"),
            }
        }
    });
    let app = Application::new(Arc::new(f.store.clone()), client(endpoint));
    let before_phase = std::fs::read(&f.store.paths().json).unwrap();
    assert!(
        app.generate_one(&id(1), request(Source::OpenRouter))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before_phase);

    f.store
        .mutate(|rows| {
            rows[0]["state"] = json!("SCHEDULED");
            Ok(())
        })
        .unwrap();
    let before_model = std::fs::read(&f.store.paths().json).unwrap();
    let mut invalid = request(Source::OpenRouter);
    invalid.model.ollama_name = "unapproved".into();
    assert!(app.generate_one(&id(1), invalid).await.is_err());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before_model);

    for invalid_field in ["uuid", "submissionTimestamp"] {
        let mut invalid = row(1, "SCHEDULED");
        invalid["title"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        invalid["description"] = json!("[[PUBLICATOR:TODO_GENERATE]]");
        if invalid_field == "uuid" {
            invalid["uuid"] = json!("short");
        } else {
            invalid.as_object_mut().unwrap().remove(invalid_field);
        }
        f.write(json!([invalid]));
        let before_invalid = std::fs::read(&f.store.paths().json).unwrap();
        assert!(
            app.generate_marked(&request(Source::OpenRouter))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(&f.store.paths().json).unwrap(),
            before_invalid,
            "invalid {invalid_field} database was rewritten"
        );
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn accepted_provider_failure_saves_both_markers() {
    let f = Fixture::new();
    f.image("1.png", 4, 4);
    f.write(json!([row(1, "SCHEDULED")]));
    let app = Application::new(
        Arc::new(f.store.clone()),
        client("http://127.0.0.1:1".into()),
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
    let app = Application::new(Arc::new(f.store.clone()), client(server.url().to_owned()));
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
