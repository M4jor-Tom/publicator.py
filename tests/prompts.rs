use publicator::{
    config::{PromptConfig, PromptGrammar},
    prompts::{PromptArchive, PromptMatch},
};
use std::{path::Path, process::Command};

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        )
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.invalid"]);
    dir
}
fn commit(dir: &std::path::Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .status()
                .unwrap()
                .success()
        )
    };
    git(&["add", "--", path]);
    git(&["commit", "-qm", "snapshot"]);
}
fn config() -> PromptConfig {
    PromptConfig {
        repo: ".".into(),
        filename: r"(?P<lineage>[^_]+)_(?P<version>[a-f0-9]+)\.png".into(),
        version_hash: "sha256".into(),
    }
}

#[test]
fn superseded_filename_resolves_old_content_not_head() {
    let tmp = repo();
    commit(tmp.path(), "scene.txt", "old prompt");
    commit(tmp.path(), "scene.txt", "new prompt");
    let grammar = PromptGrammar::compile(&config()).unwrap();
    let digest = grammar.digest(b"old prompt").unwrap();
    let archive = PromptArchive::new(tmp.path(), &config()).unwrap();
    match archive
        .resolve_path(&tmp.path().join(format!("scene.txt_{digest}.png")))
        .unwrap()
    {
        PromptMatch::Exact(v) => assert_eq!(v.text, "old prompt"),
        other => panic!("expected archived exact version, got {other:?}"),
    }
}

#[test]
fn nearest_keeps_all_ranked_lineages_and_versions_from_git_history() {
    let tmp = repo();
    commit(tmp.path(), "matching/path.txt", "old");
    commit(tmp.path(), "matching/path.txt", "new");
    commit(tmp.path(), "other/path.txt", "other");
    let archive = PromptArchive::new(tmp.path(), &config()).unwrap();
    let version = PromptGrammar::compile(&config())
        .unwrap()
        .digest(b"not archived")
        .unwrap();
    match archive
        .resolve_path(Path::new(&format!("matching/path.txt_{version}.png")))
        .unwrap()
    {
        PromptMatch::Nearest(paths) => {
            assert_eq!(paths.len(), 2);
            assert_eq!(paths[0].path, "matching/path.txt");
            assert_eq!(
                paths[0]
                    .versions
                    .iter()
                    .map(|v| v.text.as_str())
                    .collect::<Vec<_>>(),
                ["new", "old"]
            );
        }
        other => panic!("expected nearest histories, got {other:?}"),
    }
}

#[test]
fn exact_version_collapses_duplicate_paths_and_uses_content_digest() {
    let tmp = repo();
    commit(tmp.path(), "a/same.txt", "same prompt");
    commit(tmp.path(), "b/same.txt", "same prompt");
    let archive = PromptArchive::new(tmp.path(), &config()).unwrap();
    let digest = PromptGrammar::compile(&config())
        .unwrap()
        .digest(b"same prompt")
        .unwrap();
    match archive
        .resolve_path(Path::new(&format!("same.txt_{digest}.png")))
        .unwrap()
    {
        PromptMatch::Exact(version) => assert_eq!(version.paths, ["a/same.txt", "b/same.txt"]),
        other => panic!("expected exact version, got {other:?}"),
    }
}

#[test]
fn head_probe_is_throttled_for_two_seconds_then_refreshes() {
    let tmp = repo();
    commit(tmp.path(), "scene.txt", "first");
    let archive = PromptArchive::new(tmp.path(), &config()).unwrap();
    let grammar = PromptGrammar::compile(&config()).unwrap();
    let first = grammar.digest(b"first").unwrap();
    assert!(matches!(
        archive
            .resolve_path(&tmp.path().join(format!("scene.txt_{first}.png")))
            .unwrap(),
        PromptMatch::Exact(_)
    ));
    let second = grammar.digest(b"second").unwrap();
    commit(tmp.path(), "scene.txt", "second");
    let path = tmp.path().join(format!("scene.txt_{second}.png"));
    assert!(matches!(
        archive.resolve_path(&path).unwrap(),
        PromptMatch::Nearest(_)
    ));
    std::thread::sleep(std::time::Duration::from_millis(2100));
    assert!(matches!(
        archive.resolve_path(&path).unwrap(),
        PromptMatch::Exact(_)
    ));
}

#[test]
fn unreachable_blob_is_exact_but_has_no_commit_or_path_metadata() {
    let tmp = repo();
    let oid = Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    let mut child = oid;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"unreachable prompt")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let digest = PromptGrammar::compile(&config())
        .unwrap()
        .digest(b"unreachable prompt")
        .unwrap();
    let archive = PromptArchive::new(tmp.path(), &config()).unwrap();
    match archive
        .resolve_path(Path::new(&format!("scene.txt_{digest}.png")))
        .unwrap()
    {
        PromptMatch::Exact(version) => {
            assert_eq!(version.text, "unreachable prompt");
            assert_eq!(version.committed, 0);
            assert!(version.paths.is_empty());
        }
        other => panic!("expected unreachable exact blob, got {other:?}"),
    }
}

#[test]
fn removed_path_and_duplicate_basename_remain_in_archive() {
    let tmp = repo();
    commit(tmp.path(), "gone/scene.txt", "gone prompt");
    let path = tmp.path().join("gone/scene.txt");
    std::fs::remove_file(path).unwrap();
    Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["add", "-A"])
        .status()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(tmp.path())
        .args(["commit", "-qm", "remove"])
        .status()
        .unwrap();
    commit(tmp.path(), "other/scene.txt", "other prompt");
    let archive = PromptArchive::new(tmp.path(), &config()).unwrap();
    let missing = PromptGrammar::compile(&config())
        .unwrap()
        .digest(b"missing")
        .unwrap();
    match archive
        .resolve_path(Path::new(&format!("scene.txt_{missing}.png")))
        .unwrap()
    {
        PromptMatch::Nearest(paths) => assert_eq!(
            paths.iter().map(|l| l.path.as_str()).collect::<Vec<_>>(),
            ["gone/scene.txt", "other/scene.txt"]
        ),
        other => panic!("expected archived historical paths, got {other:?}"),
    }
}
