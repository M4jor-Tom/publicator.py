use publicator::config::{Config, Paths, PromptConfig, PromptGrammar, validate_publications};
use publicator::model::{Publications, validate_metadata};
use serde_json::json;
use std::fs;

#[test]
fn missing_config_uses_defaults_and_bad_toml_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert!(config.tiers.is_empty());
    assert!(config.galleries.is_empty());
    assert!(config.publicable.is_empty());
    assert_eq!(config.schedule.len(), 0);
    fs::write(dir.path().join("publicator.toml"), "[broken\n").unwrap();
    assert!(
        Config::load(dir.path())
            .unwrap_err()
            .to_string()
            .contains("publicator.toml")
    );
}

#[test]
fn prompt_grammar_uses_configured_captures_and_digest() {
    let grammar = PromptGrammar::compile(&PromptConfig {
        repo: "archive".into(),
        filename: r"img-(?P<version>[a-f0-9]+)\.png".into(),
        version_hash: "sha256".into(),
    })
    .unwrap();
    assert_eq!(
        grammar.digest(b"abc").unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(grammar.identity("img-a.png").unwrap().unwrap().version, "a");
    assert!(grammar.identity("prefix-img-a.png").unwrap().is_none());
    let sha1 = PromptGrammar::compile(&PromptConfig {
        repo: "r".into(),
        filename: r"(?P<version>.+)".into(),
        version_hash: "sha1".into(),
    })
    .unwrap();
    assert_eq!(
        sha1.digest(b"abc").unwrap(),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

#[test]
fn publication_validation_retains_extensions_zero_price_and_rejects_bad_url_or_tier() {
    let make = |url: &str, tier: &str| json!([{"uuid":"00000000-0000-4000-8000-000000000001", "state":"PICKED", "title":"t", "description":"d", "submissionTimestamp":1, "files":[], "unknown":{"kept":true}, "apparitions":[{"platformName":"deviantart","state":"unpublished","url":url,"priceIfNotFree":0,"tiers":[tier]}]}]);
    let good = make("https://example.com", "allowed");
    let rows: Publications = serde_json::from_value(good.clone()).unwrap();
    let cfg = Config {
        tiers: vec!["allowed".into()],
        ..Config::default()
    };
    validate_publications(&rows, &cfg).unwrap();
    assert_eq!(rows[0]["unknown"]["kept"], true);
    assert_eq!(rows[0]["apparitions"][0]["priceIfNotFree"], 0);
    let bad_url: Publications = serde_json::from_value(make("file:///tmp/x", "allowed")).unwrap();
    assert!(validate_publications(&bad_url, &cfg).is_err());
    let obsolete: Publications = serde_json::from_value(json!([{"uuid":"00000000-0000-4000-8000-000000000001","state":"PICKED","title":"t","description":"d","submissionTimestamp":1,"files":[],"apparitions":[{"platformName":"deviantart","state":"unpublished","tier":"x"}]}])).unwrap();
    assert!(validate_publications(&obsolete, &cfg).is_err());
    let bad_tier: Publications =
        serde_json::from_value(make("https://example.com", "other")).unwrap();
    assert!(validate_publications(&bad_tier, &cfg).is_err());
}

#[test]
fn paths_resolve_relative_json_under_data_dir_and_default_to_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(Some(dir.path()), Some(std::path::Path::new("other.json"))).unwrap();
    assert_eq!(paths.json, dir.path().join("other.json"));
    assert_eq!(
        Paths::resolve(None, None).unwrap().json,
        std::env::current_dir().unwrap().join("publications.json")
    );
}

#[test]
fn metadata_requires_complete_unmarked_title_with_unicode_limit() {
    assert!(validate_metadata("valid", "description").is_ok());
    assert!(validate_metadata("  ", "description").is_err());
    assert!(validate_metadata("Title [[PUBLICATOR:TODO_GENERATE]]", "description").is_err());
    assert!(validate_metadata(&"🦀".repeat(51), "description").is_err());
}
