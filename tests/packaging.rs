use std::{fs, process::Command};

#[path = "common/executable.rs"]
mod executable_fixture;

fn fake_command_worker(root: &std::path::Path) -> std::path::PathBuf {
    let worker = root.join("worker");
    executable_fixture::write_executable(
        &worker,
        "#!/bin/sh\nexec python -m publicator_da.worker \"$@\"\n",
    );
    worker
}

#[test]
fn cli_uses_embedded_schema_and_injected_assets_outside_the_checkout() {
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let worker = fake_command_worker(root.path());
    let skill = root.path().join("skill.md");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/.claude/skills/publish-deviantart/SKILL.md"
        ),
        &skill,
    )
    .unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(external.path())
        .env("PUBLICATOR_WORKER", &worker)
        .env("PUBLICATOR_SKILL", &skill)
        .env("PYTHONPATH", concat!(env!("CARGO_MANIFEST_DIR"), "/python"))
        .arg("check-steps")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(external.path())
        .arg("validate")
        .arg("--data-dir")
        .arg(external.path())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn check_steps_fails_when_injected_skill_asset_is_missing() {
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let worker = fake_command_worker(root.path());
    let status = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(external.path())
        .env("PUBLICATOR_WORKER", &worker)
        .env("PUBLICATOR_SKILL", root.path().join("absent.md"))
        .env("PYTHONPATH", concat!(env!("CARGO_MANIFEST_DIR"), "/python"))
        .arg("check-steps")
        .status()
        .unwrap();
    assert!(!status.success());
}
