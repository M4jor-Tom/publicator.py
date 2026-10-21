mod common;
use common::{Fixture, id, row};
use publicator::{
    application::Application,
    metadata::{Endpoints, GenerationRequest, MetadataClient, Source, VISION_MODELS},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

fn request(source: Source) -> GenerationRequest {
    GenerationRequest {
        source,
        model: VISION_MODELS[0].clone(),
        timeout: Duration::from_secs(2),
        install_missing: false,
    }
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
