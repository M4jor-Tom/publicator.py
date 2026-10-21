use publicator::{
    images::PreparedImage,
    metadata::{CatalogModel, Endpoints, GenerationRequest, MetadataClient, ModelOption, Source},
};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

mod common;

fn client(base: &str) -> MetadataClient {
    MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: base.into(),
            ollama: String::new(),
        },
        PathBuf::new(),
    )
    .with_openrouter_key("DUMMY-test-credential")
}
fn image() -> (tempfile::TempDir, PreparedImage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("art.png");
    std::fs::write(&path, b"png bytes").unwrap();
    (
        dir,
        PreparedImage {
            path,
            mime: "image/png".into(),
        },
    )
}
fn completion(model: &str) -> String {
    json!({"choices":[{"message":{"content":format!("{{\"title\":\"T\",\"description\":\"D\"}}")}}], "model": model}).to_string()
}
fn request() -> GenerationRequest {
    GenerationRequest {
        source: Source::OpenRouter,
        model: ModelOption {
            ollama_name: String::new(),
            openrouter_name: "openrouter/vendor/selected:free".into(),
            llmfit_name: String::new(),
        },
        timeout: Duration::from_secs(4),
        install_missing: false,
    }
}

#[test]
fn catalogue_requires_vision_and_zero_pricing() {
    let v = json!({"id":"vendor/free","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0"},"supported_parameters":[]});
    let m = CatalogModel::from_api(&v).unwrap();
    assert!(m.image_input && m.free);
    for v in [
        json!({"id":"paid-image","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0","image":"0.0001"}}),
        json!({"id":"paid-request","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0","request":"0.01"}}),
        json!({"id":"text-only","architecture":{"input_modalities":["text"]},"pricing":{"prompt":"0","completion":"0"}}),
    ] {
        let m = CatalogModel::from_api(&v).unwrap();
        if v["id"] == "text-only" {
            assert!(!m.image_input);
        } else {
            assert!(!m.free, "paid model advertised as free: {}", m.id);
        }
    }
}

#[tokio::test]
async fn http_429_walks_other_vendor_before_sibling_and_reports_answerer() {
    let (_dir, img) = image();
    let catalog=json!({"data":[
        {"id":"vendor/selected:free","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0"}},
        {"id":"other/model:free","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0"},"supported_parameters":[]},
        {"id":"vendor/sibling:free","architecture":{"input_modalities":["text","image"]},"pricing":{"prompt":"0","completion":"0"}}
    ]}).to_string();
    let server = common::http::Server::start(vec![
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 429,
            body: "busy".into(),
        },
        common::http::Reply {
            path: "/models".into(),
            status: 200,
            body: catalog,
        },
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 200,
            body: completion("other/model:free"),
        },
    ]);
    let got = client(server.url())
        .openrouter_generate(&img, &request(), None)
        .await
        .unwrap();
    assert_eq!(got.model_used, "other/model:free");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["body"]["model"], "vendor/selected:free");
    assert_eq!(requests[2]["body"]["model"], "other/model:free");
    assert!(requests[2]["body"].get("response_format").is_none());
    assert!(
        requests[0]["body"]
            .to_string()
            .contains("data:image/png;base64,")
    );
}

#[tokio::test]
async fn schema_rejection_retries_once_prompt_shaped_then_memoizes() {
    let (_dir, img) = image();
    let server = common::http::Server::start(vec![
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 400,
            body: r#"{"error":{"message":"response_format is not supported"}}"#.into(),
        },
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 200,
            body: completion("vendor/model"),
        },
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 200,
            body: completion("vendor/model"),
        },
    ]);
    let c = client(server.url());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    c.openrouter_attempt(&img, "vendor/model", true, deadline)
        .await
        .unwrap();
    c.openrouter_attempt(&img, "vendor/model", true, deadline)
        .await
        .unwrap();
    let req = server.requests();
    assert_eq!(req.len(), 3);
    assert!(req[0]["body"].get("response_format").is_some());
    assert!(req[1]["body"].get("response_format").is_none());
    assert!(req[1]["body"].to_string().contains("title"));
    assert!(req[2]["body"].get("response_format").is_none());
}

#[tokio::test]
async fn response_model_alias_never_becomes_paid_preference() {
    let (_dir, img) = image();
    let server = common::http::Server::start(vec![
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 200,
            body: completion("vendor/canonical-without-free"),
        },
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 200,
            body: completion("vendor/canonical-without-free"),
        },
    ]);
    let c = client(server.url());
    let first = c.openrouter_generate(&img, &request(), None).await.unwrap();
    assert_eq!(first.model_used, "vendor/selected:free");
    c.openrouter_generate(&img, &request(), Some(&first.model_used))
        .await
        .unwrap();
    assert!(
        server
            .requests()
            .iter()
            .all(|r| r["body"]["model"] == "vendor/selected:free")
    );
}

#[tokio::test]
async fn unrelated_bad_request_does_not_trigger_schema_retry() {
    let (_dir, img) = image();
    let server = common::http::Server::start(vec![common::http::Reply {
        path: "/chat/completions".into(),
        status: 400,
        body: r#"{"error":{"message":"invalid prompt"}}"#.into(),
    }]);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    assert!(
        client(server.url())
            .openrouter_attempt(&img, "vendor/model", true, deadline)
            .await
            .is_err()
    );
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn successful_selected_model_does_not_fetch_catalogue() {
    let (_dir, img) = image();
    let server = common::http::Server::start(vec![common::http::Reply {
        path: "/chat/completions".into(),
        status: 200,
        body: completion("vendor/model"),
    }]);
    client(server.url())
        .openrouter_generate(&img, &request(), None)
        .await
        .unwrap();
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn malformed_rescue_catalogue_preserves_original_model_error() {
    let (_dir, img) = image();
    let server = common::http::Server::start(vec![
        common::http::Reply {
            path: "/chat/completions".into(),
            status: 429,
            body: "original busy".into(),
        },
        common::http::Reply {
            path: "/models".into(),
            status: 200,
            body: "{bad".into(),
        },
    ]);
    let err = client(server.url())
        .openrouter_generate(&img, &request(), None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("429"), "{err}");
}

#[tokio::test]
async fn body_reads_are_inside_total_deadline() {
    // A server that delays the response body proves the deadline covers body consumption.
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Instant,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(3);
        let (mut s, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return,
            }
        };
        s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut b = [0; 2048];
        let _ = s.read(&mut b);
        let body = b"{\"choices\":[{\"message\":{\"content\":\"x\"}}]}";
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = s.write_all(head.as_bytes());
        thread::sleep(Duration::from_millis(300));
        let _ = s.write_all(body);
    });
    let (_dir, img) = image();
    let c = client(&format!("http://{addr}"));
    let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
    assert!(
        c.openrouter_attempt(&img, "vendor/model", false, deadline)
            .await
            .is_err()
    );
    worker.join().unwrap();
}

#[allow(dead_code)]
fn _api_type(_: Value) {}
