mod common;
use common::http::{Reply, Server};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    process::{Command, Stdio},
};
use tempfile::tempdir;

fn cli(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}

fn marked_data(root: &std::path::Path) {
    fs::write(
        root.join("publications.json"),
        json!([{
            "uuid":"00000000-0000-4000-8000-000000000001",
            "state":"SCHEDULED", "title":"[[PUBLICATOR:TODO_GENERATE]]",
            "description":"[[PUBLICATOR:TODO_GENERATE]]", "submissionTimestamp":1700000000,
            "files":[{"basename":"art.png","sha512sum":"0".repeat(128)}], "apparitions":[]
        }])
        .to_string(),
    )
    .unwrap();
    assert!(
        Command::new("magick")
            .args(["-size", "4x4", "xc:white"])
            .arg(root.join("art.png"))
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join("publicator.toml"), "").unwrap();
}

fn llmfit(root: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("llmfit");
    fs::write(&path, "#!/bin/sh\nprintf '%s' '{\"models\":[{\"name\":\"google/gemma-4-26B-A4B-it\",\"fit_level\":\"Good\"}]}'\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn local_reply(path: &str, value: Value) -> Reply {
    Reply {
        path: path.into(),
        status: 200,
        body: value.to_string(),
    }
}

fn run_local(root: &std::path::Path, server: &Server, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.display(),
                std::env::var_os("PATH").unwrap().to_string_lossy()
            ),
        )
        .env("PUBLICATOR_OLLAMA_URL", server.url())
        .arg("generate-meta")
        .arg("--source")
        .arg("local")
        .arg("--model")
        .arg("gemma4:26b-a4b")
        .arg("--timeout")
        .arg("5")
        .arg("--data-dir")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}

#[cfg(unix)]
fn run_local_pty(root: &std::path::Path, server: &Server, input: &[u8]) -> std::process::Output {
    use std::{fs::File, os::fd::FromRawFd};
    let mut master = -1;
    let mut slave = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        0
    );
    let master_file = unsafe { File::from_raw_fd(master) };
    let slave_file = unsafe { File::from_raw_fd(slave) };
    let stdin = slave_file.try_clone().unwrap();
    let stdout = slave_file.try_clone().unwrap();
    let stderr = slave_file.try_clone().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root)
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.display(),
                std::env::var_os("PATH").unwrap().to_string_lossy()
            ),
        )
        .env("PUBLICATOR_OLLAMA_URL", server.url())
        .args([
            "generate-meta",
            "--source",
            "local",
            "--model",
            "gemma4:26b-a4b",
            "--timeout",
            "5",
            "--data-dir",
        ])
        .arg(root)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();
    drop(slave_file);
    let mut master_file = master_file;
    master_file.write_all(input).unwrap();
    drop(master_file.try_clone().unwrap());
    let status = child.wait().unwrap();
    let mut output = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match master_file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => panic!("read PTY output: {error}"),
        }
    }
    std::process::Output {
        status,
        stdout: output.clone(),
        stderr: output,
    }
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
fn cli_local_consent_paths_are_terminal_and_nonterminal_safe() {
    use common::id;
    let expected =
        json!({"title":"Generated title","description":"Generated description"}).to_string();
    let tags = |installed: bool| {
        local_reply(
            "/api/tags",
            json!({"models": if installed { vec![json!({"name":"gemma4:26b-a4b"})] } else { vec![] }}),
        )
    };
    let pull = || local_reply("/api/pull", json!({"status":"success"}));
    let chat = || local_reply("/api/chat", json!({"message":{"content":expected}}));

    // A non-terminal refusal fails before generation and leaves exact source bytes untouched.
    let root = tempdir().unwrap();
    marked_data(root.path());
    llmfit(root.path());
    let original = fs::read(root.path().join("publications.json")).unwrap();
    let server = Server::start(vec![tags(false)]);
    let output = run_local(root.path(), &server, &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive consent"));
    assert_eq!(
        fs::read(root.path().join("publications.json")).unwrap(),
        original
    );
    assert_eq!(server.requests().len(), 1);

    // Actual terminal input: explicit no and EOF each refuse without changing data.
    for input in [b"no\n".as_slice(), b"\x04".as_slice()] {
        let root = tempdir().unwrap();
        marked_data(root.path());
        llmfit(root.path());
        let original = fs::read(root.path().join("publications.json")).unwrap();
        let server = Server::start(vec![tags(false)]);
        let output = run_local_pty(root.path(), &server, input);
        assert!(!output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("Install gemma4:26b-a4b? [Y/n]"));
        assert_eq!(
            fs::read(root.path().join("publications.json")).unwrap(),
            original
        );
    }

    // Enter and y authorize a missing-model install; --yes authorizes without a PTY.
    for (input, yes_flag) in [
        (b"\n".as_slice(), false),
        (b"y\n".as_slice(), false),
        (b"yes\n".as_slice(), false),
        (b"".as_slice(), true),
    ] {
        let root = tempdir().unwrap();
        marked_data(root.path());
        llmfit(root.path());
        let server = Server::start(vec![tags(false), tags(false), tags(false), pull(), chat()]);
        let output = if yes_flag {
            run_local(root.path(), &server, &["--yes"])
        } else {
            run_local_pty(root.path(), &server, input)
        };
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            server
                .requests()
                .iter()
                .map(|r| r["path"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "/api/tags",
                "/api/tags",
                "/api/tags",
                "/api/pull",
                "/api/chat"
            ]
        );
        assert_eq!(
            serde_json::from_slice::<Value>(
                &fs::read(root.path().join("publications.json")).unwrap()
            )
            .unwrap()[0]["state"],
            "DESCRIBED"
        );
    }

    // An already-installed model and an empty eligible batch never prompt.
    let root = tempdir().unwrap();
    marked_data(root.path());
    llmfit(root.path());
    let server = Server::start(vec![tags(true), tags(true), tags(true), chat()]);
    let output = run_local(root.path(), &server, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["/api/tags", "/api/tags", "/api/tags", "/api/chat"]
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(root.path().join("publications.json")).unwrap())
            .unwrap()[0]["uuid"],
        id(1)
    );

    let root = tempdir().unwrap();
    fs::write(root.path().join("publications.json"), "[]").unwrap();
    let output = cli(
        root.path(),
        &[
            "generate-meta",
            "--source",
            "local",
            "--model",
            "gemma4:26b-a4b",
            "--data-dir",
            root.path().to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn echo_first_uses_configured_timezone_and_vision_models_only_runs_llmfit() {
    let root = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    fs::write(
        root.path().join("publicator.toml"),
        "[schedule]\ntimezone='Pacific/Honolulu'\n",
    )
    .unwrap();
    fs::write(root.path().join("art.png"), b"not read by basename index").unwrap();
    fs::write(root.path().join("publications.json"), json!([{
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED", "title":"Title", "description":"Description",
        "submissionTimestamp":1700000000, "files":[{"basename":"art.png","sha512sum":"0".repeat(128)}],
        "apparitions":[{"platformName":"deviantart","state":"unpublished"}]
    }]).to_string()).unwrap();
    let echo = cli(
        cwd.path(),
        &["echo-first", "--data-dir", root.path().to_str().unwrap()],
    );
    assert!(
        echo.status.success(),
        "{}",
        String::from_utf8_lossy(&echo.stderr)
    );
    let echo_stdout = String::from_utf8_lossy(&echo.stdout);
    assert!(echo_stdout.contains("path: "));
    assert!(echo_stdout.contains("title: Title"));
    assert!(echo_stdout.contains("schedule: 2023-11-14 12:13 HST"));

    use std::os::unix::fs::PermissionsExt;
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let fit = bin.join("llmfit");
    fs::write(&fit, "#!/bin/sh\nprintf '%s' '{\"models\":[{\"name\":\"google/gemma-4-26B-A4B-it\",\"fit_level\":\"Good\"},{\"name\":\"google/gemma-4-31B-it\",\"fit_level\":\"Good\"}]}'\n").unwrap();
    fs::set_permissions(&fit, fs::Permissions::from_mode(0o755)).unwrap();
    let vision = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(cwd.path())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("PUBLICATOR_OLLAMA_URL", "http://127.0.0.1:1")
        .arg("vision-models")
        .output()
        .unwrap();
    assert!(
        vision.status.success(),
        "{}",
        String::from_utf8_lossy(&vision.stderr)
    );
    assert!(String::from_utf8_lossy(&vision.stdout).contains("gemma4:26b-a4b"));
}

#[test]
fn publish_empty_batch_succeeds_without_worker() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("publications.json"), "[]").unwrap();
    let output = cli(
        root.path(),
        &["da-publish", "--data-dir", root.path().to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("published: 0, failed: 0, unknown: 0")
    );
}

#[test]
fn publish_unknown_worker_result_is_nonzero() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let image = root.path().join("art.png");
    assert!(
        Command::new("magick")
            .args(["-size", "4x4", "xc:white"])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
    fs::write(root.path().join("publicator.toml"), "tags='tags.txt'\n").unwrap();
    let digest = publicator::images::file_sha512(&image).unwrap();
    fs::write(root.path().join("publications.json"), json!([{
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED", "title":"Title", "description":"Description",
        "submissionTimestamp":1700000000, "apparitionTimestampIfDifferentThanSubmission":1700002800,
        "files":[{"basename":"art.png","sha512sum":digest}],
        "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
    }]).to_string()).unwrap();
    let worker = root.path().join("worker.sh");
    fs::write(&worker, "#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in *'\"type\":\"open\"'*) echo '{\"protocol\":1,\"type\":\"ready\"}' ;; *'\"type\":\"submit\"'*) echo malformed ;; *'\"type\":\"close\"'*) exit 0 ;; esac; done\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();
    let skill = root.path().join("skill.md");
    fs::write(&skill, "fixture skill").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root.path())
        .env("PUBLICATOR_WORKER", worker)
        .env("PUBLICATOR_SKILL", skill)
        .args(["da-publish", "--data-dir", root.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("unknown: 1"),
        "stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn publish_failed_worker_result_is_nonzero() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let image = root.path().join("art.png");
    assert!(
        Command::new("magick")
            .args(["-size", "4x4", "xc:white"])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
    fs::write(root.path().join("publicator.toml"), "tags='tags.txt'\n").unwrap();
    let digest = publicator::images::file_sha512(&image).unwrap();
    fs::write(root.path().join("publications.json"), json!([{
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED", "title":"Title", "description":"Description",
        "submissionTimestamp":1700000000, "apparitionTimestampIfDifferentThanSubmission":1700002800,
        "files":[{"basename":"art.png","sha512sum":digest}],
        "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
    }]).to_string()).unwrap();
    let worker = root.path().join("worker.sh");
    fs::write(&worker, "#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in *'\"type\":\"open\"'*) echo '{\"protocol\":1,\"type\":\"ready\"}' ;; *'\"type\":\"submit\"'*) attempt=$(printf '%s' \"$line\" | sed -n 's/.*\"attempt_id\":\"\\([^\"]*\\)\".*/\\1/p'); echo \"{\\\"protocol\\\":1,\\\"type\\\":\\\"failed\\\",\\\"attempt_id\\\":\\\"$attempt\\\",\\\"uuid\\\":\\\"00000000-0000-4000-8000-000000000001\\\",\\\"error\\\":\\\"fake failure\\\"}\" ;; *'\"type\":\"close\"'*) exit 0 ;; esac; done\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();
    let skill = root.path().join("skill.md");
    fs::write(&skill, "fixture skill").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root.path())
        .env("PUBLICATOR_WORKER", worker)
        .env("PUBLICATOR_SKILL", skill)
        .args(["da-publish", "--data-dir", root.path().to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "stdout={stdout}, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("failed: 1"), "{stdout}");
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
