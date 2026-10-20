use publicator::{
    images::PreparedImage,
    metadata::{
        GenerationRequest, Metadata, ModelOption, Source, VISION_MODELS, available_sources,
        image_data_url, metadata_prompt, parse_metadata, title_description_schema,
    },
};
use serde_json::json;
use std::time::Duration;

mod common;

#[test]
fn parses_fenced_pair_but_not_partial_metadata() {
    assert_eq!(
        parse_metadata("```json\n{\"title\":\"T\",\"description\":\"D\"}\n```")
            .unwrap()
            .title,
        "T"
    );
    assert!(parse_metadata("{\"title\":\"T\"}").is_err());
    assert!(serde_json::from_str::<Source>("\"claude\"").is_err());
}

#[test]
fn metadata_requires_string_pair_and_valid_title() {
    for input in [
        r#"{"title":7,"description":"D"}"#,
        r#"{"title":"  ","description":"D"}"#,
        r#"{"title":"[[PUBLICATOR:TODO_GENERATE]]","description":"D"}"#,
        &format!(r#"{{"title":"{}","description":"D"}}"#, "x".repeat(51)),
    ] {
        assert!(parse_metadata(input).is_err(), "accepted {input}");
    }
}

#[test]
fn metadata_contract_has_two_sources_and_explicit_model_mappings() {
    assert_eq!(available_sources(), &[Source::OpenRouter, Source::Local]);
    assert_eq!(
        serde_json::to_string(&Source::OpenRouter).unwrap(),
        "\"openrouter\""
    );
    assert_eq!(serde_json::to_string(&Source::Local).unwrap(), "\"local\"");
    assert_eq!(VISION_MODELS.len(), 2);
    assert_eq!(VISION_MODELS[0].ollama_name, "gemma4:26b-a4b");
    assert!(title_description_schema().is_object());
    assert!(!metadata_prompt().is_empty());
    let _: Option<Metadata> = None;
    let _ = GenerationRequest {
        source: Source::Local,
        model: ModelOption {
            ollama_name: String::new(),
            openrouter_name: String::new(),
            llmfit_name: String::new(),
        },
        timeout: Duration::from_secs(1),
        install_missing: false,
    };
}

#[test]
fn image_data_url_encodes_bytes_with_the_prepared_image_mime() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("image.jpg");
    std::fs::write(&path, [0xfb, 0xff]).unwrap();
    let image = PreparedImage {
        path,
        mime: "image/jpeg".into(),
    };

    assert_eq!(
        image_data_url(&image).unwrap(),
        "data:image/jpeg;base64,+/8="
    );
}

#[tokio::test]
async fn loopback_http_fixture_serves_scripted_response_and_captures_body() {
    let server = common::http::Server::start(vec![common::http::Reply {
        path: "/probe".into(),
        status: 200,
        body: "ok".into(),
    }]);
    let response = reqwest::Client::new()
        .post(format!("{}{}/probe", server.url(), ""))
        .json(&json!({"safe": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    assert_eq!(
        server.requests(),
        vec![json!({"path":"/probe", "body":{"safe":true}})]
    );
}
