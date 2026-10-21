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
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

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
    eprintln!("usage: ui_acceptance --fixture-dir DIR --scenario empty|history|mixed");
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

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).expect("write fixture executable");
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn http_fixture(dir: &Path) -> (String, Arc<AtomicBool>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local HTTP fixture");
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let log = dir.join("http-calls.jsonl");
    let task = thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => respond(&mut stream, &log),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    });
    (endpoint, stop, task)
}

fn respond(stream: &mut TcpStream, log: &Path) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = [0u8; 8192];
    let count = stream.read(&mut request).unwrap_or(0);
    let text = String::from_utf8_lossy(&request[..count]);
    let path = text.lines().next().unwrap_or_default().to_owned();
    let body = if path.contains("/api/tags") {
        r#"{"models":[{"name":"gemma4:26b-a4b"}]}"#
    } else {
        "{}"
    };
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    writeln!(file, "{}", serde_json::to_string(&path).unwrap()).unwrap();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
}

fn main() -> iced::Result {
    let dir = arg("--fixture-dir");
    let scenario = arg("--scenario").to_string_lossy().into_owned();
    fs::create_dir_all(&dir).expect("create explicit disposable fixture directory");
    let art = dir.join("art");
    fs::create_dir_all(&art).unwrap();
    let rows = fixture_rows(&scenario, &art);
    let config = Config {
        publicable: vec!["art".into()],
        ..Config::default()
    };
    let paths = Paths::resolve(Some(&dir), None).unwrap();
    let store = Arc::new(Store::new(paths, config).unwrap());
    fs::write(
        &store.paths().json,
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    let (endpoint, stop_http, http_thread) = http_fixture(&dir);
    let llmfit = dir.join("llmfit");
    write_executable(
        &llmfit,
        "#!/bin/sh\necho '{\"fits\":true,\"fit\":true,\"level\":\"good\",\"reason\":\"fixture\"}'\n",
    );
    let worker = dir.join("worker.sh");
    write_executable(
        &worker,
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--check-steps" ]; then exit 0; fi
while IFS= read -r line; do
  kind=$(printf '%s' "$line" | jq -r .type)
  case "$kind" in
    open) echo '{{"protocol":1,"type":"ready"}}' ;;
    submit)
      printf '%s\n' "$line" >> '{}'
      count=$(wc -l < '{}')
      attempt=$(printf '%s' "$line" | jq -r .attempt_id)
      uuid=$(printf '%s' "$line" | jq -r .uuid)
      if [ "$count" -eq 1 ]; then
        printf '{{"protocol":1,"type":"failed","attempt_id":"%s","uuid":"%s","reason":"fixture failure"}}\n' "$attempt" "$uuid"
      elif [ "$count" -eq 2 ]; then
        printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/fixture/%s"}}\n' "$attempt" "$uuid" "$uuid"
      else
        printf '{{"protocol":1,"type":"ambiguous","attempt_id":"%s","uuid":"%s"}}\n' "$attempt" "$uuid"
      fi
      ;;
    close) exit 0 ;;
  esac
done
"#,
            dir.join("worker-calls.jsonl").display(),
            dir.join("worker-calls.jsonl").display()
        ),
    );
    let skill = dir.join("skill.md");
    fs::write(&skill, "fixture skill: no remote publication").unwrap();
    let metadata = MetadataClient::new(
        reqwest::Client::new(),
        Endpoints {
            openrouter: endpoint.clone(),
            ollama: endpoint.clone(),
        },
        llmfit,
    );
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
