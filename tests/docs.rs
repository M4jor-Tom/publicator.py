use std::path::{Path, PathBuf};

fn concrete_repo_paths(markdown: &str) -> Vec<&str> {
    markdown
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|path| path.split(':').next().unwrap_or(path))
        .filter(|path| {
            (path.starts_with("src/")
                || path.starts_with("tests/")
                || path.starts_with("docs/")
                || path.starts_with(".claude/"))
                && (Path::new(path).extension().is_some() || path.ends_with('/'))
                && !path
                    .chars()
                    .any(|ch| matches!(ch, '{' | '}' | '*' | '[' | ']' | '?'))
        })
        .collect()
}

fn missing_repo_paths(markdown: &str, root: &Path) -> Vec<PathBuf> {
    concrete_repo_paths(markdown)
        .into_iter()
        .map(|path| root.join(path))
        .filter(|path| !path.exists())
        .collect()
}

fn maintained_docs(root: &Path) -> Vec<PathBuf> {
    let mut docs = vec![root.join("AGENTS.md"), root.join("docs/README.md")];
    docs.extend(
        std::fs::read_dir(root.join("docs/features"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md")),
    );
    docs
}

#[test]
fn maintained_documentation_references_existing_concrete_repo_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for doc in maintained_docs(root) {
        let text = std::fs::read_to_string(&doc).unwrap();
        let missing = missing_repo_paths(&text, root);
        assert!(
            missing.is_empty(),
            "{} names missing paths: {missing:?}",
            doc.display()
        );
    }
}

#[test]
fn maintained_docs_keep_search_and_application_owned_metadata_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let guidance = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    let gallery = std::fs::read_to_string(root.join("docs/features/gallery-ui.md")).unwrap();
    let ai = std::fs::read_to_string(root.join("docs/features/ai-metadata.md")).unwrap();
    let config = std::fs::read_to_string(root.join("docs/features/config.md")).unwrap();

    assert!(guidance.contains("nix run .#ui"));
    assert!(gallery.contains("runtime subset"));
    assert!(gallery.contains("Empty/0, 25, 50, 75, or 100"));
    assert!(gallery.contains("defaults to 25"));
    assert!(gallery.contains("all configured `publicable` paths selected"));
    assert!(config.contains("publicable") && config.contains("runtime"));
    assert!(ai.contains("both title and description"));
    assert!(ai.contains("publication remains SCHEDULED and retryable"));
}

#[test]
fn documentation_path_check_ignores_groups_and_globs_but_catches_missing_literals() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let missing = missing_repo_paths(
        "`src/{mod,error}.rs` `docs/features/*.md` `src/missing-concrete-path.rs`",
        root,
    );
    assert_eq!(missing, [root.join("src/missing-concrete-path.rs")]);
}

#[test]
fn maintained_path_check_excludes_historical_roadmap_and_adr_paths() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let docs = maintained_docs(root);
    assert!(docs.contains(&root.join("docs/features/config.md")));
    assert!(!docs.contains(&root.join("docs/ROADMAP.md")));
    assert!(
        !docs
            .iter()
            .any(|path| path.starts_with(root.join("docs/adr")))
    );

    let roadmap = std::fs::read_to_string(root.join("docs/ROADMAP.md")).unwrap();
    let adr = std::fs::read_to_string(
        root.join("docs/adr/0009-separate-interfaces-application-operations-and-store.md"),
    )
    .unwrap();
    assert!(roadmap.contains("src/publicator/webui/{page,server,calendar_view,prompt_view}.py"));
    assert!(adr.contains("src/publicator/webui/server.py"));
}
