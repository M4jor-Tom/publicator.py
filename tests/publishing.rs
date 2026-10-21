mod common;
use common::{Fixture, row};
use publicator::{
    application::Application,
    metadata::{Endpoints, MetadataClient},
    publishing::{PublishRequest, Publisher},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

fn app(f: &Fixture) -> Application {
    Application::new(
        Arc::new(
            publicator::store::Store::new(
                publicator::config::Paths::resolve(Some(f.root.path()), None).unwrap(),
                publicator::config::Config::default(),
            )
            .unwrap(),
        ),
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints {
                openrouter: "http://127.0.0.1:1".into(),
                ollama: "http://127.0.0.1:1".into(),
            },
            "llmfit".into(),
        ),
        || 0,
    )
}
#[tokio::test]
async fn historical_only_publications_do_not_start_worker() {
    let f = Fixture::new();
    f.write(json!([row(1, "APPEARED")]));
    let app = app(&f);
    let publisher = Publisher::new(
        PathBuf::from("/does/not/exist"),
        PathBuf::from("/does/not/exist"),
    );
    let report = publisher
        .publish(
            &app,
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.published, 0);
    assert_eq!(report.failed, 0);
    assert_eq!(report.unknown, 0);
    assert_eq!(report.unattempted, 0);
}
#[tokio::test]
async fn publishing_requires_configured_publisher_on_application() {
    let f = Fixture::new();
    let result = app(&f)
        .publish(
            PublishRequest {
                uuids: None,
                headless: false,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert!(result.is_err());
}
