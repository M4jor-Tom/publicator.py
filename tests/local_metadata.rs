use publicator::{
    images::PreparedImage,
    metadata::{
        Endpoints, GenerationRequest, MetadataClient, ModelOption, Source, VISION_MODELS,
        consent_required, title_description_schema,
    },
};
use serde_json::{Value, json};
use std::time::Duration;

mod common;
use common::http::{Reply, Server};

fn model_option() -> ModelOption {
    VISION_MODELS[0].clone()
}
fn local_client_for_test(server: &Server) -> (MetadataClient, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("llmfit");
    let report =
        json!({"models":[{"name":"google/gemma-4-26B-A4B-it","fit_level":"Good","notes":[]}]});
    std::fs::write(
        &executable,
        format!("#!/bin/sh\nprintf '%s' '{}'\n", report),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    (
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints {
                openrouter: "http://127.0.0.1:1".into(),
                ollama: server.url().into(),
            },
            executable,
        ),
        dir,
    )
}
fn reply(path: &str, status: u16, body: Value) -> Reply {
    Reply {
        path: path.into(),
        status,
        body: body.to_string(),
    }
}

#[tokio::test]
async fn model_is_not_pulled_without_consent() {
    let server = Server::start(vec![reply("/api/tags", 200, json!({"models":[]}))]);
    let (client, _llmfit) = local_client_for_test(&server);
    let result = client
        .prepare_local(
            &model_option(),
            false,
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await;
    assert!(result.is_err());
    assert!(server.requests().iter().all(|r| r["path"] != "/api/pull"));
}

#[tokio::test]
async fn failed_pull_does_not_proceed_to_generation() {
    let server = Server::start(vec![
        reply("/api/tags", 200, json!({"models":[]})),
        reply("/api/pull", 500, json!({"error":"no space"})),
    ]);
    let (client, _llmfit) = local_client_for_test(&server);
    let result = client
        .prepare_local(
            &model_option(),
            true,
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["/api/tags", "/api/pull"]
    );
}

#[tokio::test]
async fn local_chat_sends_schema_and_image_and_parses_pair() {
    let server = Server::start(vec![
        reply("/api/tags", 200, json!({"models":[]})),
        reply("/api/pull", 200, json!({"status":"success"})),
        reply(
            "/api/chat",
            200,
            json!({"message":{"content":"{\"title\":\"A title\",\"description\":\"A scene unfolds.\"}"}}),
        ),
    ]);
    let (client, _llmfit) = local_client_for_test(&server);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("art.png");
    std::fs::write(&path, [0xfb, 0xff]).unwrap();
    let generated = client
        .generate(
            &PreparedImage {
                path,
                mime: "image/png".into(),
            },
            &GenerationRequest {
                source: Source::Local,
                model: model_option(),
                timeout: Duration::from_secs(3),
                install_missing: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(generated.metadata.title, "A title");
    let req = server.requests();
    assert_eq!(
        req[1]["body"],
        json!({"model":"gemma4:26b-a4b","stream":false})
    );
    let body = &req[2]["body"];
    assert_eq!(body["model"], "gemma4:26b-a4b");
    assert_eq!(body["stream"], false);
    assert_eq!(body["format"], title_description_schema());
    assert_eq!(body["messages"][0]["images"], json!(["+/8="]));
}

#[tokio::test]
async fn local_chat_rejects_malformed_or_empty_metadata() {
    for content in ["not json", "{\"title\":\"\",\"description\":\"text\"}"] {
        let server = Server::start(vec![
            reply(
                "/api/tags",
                200,
                json!({"models":[{"name":"gemma4:26b-a4b"}]}),
            ),
            reply("/api/chat", 200, json!({"message":{"content":content}})),
        ]);
        let (client, _llmfit) = local_client_for_test(&server);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.png");
        std::fs::write(&path, b"x").unwrap();
        assert!(
            client
                .local_generate(
                    &PreparedImage {
                        path,
                        mime: "image/png".into()
                    },
                    &GenerationRequest {
                        source: Source::Local,
                        model: model_option(),
                        timeout: Duration::from_secs(2),
                        install_missing: false
                    }
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn llmfit_good_too_tight_and_unassessable_verdicts_are_distinct() {
    for (stdout, exit, expected, level) in [
        (
            r#"{"models":[{"name":"google/gemma-4-26B-A4B-it","fit_level":"Good"}]}"#,
            true,
            true,
            "Good",
        ),
        (
            r#"{"models":[{"name":"google/gemma-4-26B-A4B-it","fit_level":"Too Tight"}]}"#,
            true,
            false,
            "Too Tight",
        ),
        ("not json", true, false, "Unavailable"),
        ("", false, false, "Unavailable"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("llmfit");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\n{}\n",
                stdout,
                if exit { "exit 0" } else { "exit 1" }
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let client = MetadataClient::new(reqwest::Client::new(), Endpoints::default(), exe);
        let fit = client.local_fit(&model_option()).await.unwrap();
        assert_eq!(fit.fit, expected);
        assert_eq!(fit.level, level);
    }
}

#[test]
fn consent_is_only_required_for_a_fitting_absent_model() {
    let missing = publicator::metadata::LocalStatus {
        fit: publicator::metadata::Fit {
            fit: true,
            level: "Good".into(),
            reason: String::new(),
        },
        installed: false,
        available: true,
    };
    assert!(consent_required(&missing));
    let installed = publicator::metadata::LocalStatus {
        installed: true,
        ..missing.clone()
    };
    assert!(!consent_required(&installed));
    let unavailable = publicator::metadata::LocalStatus {
        fit: publicator::metadata::Fit {
            fit: false,
            level: "Too Tight".into(),
            reason: "ram".into(),
        },
        installed: false,
        available: false,
    };
    assert!(!consent_required(&unavailable));
}
