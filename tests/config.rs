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
    for (name, want) in [
        (
            "sha512",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            "sha3_256",
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532",
        ),
        (
            "blake2b",
            "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923",
        ),
    ] {
        let g = PromptGrammar::compile(&PromptConfig {
            repo: "r".into(),
            filename: r"(?P<version>.+)".into(),
            version_hash: name.into(),
        })
        .unwrap();
        assert_eq!(&g.digest(b"abc").unwrap(), want, "{name}");
    }
    let unknown = PromptGrammar::compile(&PromptConfig {
        repo: "r".into(),
        filename: r"(?P<version>.+)".into(),
        version_hash: "not-a-real-digest".into(),
    })
    .err()
    .unwrap();
    assert!(unknown.to_string().contains("version_hash"));
}

#[test]
fn prompt_grammar_uses_pcre_unicode_lookbehind_and_named_backreference() {
    let g = PromptGrammar::compile(&PromptConfig {
        repo: "r".into(),
        filename: r"(?<=^)(?P<lineage>\p{Greek}*)-(?P<version>\p{Greek}+)-(?P=lineage)".into(),
        version_hash: "sha256".into(),
    })
    .unwrap();
    assert_eq!(g.identity("αβ-γ-αβ").unwrap().unwrap().version, "γ");
    assert_eq!(g.identity("-γ-").unwrap().unwrap().lineage, None);
    assert!(g.identity("xαβ-γ-αβ").unwrap().is_none());
}

#[test]
fn empty_prompts_disable_prompt_grammar_and_incomplete_prompts_name_field() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("publicator.toml"), "[prompts]\n").unwrap();
    assert!(Config::load(dir.path()).unwrap().prompts.is_none());
    fs::write(
        dir.path().join("publicator.toml"),
        "[prompts]\nrepo = 'archive'\n",
    )
    .unwrap();
    let err = Config::load(dir.path()).unwrap_err().to_string();
    assert!(err.contains("prompts") && err.contains("filename"));
}

#[test]
fn missing_publications_file_validates_as_empty_but_malformed_json_fails() {
    let dir = tempfile::tempdir().unwrap();
    let binary = env!("CARGO_BIN_EXE_publicator");
    let missing = std::process::Command::new(binary)
        .args(["validate", "--data-dir", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        missing.status.success(),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    assert_eq!(String::from_utf8(missing.stdout).unwrap().trim(), "Valid");
    fs::write(dir.path().join("publications.json"), "{bad").unwrap();
    let malformed = std::process::Command::new(binary)
        .args(["validate", "--data-dir", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!malformed.status.success());
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("JSON error"));
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
