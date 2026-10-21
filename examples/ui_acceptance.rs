use publicator::{
    application::Application,
    config::{Config, Paths},
    metadata::{Endpoints, MetadataClient},
    model::GENERATE_MARKER,
    publishing::Publisher,
    store::Store,
    ui::{self, UiOptions},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[cfg(test)]
use std::process::{Command, Stdio};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(default)]
struct FixtureSettings {
    installed_models: Vec<String>,
    ollama_available: bool,
    fit_level: String,
    fit_reason: String,
    openrouter_delay_ms: u64,
    local_delay_ms: u64,
    openrouter_failures: usize,
    local_failures: usize,
    worker_outcomes: Vec<String>,
    tiers: Vec<String>,
    galleries: Vec<String>,
}

impl Default for FixtureSettings {
    fn default() -> Self {
        Self {
            installed_models: vec!["gemma4:31b".into()],
            ollama_available: true,
            fit_level: "Good".into(),
            fit_reason: "Disposable fixture reports a suitable host.".into(),
            openrouter_delay_ms: 0,
            local_delay_ms: 0,
            openrouter_failures: 0,
            local_failures: 0,
            worker_outcomes: vec!["failed".into(), "scheduled".into(), "unknown".into()],
            tiers: vec!["Fixture Basic".into(), "Fixture Premium".into()],
            galleries: vec!["Fixture Gallery A".into(), "Fixture Gallery B".into()],
        }
    }
}

#[derive(Default)]
struct FakeCallState {
    sequence: usize,
    openrouter_failures: usize,
    local_failures: usize,
}

fn arg(name: &str) -> PathBuf {
    let mut args = std::env::args_os().skip(1);
    while let Some(key) = args.next() {
        if key == name {
            return args.next().map(PathBuf::from).unwrap_or_else(|| usage());
        }
    }
    usage()
}

fn usage() -> ! {
    eprintln!("usage: ui_acceptance --fixture-dir DIR --scenario empty|history|mixed|reopen");
    std::process::exit(2)
}

fn image(path: &Path, color: [u8; 3]) {
    image::RgbImage::from_pixel(480, 300, image::Rgb(color))
        .save(path)
        .expect("write disposable image fixture");
}

fn row(uuid: u64, state: &str, color: [u8; 3], dir: &Path) -> Value {
    let name = format!("{uuid}.png");
    let path = dir.join(&name);
    image(&path, color);
    json!({
        "uuid": format!("00000000-0000-4000-8000-{uuid:012}"),
        "state": state,
        "title": if state == "SCHEDULED" { GENERATE_MARKER } else { "Fixture title" },
        "description": if state == "SCHEDULED" { GENERATE_MARKER } else { "Fixture description" },
        "submissionTimestamp": 1_700_000_000,
        "apparitionTimestampIfDifferentThanSubmission": 1_700_002_800 + uuid as i64 * 86400,
        "files": [{"basename": name, "sha512sum": publicator::images::file_sha512(&path).unwrap()}],
        "apparitions": if state == "APPEARED" {
            json!([{"platformName":"deviantart","state":"published_or_scheduled","url":"https://example.test/fixture/{uuid}"}])
        } else if state == "DESCRIBED" {
            json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}])
        } else { json!([]) }
    })
}

fn fixture_rows(scenario: &str, dir: &Path) -> Vec<Value> {
    match scenario {
        "empty" => Vec::new(),
        "history" => vec![row(1, "APPEARED", [80, 140, 210], dir)],
        "publish" => {
            let mut history = row(22, "APPEARED", [220, 180, 50], dir);
            history["apparitions"] = json!([
                {
                    "platformName": "deviantart",
                    "state": "published_or_scheduled",
                    "url": "https://example.test/historical/22"
                },
                {"platformName": "historical-platform", "state": "unpublished"}
            ]);
            vec![
                row(20, "DESCRIBED", [100, 100, 220], dir),
                row(21, "DESCRIBED", [150, 100, 30], dir),
                history,
            ]
        }
        "mixed" => {
            for (name, color) in [
                ("candidate-a.png", [140, 20, 120]),
                ("candidate-b.png", [40, 120, 210]),
                ("candidate-c.png", [190, 90, 10]),
                ("candidate-d.png", [15, 150, 160]),
                ("candidate-e.png", [190, 30, 50]),
                ("candidate-f.png", [20, 60, 170]),
            ] {
                image(&dir.join(name), color);
            }
            vec![
                row(10, "PICKED", [220, 80, 80], dir),
                row(11, "SCHEDULED", [80, 180, 110], dir),
                row(12, "DESCRIBED", [100, 100, 220], dir),
                row(13, "APPEARED", [220, 180, 50], dir),
                row(14, "DESCRIBED", [150, 100, 30], dir),
                row(15, "SCHEDULED", [60, 30, 190], dir),
            ]
        }
        _ => usage(),
    }
}

fn initialize_publications_if_absent(path: &Path, rows: &[Value]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(rows).expect("serialize fixture rows");
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error),
    };
    file.write_all(&bytes)
}

fn load_fixture_settings(dir: &Path) -> FixtureSettings {
    let path = dir.join("native-fixture.json");
    let defaults = serde_json::to_vec_pretty(&FixtureSettings::default()).unwrap();
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file.write_all(&defaults).expect("write fixture settings"),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => panic!("create fixture settings: {error}"),
    }
    serde_json::from_slice(&fs::read(path).expect("read fixture settings"))
        .expect("parse fixture settings")
}

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).expect("write fixture executable");
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn write_llmfit(path: &Path, settings_path: &Path) {
    let settings_path = serde_json::to_string(&settings_path.to_string_lossy().as_ref()).unwrap();
    write_executable(
        path,
        &format!(
            r#"#!/usr/bin/env python3
import json, pathlib, sys
settings = json.loads(pathlib.Path({settings_path}).read_text())
models = ["google/gemma-4-26B-A4B-it", "google/gemma-4-31B-it"]
print(json.dumps(dict(models=[dict(name=name, fit_level=settings["fit_level"], notes=[settings["fit_reason"]]) for name in models])))
"#
        ),
    );
}

fn write_worker(path: &Path) {
    write_executable(
        path,
        r#"#!/bin/sh
if [ "$1" = "--check-steps" ]; then exit 0; fi
settings="$(dirname -- "$0")/native-fixture.json"
calls="$(dirname -- "$0")/worker-calls.jsonl"
while IFS= read -r line; do
  kind=$(printf '%s' "$line" | jq -r .type)
  case "$kind" in
    open) echo '{"protocol":1,"type":"ready"}' ;;
    submit)
      printf '%s\n' "$line" >> "$calls"
      count=$(wc -l < "$calls")
      attempt=$(printf '%s' "$line" | jq -r .attempt_id)
      uuid=$(printf '%s' "$line" | jq -r .uuid)
      outcome=$(jq -r --argjson count "$count" '.worker_outcomes[$count - 1] // "unknown"' "$settings")
      case "$outcome" in
        failed) printf '{"protocol":1,"type":"failed","attempt_id":"%s","uuid":"%s","error":"fixture failure"}\n' "$attempt" "$uuid" ;;
        scheduled) printf '{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/fixture/%s"}\n' "$attempt" "$uuid" "$uuid" ;;
        *) printf '{"protocol":1,"type":"unknown","attempt_id":"%s","uuid":"%s","error":"disposable fixture ambiguity"}\n' "$attempt" "$uuid" ;;
      esac
      ;;
    close) exit 0 ;;
  esac
done
"#,
    );
}

fn http_fixture(
    dir: &Path,
    settings: FixtureSettings,
) -> (String, Arc<AtomicBool>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local HTTP fixture");
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let log = dir.join("http-calls.jsonl");
    let settings = Arc::new(settings);
    let calls = Arc::new(Mutex::new(FakeCallState {
        openrouter_failures: settings.openrouter_failures,
        local_failures: settings.local_failures,
        ..FakeCallState::default()
    }));
    let task_settings = settings.clone();
    let task_calls = calls.clone();
    let task = thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => respond(&mut stream, &log, &task_settings, &task_calls),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    });
    (endpoint, stop, task)
}

fn read_request(stream: &TcpStream) -> std::io::Result<(String, String, Vec<u8>)> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; content_length.min(32 * 1024 * 1024)];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    Ok((
        parts.next().unwrap_or_default().to_owned(),
        parts.next().unwrap_or_default().to_owned(),
        body,
    ))
}

fn metadata_content(sequence: usize) -> String {
    serde_json::json!({
        "title": format!("Fixture generated {sequence}"),
        "description": format!("Fixture metadata response {sequence}. This is a disposable local response.")
    })
    .to_string()
}

fn fake_response(
    method: &str,
    path: &str,
    request_body: &[u8],
    settings: &FixtureSettings,
    calls: &Mutex<FakeCallState>,
) -> (u16, Value, String) {
    let parsed = serde_json::from_slice::<Value>(request_body).unwrap_or(Value::Null);
    let model = parsed.get("model").and_then(Value::as_str);
    let delay = if path == "/chat/completions" {
        settings.openrouter_delay_ms
    } else if path == "/api/chat" {
        settings.local_delay_ms
    } else {
        0
    };
    if delay > 0 {
        thread::sleep(Duration::from_millis(delay));
    }
    let mut state = calls.lock().unwrap();
    state.sequence += 1;
    let sequence = state.sequence;
    let failed = match path {
        "/chat/completions" if state.openrouter_failures > 0 => {
            state.openrouter_failures -= 1;
            true
        }
        "/api/chat" if state.local_failures > 0 => {
            state.local_failures -= 1;
            true
        }
        _ => false,
    };
    let (status, response) = match (method, path) {
        ("GET", "/api/tags") if !settings.ollama_available => (
            503,
            serde_json::json!({"error":{"message":"fixture Ollama unavailable"}}),
        ),
        ("GET", "/api/tags") => (
            200,
            serde_json::json!({"models": settings.installed_models.iter().map(|name| serde_json::json!({"name":name})).collect::<Vec<_>>() }),
        ),
        ("GET", "/models") => (
            200,
            serde_json::json!({"data":[{
                "id":"google/gemma-4-26b-a4b-it:free",
                "architecture":{"input_modalities":["text","image"]},
                "pricing":{"prompt":"0","completion":"0"},
                "supported_parameters":["response_format"]
            }]}),
        ),
        ("POST", "/chat/completions") if failed => (
            503,
            serde_json::json!({"error":{"message":"disposable OpenRouter fixture failure"}}),
        ),
        ("POST", "/chat/completions") => (
            200,
            serde_json::json!({"choices":[{"message":{"content":metadata_content(sequence)}}]}),
        ),
        ("POST", "/api/pull") => (200, serde_json::json!({"status":"success"})),
        ("POST", "/api/chat") if failed => (
            503,
            serde_json::json!({"error":{"message":"disposable Ollama fixture failure"}}),
        ),
        ("POST", "/api/chat") => (
            200,
            serde_json::json!({"message":{"content":metadata_content(sequence)}}),
        ),
        _ => (
            404,
            serde_json::json!({"error":{"message":"unknown fixture route"}}),
        ),
    };
    let record = serde_json::json!({
        "sequence": sequence,
        "method": method,
        "path": path,
        "model": model,
        "body_bytes": request_body.len(),
        "status": status
    });
    let log = serde_json::to_string(&record).unwrap();
    drop(state);
    (status, response, log)
}

fn respond(
    stream: &mut TcpStream,
    log: &Path,
    settings: &FixtureSettings,
    calls: &Mutex<FakeCallState>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (method, path, request_body) = match read_request(stream) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("fixture request read failed: {error}");
            return;
        }
    };
    let (status, response, record) = fake_response(&method, &path, &request_body, settings, calls);
    let response = response.to_string();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    writeln!(file, "{record}").unwrap();
    let reason = if status == 200 {
        "OK"
    } else if status == 503 {
        "Service Unavailable"
    } else {
        "Not Found"
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use publicator::metadata::VISION_MODELS;

    #[test]
    fn fixture_worker_outcomes_are_configurable_for_retry_and_reconciliation() {
        let dir = tempfile::tempdir().unwrap();
        let settings = FixtureSettings {
            worker_outcomes: vec![
                "scheduled".into(),
                "failed".into(),
                "scheduled".into(),
                "unknown".into(),
            ],
            ..FixtureSettings::default()
        };
        fs::write(
            dir.path().join("native-fixture.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let worker = dir.path().join("worker.sh");
        write_worker(&worker);
        let mut child = Command::new(&worker)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{{\"type\":\"open\"}}").unwrap();
        for suffix in 1..=4 {
            writeln!(
                stdin,
                "{{\"type\":\"submit\",\"attempt_id\":\"00000000-0000-4000-8000-{suffix:012}\",\"uuid\":\"00000000-0000-4000-8000-{suffix:012}\"}}"
            )
            .unwrap();
        }
        writeln!(stdin, "{{\"type\":\"close\"}}").unwrap();
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let events = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events[0]["type"], "ready");
        let protocol_events = events[1..]
            .iter()
            .cloned()
            .map(serde_json::from_value::<publicator::publish_protocol::WorkerEvent>)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(protocol_events.len(), 4);
        assert_eq!(
            events[1..]
                .iter()
                .map(|event| event["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["scheduled", "failed", "scheduled", "unknown"]
        );
        assert_eq!(
            events[1..]
                .iter()
                .map(|event| event["attempt_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            (1..=4)
                .map(|suffix| format!("00000000-0000-4000-8000-{suffix:012}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("worker-calls.jsonl"))
                .unwrap()
                .lines()
                .count(),
            4
        );
    }

    #[test]
    fn publish_fixture_has_two_registered_targets_and_unregistered_history() {
        let dir = tempfile::tempdir().unwrap();
        let rows = fixture_rows("publish", dir.path());
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .filter(|row| {
                    row["apparitions"].as_array().unwrap().iter().any(|app| {
                        app["platformName"] == "deviantart" && app["state"] == "unpublished"
                    })
                })
                .count(),
            2
        );
        assert_eq!(
            rows[2]["apparitions"][1]["platformName"],
            "historical-platform"
        );
        assert_eq!(rows[2]["apparitions"][1]["state"], "unpublished");
    }

    #[test]
    fn fixture_initialization_preserves_existing_publication_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("publications.json");
        let seed = vec![json!({"seed": true})];
        initialize_publications_if_absent(&path, &seed).unwrap();
        fs::write(&path, b"saved fixture edit").unwrap();
        initialize_publications_if_absent(&path, &seed).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"saved fixture edit");
    }

    #[test]
    fn fake_metadata_response_is_a_valid_pair_and_logs_model_order() {
        let settings = FixtureSettings::default();
        let calls = Mutex::new(FakeCallState::default());
        let request = serde_json::json!({"model":"openrouter/fake-model"}).to_string();
        let (status, response, record) = fake_response(
            "POST",
            "/chat/completions",
            request.as_bytes(),
            &settings,
            &calls,
        );
        let content = response
            .pointer("/choices/0/message/content")
            .unwrap()
            .as_str()
            .unwrap();
        let metadata = publicator::metadata::parse_metadata(content).unwrap();
        assert_eq!(status, 200);
        assert_eq!(metadata.title, "Fixture generated 1");
        assert_eq!(
            serde_json::from_str::<Value>(&record).unwrap()["model"],
            "openrouter/fake-model"
        );
    }

    #[test]
    fn fixture_failure_is_bounded_to_the_configured_number_of_requests() {
        let settings = FixtureSettings {
            openrouter_failures: 1,
            ..FixtureSettings::default()
        };
        let calls = Mutex::new(FakeCallState {
            openrouter_failures: 1,
            ..FakeCallState::default()
        });
        let request = serde_json::json!({"model":"fake"}).to_string();
        let (first, _, _) = fake_response(
            "POST",
            "/chat/completions",
            request.as_bytes(),
            &settings,
            &calls,
        );
        let (second, body, _) = fake_response(
            "POST",
            "/chat/completions",
            request.as_bytes(),
            &settings,
            &calls,
        );
        assert_eq!(first, 503);
        assert_eq!(second, 200);
        assert_eq!(
            publicator::metadata::parse_metadata(
                body.pointer("/choices/0/message/content")
                    .unwrap()
                    .as_str()
                    .unwrap()
            )
            .unwrap()
            .title,
            "Fixture generated 2"
        );
    }

    #[tokio::test]
    async fn fixture_llmfit_emits_the_native_status_parser_shape() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("native-fixture.json");
        let settings = FixtureSettings::default();
        fs::write(&settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
        let program = dir.path().join("llmfit");
        write_llmfit(&program, &settings_path);
        let client = MetadataClient::new(reqwest::Client::new(), Endpoints::default(), program);
        let fit = client.local_fit(&VISION_MODELS[0]).await.unwrap();
        assert!(fit.fit);
        assert_eq!(fit.level, "Good");
        assert_eq!(fit.reason, settings.fit_reason);
    }
}

fn main() -> iced::Result {
    let dir = arg("--fixture-dir");
    let scenario = arg("--scenario").to_string_lossy().into_owned();
    fs::create_dir_all(&dir).expect("create explicit disposable fixture directory");
    let art = dir.join("art");
    fs::create_dir_all(&art).unwrap();
    let seed_scenario = if scenario == "reopen" {
        "mixed"
    } else {
        &scenario
    };
    let rows = fixture_rows(seed_scenario, &art);
    let settings_path = dir.join("native-fixture.json");
    let settings = load_fixture_settings(&dir);
    fs::write(dir.join("tags.txt"), "fixture-tag\n").unwrap();
    let config = Config {
        publicable: vec!["art".into()],
        tags: Some("tags.txt".into()),
        tiers: settings.tiers.clone(),
        galleries: settings.galleries.clone(),
        ..Config::default()
    };
    let paths = Paths::resolve(Some(&dir), None).unwrap();
    initialize_publications_if_absent(&paths.json, &rows).unwrap();
    let store = Arc::new(Store::new(paths, config).unwrap());
    let (endpoint, stop_http, http_thread) = http_fixture(&dir, settings);
    let llmfit = dir.join("llmfit");
    write_llmfit(&llmfit, &settings_path);
    let worker = dir.join("worker.sh");
    write_worker(&worker);
    let skill = dir.join("skill.md");
    fs::write(&skill, "fixture skill: no remote publication").unwrap();
    let metadata = MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: endpoint.clone(),
            ollama: endpoint.clone(),
        },
        llmfit,
    )
    .with_openrouter_key("DUMMY_NATIVE_ACCEPTANCE_KEY");
    let application = Arc::new(
        Application::new(store, metadata, || 1_800_000_000)
            .with_publisher(Publisher::new(worker, skill)),
    );
    let result = ui::run(
        application,
        UiOptions {
            openrouter_model: None,
            ai_timeout: Duration::from_secs(3),
        },
    );
    stop_http.store(true, Ordering::Release);
    let _ = TcpStream::connect(endpoint.trim_start_matches("http://"));
    let _ = http_thread.join();
    result
}
