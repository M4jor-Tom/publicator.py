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
    let mut command = Command::new(env!("CARGO_BIN_EXE_publicator"));
    command.current_dir(cwd).args(args);
    common::bounded_output(&mut command, std::time::Duration::from_secs(120))
}

fn marked_data(root: &std::path::Path) {
    common::solid_image(&root.join("art.png"), "white", 4, 4);
    let hash = publicator::images::file_sha512(&root.join("art.png")).unwrap();
    fs::write(
        root.join("publications.json"),
        json!([{
            "uuid":"00000000-0000-4000-8000-000000000001",
            "state":"SCHEDULED", "title":"[[PUBLICATOR:TODO_GENERATE]]",
            "description":"[[PUBLICATOR:TODO_GENERATE]]", "submissionTimestamp":1700000000,
            "files":[{"basename":"art.png","sha512sum":hash}], "apparitions":[]
        }])
        .to_string(),
    )
    .unwrap();
    fs::write(root.join("publicator.toml"), "").unwrap();
}

fn llmfit(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("llmfit");
    common::executable::write_executable(
        &path,
        "#!/bin/sh\nprintf '%s' '{\"models\":[{\"name\":\"google/gemma-4-26B-A4B-it\",\"fit_level\":\"Good\"}]}'\n",
    );
    path
}

fn publication_batch(root: &std::path::Path) {
    use common::{id, row};
    let mut historical = row(1, "APPEARED");
    historical["apparitions"] = json!([{"platformName":"historical-site","state":"unpublished"}]);
    let mut unrelated = row(2, "PICKED");
    unrelated["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    let mut rows = vec![historical, unrelated];
    for n in 3..=4 {
        let image = root.join(format!("art{n}.png"));
        common::solid_image(&image, "white", 4, 4);
        let mut item = row(n, "DESCRIBED");
        item["files"][0]["basename"] = json!(format!("art{n}.png"));
        item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
        item["apparitionTimestampIfDifferentThanSubmission"] = json!(1700002800);
        item["apparitions"] =
            json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
        rows.push(item);
    }
    fs::write(root.join("tags.txt"), "tag\n").unwrap();
    fs::write(root.join("publicator.toml"), "tags='tags.txt'\n").unwrap();
    fs::write(
        root.join("publications.json"),
        serde_json::to_string(&rows).unwrap(),
    )
    .unwrap();
    assert_eq!(id(3), "00000000-0000-4000-8000-000000000003");
}

fn logging_worker(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let worker = root.join("worker.sh");
    let calls = root.join("worker-calls.txt");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--check-steps" ]; then echo "$*" >> '{}'; exit 0; fi
while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo OPEN >> '{}'; echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*)
  attempt=$(printf '%s' "$line" | sed -n 's/.*"attempt_id":"\([^"]*\)".*/\1/p')
  uuid=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p')
  echo "$uuid" >> '{}'
  printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s"}}\n' "$attempt" "$uuid" ;;
*'"type":"close"'*) exit 0 ;;
esac
done
"#,
        calls.display(),
        calls.display(),
        calls.display()
    );
    common::executable::write_executable(&worker, &script);
    (worker, calls)
}

fn run_local(root: &std::path::Path, server: &Server, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_publicator"));
    command
        .current_dir(root)
        .env("PATH", common::path_with(root))
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
        .args(args);
    common::bounded_output(&mut command, std::time::Duration::from_secs(60))
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
        .env("PATH", common::path_with(root))
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
    let mut status = None;
    let exited = common::eventually(std::time::Duration::from_secs(60), || {
        status = child.try_wait().unwrap();
        status.is_some()
    });
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    let status = status.expect("generate-meta exited within 60 s");
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

#[cfg(feature = "desktop")]
#[test]
fn ui_rejects_unknown_legacy_limit_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_publicator-ui"))
        .args(["-n", "10"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
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
        !cli(root.path(), &["generate-meta", "--source", "unknown"])
            .status
            .success()
    );
    assert!(
        !cli(root.path(), &["generate-meta", "--source", "local"])
            .status
            .success()
    );
}

#[test]
fn default_publish_is_first_pending_and_all_uuid_remain_explicit_batches() {
    use common::id;
    for (selection, expected) in [
        (vec![], vec![id(3)]),
        (vec!["--all"], vec![id(3), id(4)]),
        (
            vec!["--uuid", "00000000-0000-4000-8000-000000000004"],
            vec![id(4)],
        ),
    ] {
        let root = tempdir().unwrap();
        publication_batch(root.path());
        let (worker, calls) = logging_worker(root.path());
        let skill = root.path().join("skill.md");
        fs::write(&skill, "fixture").unwrap();
        let mut args = vec!["da-publish", "--data-dir", root.path().to_str().unwrap()];
        args.extend(selection);
        let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
            .current_dir(root.path())
            .env("PUBLICATOR_WORKER", &worker)
            .env("PUBLICATOR_SKILL", &skill)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let called = fs::read_to_string(calls)
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with("--check-steps") && *line != "OPEN")
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(called, expected);
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("published: {},", expected.len()))
        );
    }
}

#[test]
fn headless_flag_reaches_the_publish_worker_open_command() {
    assert_eq!(publish_open("", true)["headless"], true);
}

#[test]
fn browser_headless_config_reaches_the_publish_worker_open_command() {
    assert_eq!(
        publish_open("[browser]\nheadless = true\n", false)["headless"],
        true
    );
    assert_eq!(publish_open("", false)["headless"], false);
}

/// The worker `open` command of one `da-publish` run, with `extra` appended to `publicator.toml`.
fn publish_open(extra: &str, flag: bool) -> Value {
    let root = tempdir().unwrap();
    publication_batch(root.path());
    let config = root.path().join("publicator.toml");
    let base = fs::read_to_string(&config).unwrap();
    fs::write(&config, base + extra).unwrap();
    let worker = root.path().join("headless-worker.sh");
    let open = root.path().join("open.json");
    common::executable::write_executable(
        &worker,
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--check-steps" ]; then exit 0; fi
while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) printf '%s' "$line" > '{}'; echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s"}}\n' "$attempt" "$uuid" ;;
*'"type":"close"'*) exit 0 ;;
esac
done
"#,
            open.display()
        ),
    );
    let skill = root.path().join("skill.md");
    fs::write(&skill, "fixture").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root.path())
        .env("PUBLICATOR_WORKER", &worker)
        .env("PUBLICATOR_SKILL", &skill)
        .args([
            "da-publish",
            "--uuid",
            "00000000-0000-4000-8000-000000000003",
        ])
        .args(flag.then_some("--headless"))
        .arg("--data-dir")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&fs::read(open).unwrap()).unwrap()
}

#[test]
fn check_steps_is_check_only_even_with_submission_flags_or_bad_data() {
    for malformed in [false, true] {
        let root = tempdir().unwrap();
        if malformed {
            fs::write(root.path().join("publications.json"), b"{not-json").unwrap();
        } else {
            publication_batch(root.path());
        }
        let original = fs::read(root.path().join("publications.json")).unwrap();
        let (worker, calls) = logging_worker(root.path());
        let skill = root.path().join("skill.md");
        fs::write(&skill, "fixture").unwrap();
        for selection in [
            vec!["--all"],
            vec![
                "--uuid",
                "00000000-0000-4000-8000-000000000003",
                "--headless",
            ],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
                .current_dir(root.path())
                .env("PUBLICATOR_WORKER", &worker)
                .env("PUBLICATOR_SKILL", &skill)
                .arg("da-publish")
                .arg("--check-steps")
                .args(&selection)
                .arg("--data-dir")
                .arg(root.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("check-steps OK"));
            assert_eq!(
                fs::read(root.path().join("publications.json")).unwrap(),
                original
            );
            assert!(!root.path().join(".publicator").exists());
        }
        let calls = fs::read_to_string(calls).unwrap();
        assert_eq!(calls.lines().count(), 2);
        assert!(calls.lines().all(|line| line.starts_with("--check-steps")));
    }
    let root = tempdir().unwrap();
    let reconcile = cli(
        root.path(),
        &[
            "da-publish",
            "--check-steps",
            "--reconcile-attempt",
            "attempt",
            "--outcome",
            "not-submitted",
        ],
    );
    assert!(!reconcile.status.success());
}

#[test]
fn explicit_missing_uuid_is_an_error_after_steps_check() {
    let root = tempdir().unwrap();
    let worker = root.path().join("worker.sh");
    common::executable::write_executable(&worker, "#!/bin/sh\nexit 0\n");
    let result = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root.path())
        .env("PUBLICATOR_WORKER", &worker)
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
        Reply::new(
            "/api/tags",
            200,
            json!({"models": if installed { vec![json!({"name":"gemma4:26b-a4b"})] } else { vec![] }}),
        )
    };
    let pull = || Reply::new("/api/pull", 200, json!({"status":"success"}));
    let chat = || Reply::new("/api/chat", 200, json!({"message":{"content":expected}}));

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

    // Actual terminal input: all refusal spellings, arbitrary answers and EOF refuse unchanged.
    for input in [
        b"n\n".as_slice(),
        b"N\n".as_slice(),
        b"no\n".as_slice(),
        b"NO\n".as_slice(),
        b"maybe\n".as_slice(),
        b"anything\n".as_slice(),
        b"\x04".as_slice(),
    ] {
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
        assert_eq!(
            server
                .requests()
                .iter()
                .map(|r| r["path"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["/api/tags"]
        );
    }

    // Enter and y/yes authorize; both -y and --yes do so without a PTY.
    for (input, flag) in [
        (b"\n".as_slice(), None),
        (b"y\n".as_slice(), None),
        (b"yes\n".as_slice(), None),
        (b"".as_slice(), Some("--yes")),
        (b"".as_slice(), Some("-y")),
    ] {
        let root = tempdir().unwrap();
        marked_data(root.path());
        llmfit(root.path());
        let server = Server::start(vec![tags(false), tags(false), tags(false), pull(), chat()]);
        let output = if let Some(flag) = flag {
            run_local(root.path(), &server, &[flag])
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
    let marker = root.path().join("llmfit-called");
    let fit = root.path().join("llmfit");
    common::executable::write_executable(
        &fit,
        &format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_publicator"))
        .current_dir(root.path())
        .env("PATH", common::path_with(root.path()))
        .env("PUBLICATOR_OLLAMA_URL", "http://127.0.0.1:1")
        .args([
            "generate-meta",
            "--source",
            "local",
            "--model",
            "gemma4:26b-a4b",
            "--data-dir",
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!marker.exists());
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
    fs::write(root.path().join("art.png"), b"recorded artwork bytes").unwrap();
    echo_first_data(root.path(), &root.path().join("art.png"));
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

    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let fit = bin.join("llmfit");
    common::executable::write_executable(
        &fit,
        "#!/bin/sh\nprintf '%s' '{\"models\":[{\"name\":\"google/gemma-4-26B-A4B-it\",\"fit_level\":\"Good\"},{\"name\":\"google/gemma-4-31B-it\",\"fit_level\":\"Good\"}]}'\n",
    );
    let mut vision = Command::new(env!("CARGO_BIN_EXE_publicator"));
    vision
        .current_dir(cwd.path())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("PUBLICATOR_OLLAMA_URL", "http://127.0.0.1:1")
        .arg("vision-models");
    let vision = common::bounded_output(&mut vision, std::time::Duration::from_secs(60));
    assert!(
        vision.status.success(),
        "{}",
        String::from_utf8_lossy(&vision.stderr)
    );
    assert!(String::from_utf8_lossy(&vision.stdout).contains("gemma4:26b-a4b"));
}

/// One pending DeviantArt row whose `art.png` records the hash of `recorded`'s bytes.
fn echo_first_data(root: &std::path::Path, recorded: &std::path::Path) {
    fs::write(root.join("publications.json"), json!([{
        "uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED", "title":"Title", "description":"Description",
        "submissionTimestamp":1700000000,
        "files":[{"basename":"art.png","sha512sum":publicator::images::file_sha512(recorded).unwrap()}],
        "apparitions":[{"platformName":"deviantart","state":"unpublished"}]
    }]).to_string()).unwrap();
}

#[test]
fn echo_first_prints_the_recorded_file_among_same_named_files() {
    for recorded in ["a", "b"] {
        let root = tempdir().unwrap();
        // a/ is both named and created first, so a name-, creation- or reverse-ordered walk meets
        // the unrecorded namesake first in one of the two cases.
        for dir in ["a", "b"] {
            fs::create_dir(root.path().join(dir)).unwrap();
            fs::write(root.path().join(dir).join("art.png"), dir).unwrap();
        }
        let expected = root.path().join(recorded).join("art.png");
        echo_first_data(root.path(), &expected);
        let echo = cli(
            root.path(),
            &["echo-first", "--data-dir", root.path().to_str().unwrap()],
        );
        assert!(
            echo.status.success(),
            "{}",
            String::from_utf8_lossy(&echo.stderr)
        );
        assert!(
            String::from_utf8_lossy(&echo.stdout)
                .lines()
                .any(|line| line == format!("path: {}", expected.display())),
            "{}",
            String::from_utf8_lossy(&echo.stdout)
        );
    }
}

#[test]
fn echo_first_refuses_artwork_whose_bytes_changed() {
    let root = tempdir().unwrap();
    let art = root.path().join("art.png");
    fs::write(&art, b"recorded artwork bytes").unwrap();
    echo_first_data(root.path(), &art);
    fs::write(&art, b"edited artwork bytes").unwrap();
    let echo = cli(
        root.path(),
        &["echo-first", "--data-dir", root.path().to_str().unwrap()],
    );
    assert!(!echo.status.success());
    assert!(
        String::from_utf8_lossy(&echo.stderr).contains("no longer matches its recorded content"),
        "{}",
        String::from_utf8_lossy(&echo.stderr)
    );
    assert!(!String::from_utf8_lossy(&echo.stdout).contains("path:"));
}

#[test]
fn echo_first_fails_when_no_publication_is_pending() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("publications.json"), "[]").unwrap();
    let output = cli(
        root.path(),
        &["echo-first", "--data-dir", root.path().to_str().unwrap()],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no unpublished publication"));
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

fn publish_fixture() -> common::Fixture {
    let f = common::Fixture::new();
    let image = f.image("art.png", 4, 4);
    fs::write(f.path("tags.txt"), "tag\n").unwrap();
    fs::write(f.path("publicator.toml"), "tags='tags.txt'\n").unwrap();
    let mut publication = common::row(1, "DESCRIBED");
    publication["files"][0]["basename"] = json!("art.png");
    publication["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
    publication["apparitionTimestampIfDifferentThanSubmission"] = json!(1700002800);
    publication["apparitions"] = json!([
        {"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}
    ]);
    f.write(json!([publication]));
    f
}

fn publish_command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_publicator"));
    command
        .current_dir(root)
        .env("PUBLICATOR_WORKER", root.join("worker.sh"))
        .env("PUBLICATOR_SKILL", root.join("skill.md"))
        .args(["da-publish", "--data-dir", root.to_str().unwrap()]);
    command
}

fn run_publish(root: &std::path::Path) -> std::process::Output {
    common::bounded_output(
        &mut publish_command(root),
        std::time::Duration::from_secs(120),
    )
}

fn run_publish_worker(root: &std::path::Path, script: &str) -> std::process::Output {
    let worker = root.join("worker.sh");
    common::executable::write_executable(&worker, script);
    fs::write(root.join("skill.md"), "fixture skill").unwrap();
    run_publish(root)
}

#[test]
fn publish_unknown_worker_result_is_nonzero() {
    let f = publish_fixture();
    let root = f.root.path();
    let starts = root.join("worker-starts.txt");
    let output = run_publish_worker(
        root,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in *'\"type\":\"open\"'*) echo start >> '{}'; echo '{{\"protocol\":1,\"type\":\"ready\"}}' ;; *'\"type\":\"submit\"'*) echo malformed ;; *'\"type\":\"close\"'*) exit 0 ;; esac; done\n",
            starts.display()
        ),
    );
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
    assert_eq!(fs::read_to_string(&starts).unwrap().lines().count(), 1);
    let unresolved = run_publish(root);
    assert!(!unresolved.status.success());
    assert!(String::from_utf8_lossy(&unresolved.stderr).contains("unresolved"));
    assert_eq!(fs::read_to_string(starts).unwrap().lines().count(), 1);
}

#[test]
fn publish_failed_worker_result_is_nonzero() {
    let f = publish_fixture();
    let root = f.root.path();
    let output = run_publish_worker(
        root,
        "#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in *'\"type\":\"open\"'*) echo '{\"protocol\":1,\"type\":\"ready\"}' ;; *'\"type\":\"submit\"'*) attempt=$(printf '%s' \"$line\" | sed -n 's/.*\"attempt_id\":\"\\([^\"]*\\)\".*/\\1/p'); echo \"{\\\"protocol\\\":1,\\\"type\\\":\\\"failed\\\",\\\"attempt_id\\\":\\\"$attempt\\\",\\\"uuid\\\":\\\"00000000-0000-4000-8000-000000000001\\\",\\\"error\\\":\\\"fake failure\\\"}\" ;; *'\"type\":\"close\"'*) exit 0 ;; esac; done\n",
    );
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

#[test]
fn validate_rejects_invalid_schedule_semantics() {
    for (config, diagnostic) in [
        ("[schedule]\nhour = 24\n", "hour must be 0..=23"),
        ("[schedule]\ntimezone = 7\n", "timezone must be an IANA"),
        ("[schedule]\nprofiles = []\n", "profiles is empty"),
    ] {
        let root = tempdir().unwrap();
        fs::write(root.path().join("publications.json"), "[]").unwrap();
        fs::write(root.path().join("publicator.toml"), config).unwrap();
        let output = cli(
            root.path(),
            &["validate", "--data-dir", root.path().to_str().unwrap()],
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {config:?}");
        assert!(
            stderr.contains(&format!("schedule: {diagnostic}")),
            "{stderr}"
        );
    }
}

#[test]
fn sigint_during_publish_stops_the_worker_group_and_keeps_the_attempt_blocked() {
    let f = publish_fixture();
    let root = f.root.path();
    let (calls, gate) = (root.join("calls.log"), root.join("gate"));
    common::executable::write_executable(
        &root.join("worker.sh"),
        &common::hanging_submit_worker(&calls, &gate),
    );
    fs::write(root.join("skill.md"), "fixture skill").unwrap();
    let _watchdog = common::GroupWatchdog(&gate);
    let mut cli = common::Reaped(
        publish_command(root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let started = common::eventually(std::time::Duration::from_secs(20), || {
        common::gate_pids(&gate).is_some()
    });
    assert!(started, "worker never reached submit");
    assert_eq!(unsafe { libc::kill(cli.0.id() as i32, libc::SIGINT) }, 0);
    assert!(common::eventually(
        std::time::Duration::from_secs(10),
        || { cli.0.try_wait().unwrap().is_some() }
    ));
    let mut stderr = String::new();
    cli.0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(cli.0.wait().unwrap().code(), Some(1), "{stderr}");
    assert!(stderr.contains("interrupted"), "{stderr}");
    common::assert_abandoned_attempt(root, &gate, &calls);
    let retry = run_publish(root);
    assert!(!retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("unresolved"));
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 1);
}

#[test]
fn cli_reconciliation_of_an_unknown_outcome_restores_exactly_the_decided_state() {
    let url = "https://www.deviantart.com/example/art/work";
    for (decision, sends, stored_url) in [
        (vec!["--outcome", "scheduled", "--url", url], 1, Some(url)),
        (vec!["--outcome", "scheduled"], 1, None),
        (vec!["--outcome", "not-submitted"], 2, None),
    ] {
        let f = publish_fixture();
        let root = f.root.path();
        let elsewhere = tempdir().unwrap();
        let calls = root.join("calls.log");
        let lost_once = root.join("lost-once");
        common::executable::write_executable(
            &root.join("worker.sh"),
            &format!(
                r#"#!/bin/sh
[ "$1" = "--check-steps" ] && exit 0
while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) echo sent >> '{calls}'
  if [ -e '{lost}' ]; then printf '%s' "$line" | jq -c '{{protocol:1,type:"scheduled",attempt_id,uuid}}'
  else touch '{lost}'; printf '%s' "$line" | jq -c '{{protocol:1,type:"unknown",attempt_id,uuid,error:"confirmation lost"}}'; fi ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
                calls = calls.display(),
                lost = lost_once.display()
            ),
        );
        fs::write(root.join("skill.md"), "fixture skill").unwrap();
        let run = |extra: &[&str]| {
            let output = publish_command(root)
                .current_dir(elsewhere.path())
                .args(extra)
                .output()
                .unwrap();
            let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
            (
                output.status.success(),
                text(&output.stdout),
                text(&output.stderr),
            )
        };
        let sent = || fs::read_to_string(&calls).unwrap().lines().count();

        let (ok, stdout, _) = run(&[]);
        assert!(!ok && stdout.contains("unknown: 1"), "{stdout}");
        let (ok, _, stderr) = run(&[]);
        assert!(!ok && stderr.contains("unresolved"), "{stderr}");
        assert_eq!(sent(), 1);
        let attempt = common::only_attempt(root);
        let (ok, stdout, stderr) =
            run(&[&["--reconcile-attempt", &attempt][..], &decision].concat());
        assert!(
            ok && stdout.contains(&format!("reconciled {attempt}")),
            "{stderr}"
        );

        let (ok, stdout, stderr) = run(&[]);
        assert!(ok, "{decision:?}: {stdout} {stderr}");
        assert!(
            stdout.contains(&format!("published: {}", sends - 1)),
            "{stdout}"
        );
        assert_eq!(sent(), sends, "{decision:?}");
        let delivery = f.read()[0]["apparitions"][0].clone();
        assert_eq!(delivery["state"], "published_or_scheduled");
        assert_eq!(delivery.get("url").and_then(Value::as_str), stored_url);
        let receipt: Value = serde_json::from_slice(
            &fs::read(root.join(format!(".publicator/publish/{attempt}/receipt.json"))).unwrap(),
        )
        .unwrap();
        let resolved = if sends == 1 {
            "applied"
        } else {
            "not_submitted"
        };
        assert_eq!(receipt[resolved], true, "{receipt}");
        let (ok, stdout, _) = run(&[]);
        assert!(
            ok && stdout.contains("published: 0, failed: 0, unknown: 0"),
            "{stdout}"
        );
        assert_eq!(sent(), sends);
    }
}

/// Phrases the CLI unknown-outcome guidance and the skill's recovery prose must share.
const RECOVERY_PHRASES: [&str; 7] = [
    "UNRESOLVED",
    "--reconcile-attempt",
    "--outcome scheduled",
    "--outcome not-submitted",
    "Inspect DeviantArt first",
    "do not resubmit or redo",
    ".publicator/",
];

fn assert_recovery_guidance(stderr: &[u8], attempt: &str) {
    let text = String::from_utf8_lossy(stderr);
    for phrase in RECOVERY_PHRASES {
        assert!(text.contains(phrase), "missing {phrase:?}: {text}");
    }
    let listed = format!("UNRESOLVED {attempt} (00000000-0000-4000-8000-000000000001)");
    let release = format!("--reconcile-attempt {attempt} --outcome not-submitted");
    assert!(text.contains(&listed) && text.contains(&release), "{text}");
}

#[test]
fn unknown_publish_outcome_prints_reconcile_guidance_matching_the_skill() {
    let f = publish_fixture();
    let root = f.root.path();
    let first = run_publish_worker(
        root,
        "#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then exit 0; fi\nwhile IFS= read -r line; do case \"$line\" in *'\"type\":\"open\"'*) echo '{\"protocol\":1,\"type\":\"ready\"}' ;; *'\"type\":\"submit\"'*) echo malformed ;; *) exit 0 ;; esac; done\n",
    );
    assert!(!first.status.success());
    let attempt = &common::only_attempt(root);
    let blocked = run_publish(root);
    assert!(!blocked.status.success());
    assert_recovery_guidance(&first.stderr, attempt);
    assert_recovery_guidance(&blocked.stderr, attempt);
    assert!(
        root.join(format!(".publicator/publish/{attempt}/receipt.json"))
            .is_file()
    );

    let skill = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/skills/publish-deviantart/SKILL.md"
    ))
    .unwrap();
    for phrase in RECOVERY_PHRASES {
        assert!(skill.contains(phrase), "skill lacks {phrase:?}");
    }
    assert!(!skill.contains("AUTOMATION FAILED"));
    assert!(!skill.contains("Fall through to the manual steps"));

    let data_dir = root.to_str().unwrap();
    let reconciled = cli(
        root,
        &[
            "da-publish",
            "--data-dir",
            data_dir,
            "--reconcile-attempt",
            attempt,
            "--outcome",
            "not-submitted",
        ],
    );
    assert!(
        reconciled.status.success(),
        "{}",
        String::from_utf8_lossy(&reconciled.stderr)
    );
}

#[test]
fn checkout_check_steps_runs_the_default_worker_without_overrides() {
    let output = common::bounded_output(
        Command::new(env!("CARGO_BIN_EXE_publicator"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env_remove("PUBLICATOR_WORKER")
            .env_remove("PUBLICATOR_SKILL")
            .env_remove("PYTHONPATH")
            .arg("check-steps"),
        std::time::Duration::from_secs(60),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The real worker compared its STEPS with the static skill, not just Rust's exit path.
    assert!(stdout.contains("steps in sync with the skill"), "{stdout}");
}

const DUMMY_NOW: i64 = 1_800_000_000;

/// Runs `scripts/dummy-data.sh DIR` under a watchdog, optionally with spy tools first on PATH.
fn dummy_data(dir: &std::path::Path, spies: Option<&std::path::Path>) -> std::process::Output {
    let mut command = Command::new("bash");
    command
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/dummy-data.sh"
        ))
        .arg(dir)
        .env("PUBLICATOR_DUMMY_NOW", DUMMY_NOW.to_string());
    if let Some(spies) = spies {
        command.env("PATH", common::path_with(spies));
    }
    common::bounded_output(&mut command, std::time::Duration::from_secs(120))
}

#[test]
fn dummy_data_refuses_existing_destinations_before_running_tools() {
    let root = tempdir().unwrap();
    let (spies, calls, data) = (
        root.path().join("bin"),
        root.path().join("calls"),
        root.path().join("data"),
    );
    fs::create_dir(&spies).unwrap();
    for tool in ["magick", "jq", "git"] {
        common::executable::write_executable(
            &spies.join(tool),
            &format!("#!/bin/sh\necho {tool} >> '{}'\nexit 1\n", calls.display()),
        );
    }
    fs::create_dir_all(data.join("picked")).unwrap();
    let sentinels = [
        ("publicator.toml", "publicable = [\"picked\"]\n"),
        ("publications.json", "[]"),
        ("picked/art.png", "sentinel art"),
    ];
    for (name, bytes) in sentinels {
        fs::write(data.join(name), bytes).unwrap();
    }
    std::os::unix::fs::symlink(&data, root.path().join("link")).unwrap();
    for target in [
        data.clone(),
        root.path().join("link"),
        data.join("publications.json"),
    ] {
        let output = dummy_data(&target, Some(&spies));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{target:?}: {stderr}");
        assert!(stderr.contains("Refusing"), "{target:?}: {stderr}");
    }
    assert!(!calls.exists(), "{}", fs::read_to_string(&calls).unwrap());
    for (name, bytes) in sentinels {
        assert_eq!(fs::read_to_string(data.join(name)).unwrap(), bytes);
    }
    assert_eq!(fs::read_dir(&data).unwrap().count(), 3);
}

#[test]
fn dummy_data_initializes_a_fresh_valid_root_and_refuses_to_reinitialize_it() {
    let root = tempdir().unwrap();
    let data = root.path().join("fresh");
    let output = dummy_data(&data, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data_arg = data.to_str().unwrap();
    let validate = cli(root.path(), &["validate", "--data-dir", data_arg]);
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let audit = cli(root.path(), &["prompt-audit", "--data-dir", data_arg]);
    let report = String::from_utf8_lossy(&audit.stdout);
    assert!(
        report.contains("exact: 2 images") && report.contains("nearest: 1 images"),
        "{report}"
    );
    let tags = publicator::config::Config::load(&data)
        .unwrap()
        .tags
        .unwrap();
    assert!(
        !fs::read_to_string(data.join(tags))
            .unwrap()
            .trim()
            .is_empty()
    );
    let saved = fs::read(data.join("publications.json")).unwrap();
    let rows: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(rows[0]["submissionTimestamp"], DUMMY_NOW - 30 * 86_400);

    let rerun = dummy_data(&data, None);
    assert!(!rerun.status.success());
    assert_eq!(fs::read(data.join("publications.json")).unwrap(), saved);
}

#[test]
fn roll_back_steps_back_one_state_and_refuses_picked_and_appeared() {
    let root = tempdir().unwrap();
    let data_dir = root.path().to_str().unwrap();
    let json = root.path().join("publications.json");
    fs::write(
        &json,
        serde_json::to_vec(&serde_json::json!([
            common::row(1, "DESCRIBED"),
            common::row(2, "APPEARED")
        ]))
        .unwrap(),
    )
    .unwrap();
    let roll_back = |n| {
        cli(
            root.path(),
            &[
                "roll-back",
                "--data-dir",
                data_dir,
                "--uuid",
                &common::id(n),
            ],
        )
    };
    for state in ["SCHEDULED", "PICKED"] {
        let rolled = roll_back(1);
        assert!(
            rolled.status.success(),
            "{}",
            String::from_utf8_lossy(&rolled.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&rolled.stdout).trim(),
            format!("rolled back {} to {state}", common::id(1))
        );
        let rows: serde_json::Value = serde_json::from_slice(&fs::read(&json).unwrap()).unwrap();
        assert_eq!(rows[0]["state"], state);
        assert_eq!(rows[0]["title"], "Title");
        assert_eq!(rows[0]["description"], "Description");
    }
    let before = fs::read(&json).unwrap();
    let picked = roll_back(1);
    assert!(!picked.status.success());
    assert!(String::from_utf8_lossy(&picked.stderr).contains("delete-publication"));
    assert!(!roll_back(2).status.success());
    assert_eq!(fs::read(&json).unwrap(), before);
}

#[test]
fn delete_publication_removes_only_the_named_undelivered_entry() {
    let root = tempdir().unwrap();
    let data_dir = root.path().to_str().unwrap();
    fs::write(
        root.path().join("publications.json"),
        serde_json::to_vec(&serde_json::json!([
            common::row(1, "PICKED"),
            common::row(2, "APPEARED")
        ]))
        .unwrap(),
    )
    .unwrap();
    let deleted = cli(
        root.path(),
        &[
            "delete-publication",
            "--data-dir",
            data_dir,
            "--uuid",
            &common::id(1),
        ],
    );
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&deleted.stdout).trim(),
        format!("deleted {}", common::id(1))
    );
    for uuid in [common::id(1), common::id(2)] {
        let refused = cli(
            root.path(),
            &[
                "delete-publication",
                "--data-dir",
                data_dir,
                "--uuid",
                &uuid,
            ],
        );
        assert!(!refused.status.success(), "{uuid}");
    }
    let rows: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join("publications.json")).unwrap()).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["uuid"], common::id(2));
}
