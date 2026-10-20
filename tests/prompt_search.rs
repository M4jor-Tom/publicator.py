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
fn audit_counts_parsed_exact_and_distinct_versions() {
    let (dir, cfg) = setup();
    let digest = cfg::digest(b"A Moonlit Garden");
    image(
        &dir.path().join("art"),
        b"one",
        &format!("prompt.txt_{digest}.png"),
    );
    image(&dir.path().join("art"), b"two", "plain.png");
    let counts = audit(dir.path(), &cfg, Some(&archive(dir.path(), &cfg))).unwrap();
    assert_eq!(
        (
            counts.files,
            counts.parsed,
            counts.exact,
            counts.nearest,
            counts.unknown
        ),
        (2, 1, 1, 0, 0)
    );
    assert_eq!((counts.exact_versions, counts.nearest_versions), (1, 0));
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
