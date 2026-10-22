use publicator::{
    config::{Config, PromptConfig},
    images::Search,
    prompts::{PromptArchive, audit, search},
};
use std::{path::Path, process::Command};

fn setup() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("prompts");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        )
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.invalid"]);
    let file = repo.join("lineage/prompt.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "A Moonlit Garden").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "snapshot"]);
    std::fs::create_dir(dir.path().join("art")).unwrap();
    let cfg = Config {
        publicable: vec!["art".into()],
        prompts: Some(PromptConfig {
            repo: "prompts".into(),
            filename: r"(?P<lineage>[^_]+)_(?P<version>[a-f0-9]+)\.png".into(),
            version_hash: "sha256".into(),
        }),
        ..Config::default()
    };
    (dir, cfg)
}
fn image(dir: &Path, text: &[u8], basename: &str) {
    std::fs::write(dir.join(basename), text).unwrap();
}
fn archive(dir: &Path, cfg: &Config) -> PromptArchive {
    PromptArchive::new(dir, cfg.prompts.as_ref().unwrap()).unwrap()
}

#[test]
fn prompt_text_search_returns_only_exact_text_matches() {
    let (dir, cfg) = setup();
    let digest = cfg::digest(b"A Moonlit Garden");
    let art = dir.path().join("art");
    image(&art, b"one", &format!("prompt.txt_{digest}.png"));
    image(&art, b"two", "plain.png");
    let store = store(&dir, cfg.clone());
    let result = search(
        &store,
        &Search {
            limit: 25,
            paths: vec!["art".into()],
        },
        Some(&archive(dir.path(), &cfg)),
        Some("moonlit"),
        None,
    )
    .unwrap();
    assert_eq!(result.exact.len(), 1);
    assert_eq!(
        result.exact[0].path,
        art.join(format!("prompt.txt_{digest}.png"))
    );
    assert!(result.maybe.is_empty());
    assert_eq!(result.skipped, 0);
    assert!(!result.truncated);
}

#[test]
fn nearest_prompt_text_is_skipped_but_explicit_lineage_is_a_maybe_hit() {
    let (dir, cfg) = setup();
    let missing = "f".repeat(64);
    let art = dir.path().join("art");
    image(&art, b"unarchived", &format!("prompt.txt_{missing}.png"));
    let store = store(&dir, cfg.clone());
    let archive = archive(dir.path(), &cfg);
    let base = Search {
        limit: 25,
        paths: vec!["art".into()],
    };
    let skipped = search(&store, &base, Some(&archive), Some("moonlit"), None).unwrap();
    assert!(skipped.exact.is_empty() && skipped.maybe.is_empty());
    assert_eq!(skipped.skipped, 1);
    let maybe = search(
        &store,
        &base,
        Some(&archive),
        Some("moonlit"),
        Some("lineage/prompt.txt"),
    )
    .unwrap();
    assert!(maybe.exact.is_empty());
    assert_eq!(maybe.maybe.len(), 1);
    assert_eq!(maybe.skipped, 0);
}

#[test]
fn search_caps_matching_groups_before_hashing_and_reports_truncation() {
    let (dir, cfg) = setup();
    let art = dir.path().join("art");
    let digest = cfg::digest(b"A Moonlit Garden");
    for index in 0..205 {
        image(
            &art,
            format!("art-{index}").as_bytes(),
            &format!("prompt-{index}_{digest}.png"),
        );
    }
    let store = store(&dir, cfg.clone());
    let archive = archive(dir.path(), &cfg);
    let result = search(
        &store,
        &Search {
            limit: 100,
            paths: vec!["art".into()],
        },
        Some(&archive),
        None,
        None,
    )
    .unwrap();
    assert_eq!(result.exact.len(), 100);
    assert!(result.truncated);
}

#[test]
fn feature_off_search_is_inert_and_audit_counts_files_only() {
    let (dir, _) = setup();
    image(&dir.path().join("art"), b"x", "plain.png");
    let cfg = Config {
        publicable: vec!["art".into()],
        ..Config::default()
    };
    let result = search(
        &store(&dir, cfg.clone()),
        &Search {
            limit: 25,
            paths: vec!["art".into()],
        },
        None,
        Some("anything"),
        None,
    )
    .unwrap();
    assert!(result.exact.is_empty() && result.maybe.is_empty());
    assert_eq!(result.skipped, 0);
    assert!(!result.truncated);
    let counts = audit(dir.path(), &cfg, None).unwrap();
    assert_eq!(counts.files, 1);
    assert_eq!(
        counts.parsed + counts.exact + counts.nearest + counts.unknown,
        0
    );
}

#[test]
fn prompt_audit_counts_each_resolution_and_prints_parsed_rate_first() {
    let (dir, cfg) = setup();
    let digest = cfg::digest(b"A Moonlit Garden");
    let art = dir.path().join("art");
    image(&art, b"one", &format!("prompt.txt_{digest}.png"));
    image(&art, b"two", &format!("prompt.txt_{}.png", "f".repeat(64)));
    image(
        &art,
        b"three",
        &format!("unknown.txt_{}.png", "e".repeat(64)),
    );
    image(&art, b"four", "plain.png");
    let counts = audit(dir.path(), &cfg, Some(&archive(dir.path(), &cfg))).unwrap();
    assert_eq!(
        (
            counts.files,
            counts.parsed,
            counts.exact,
            counts.exact_versions,
            counts.nearest,
            counts.nearest_versions,
            counts.unknown
        ),
        (4, 3, 1, 1, 1, 1, 1)
    );
    std::fs::write(
        dir.path().join("publicator.toml"),
        r#"
publicable = ["art"]
[prompts]
repo = "prompts"
filename = '(?P<lineage>[^_]+)_(?P<version>[a-f0-9]+)\.png'
version_hash = "sha256"
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .args(["prompt-audit", "--data-dir", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.lines().next().unwrap().starts_with("parsed: 3 of 4"));
    assert!(report.contains("exact: 1 images / 1 versions"));
    assert!(report.contains("nearest: 1 images / 1 versions"));
    assert!(report.contains("unknown: 1"));
}

#[test]
fn broken_optional_git_archive_is_unknown_for_audit_and_visible_for_prompt_search() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("prompts")).unwrap();
    let art = dir.path().join("art");
    std::fs::create_dir(&art).unwrap();
    let prompt_config = PromptConfig {
        repo: "prompts".into(),
        filename: r"(?P<lineage>[^_]+)_(?P<version>[a-f0-9]+)\.png".into(),
        version_hash: "sha256".into(),
    };
    let cfg = Config {
        publicable: vec!["art".into()],
        prompts: Some(prompt_config.clone()),
        ..Config::default()
    };
    image(
        &art,
        b"no archive available",
        &format!("prompt.txt_{}.png", cfg::digest(b"not archived")),
    );
    let archive = PromptArchive::from_config(dir.path(), &prompt_config)
        .unwrap()
        .unwrap();

    let counts = audit(dir.path(), &cfg, Some(&archive)).unwrap();
    assert_eq!(
        (
            counts.files,
            counts.parsed,
            counts.exact,
            counts.nearest,
            counts.unknown
        ),
        (1, 1, 0, 0, 1)
    );

    let store = store(&dir, cfg);
    let error = search(
        &store,
        &Search {
            limit: 25,
            paths: vec!["art".into()],
        },
        Some(&archive),
        Some("moonlit"),
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("prompt archive unavailable"));
}

#[test]
fn audit_without_prompt_config_counts_no_files_as_all_zeroes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("art")).unwrap();
    let config = Config {
        publicable: vec!["art".into()],
        ..Config::default()
    };
    let counts = audit(dir.path(), &config, None).unwrap();
    assert_eq!(
        (
            counts.files,
            counts.parsed,
            counts.exact,
            counts.exact_versions,
            counts.nearest,
            counts.nearest_versions,
            counts.unknown
        ),
        (0, 0, 0, 0, 0, 0, 0)
    );
}

fn store(dir: &tempfile::TempDir, config: Config) -> publicator::store::Store {
    let paths = publicator::config::Paths::resolve(Some(dir.path()), None).unwrap();
    publicator::store::Store::new(paths, config).unwrap()
}
mod cfg {
    use openssl::hash::{Hasher, MessageDigest};
    pub fn digest(text: &[u8]) -> String {
        let mut h = Hasher::new(MessageDigest::sha256()).unwrap();
        h.update(text).unwrap();
        h.finish()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
