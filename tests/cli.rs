use std::{fs, process::Command};
use tempfile::tempdir;

fn cli(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn validate_uses_json_data_dir_config_from_unrelated_cwd() {
    let root = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    fs::write(
        root.path().join("publicator.toml"),
        "[deviantart]\ntiers=['allowed']\n",
    )
    .unwrap();
    fs::write(root.path().join("rows.json"), "[]").unwrap();
    let result = cli(
        cwd.path(),
        &[
            "validate",
            "--data-dir",
            root.path().to_str().unwrap(),
            "--json",
            "rows.json",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "Valid");
}

#[test]
fn validation_reports_missing_or_malformed_data_as_failure() {
    let root = tempdir().unwrap();
    let missing = cli(
        root.path(),
        &["validate", "--data-dir", root.path().to_str().unwrap()],
    );
    assert!(
        missing.status.success(),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    fs::write(root.path().join("publications.json"), "{").unwrap();
    let malformed = cli(
        root.path(),
        &["validate", "--data-dir", root.path().to_str().unwrap()],
    );
    assert!(!malformed.status.success());
}

#[test]
fn da_selection_flags_are_mutually_exclusive() {
    let root = tempdir().unwrap();
    let result = cli(root.path(), &["da-publish", "--uuid", "abc", "--all"]);
    assert!(!result.status.success());
}

#[test]
fn removed_provider_is_an_explicit_cli_error() {
    let root = tempdir().unwrap();
    let result = cli(root.path(), &["generate-meta", "--source", "claude"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("claude"));
}

#[test]
fn generation_requires_explicit_source_and_local_model() {
    let root = tempdir().unwrap();
    assert!(!cli(root.path(), &["generate-meta"]).status.success());
    assert!(
        !cli(root.path(), &["generate-meta", "--source", "local"])
            .status
            .success()
    );
}

#[test]
fn explicit_missing_uuid_is_an_error_after_steps_check() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let worker = root.path().join("worker.sh");
    fs::write(&worker, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&worker).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&worker, permissions).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root.path())
        .env("PUBLICATOR_WORKER", worker)
        .env("PUBLICATOR_SKILL", root.path().join("skill.md"))
        .args([
            "da-publish",
            "--uuid",
            "missing",
            "--data-dir",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
}

#[test]
fn reconcile_and_submission_options_cannot_be_mixed() {
    let root = tempdir().unwrap();
    let result = cli(
        root.path(),
        &[
            "da-publish",
            "--reconcile-attempt",
            "attempt",
            "--outcome",
            "not-submitted",
            "--all",
        ],
    );
    assert!(!result.status.success());
}
