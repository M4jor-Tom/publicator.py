mod common;
use common::{Fixture, row};
use publicator::{
    application::Application,
    metadata::{Endpoints, MetadataClient},
    publishing::{PublishRequest, Publisher},
};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn app(f: &Fixture) -> Application {
    app_with_timezone(f, None)
}
fn app_with_timezone(f: &Fixture, timezone: Option<&str>) -> Application {
    std::fs::write(f.path("tags.txt"), "one\ntwo\n").unwrap();
    let mut config = publicator::config::Config::default();
    config.tags = Some("tags.txt".into());
    if let Some(timezone) = timezone {
        config
            .schedule
            .insert("timezone".into(), toml::Value::String(timezone.into()));
    }
    Application::new(
        Arc::new(
            publicator::store::Store::new(
                publicator::config::Paths::resolve(Some(f.root.path()), None).unwrap(),
                config,
            )
            .unwrap(),
        ),
        MetadataClient::new(
            reqwest::Client::new(),
            Endpoints {
                openrouter: "http://127.0.0.1:1".into(),
                ollama: "http://127.0.0.1:1".into(),
            },
            "llmfit".into(),
        ),
        || 0,
    )
}
#[tokio::test]
async fn historical_only_publications_do_not_start_worker() {
    let f = Fixture::new();
    f.write(json!([row(1, "APPEARED")]));
    let app = app(&f);
    let publisher = Publisher::new(
        PathBuf::from("/does/not/exist"),
        PathBuf::from("/does/not/exist"),
    );
    let report = publisher
        .publish(
            &app,
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.published, 0);
    assert_eq!(report.failed, 0);
    assert_eq!(report.unknown, 0);
    assert_eq!(report.unattempted, 0);
}
fn configured_fixture() -> Fixture {
    let f = Fixture::new();
    let image = f.image("1.png", 4, 3);
    let mut item = row(1, "DESCRIBED");
    item["files"][0]["basename"] = json!("1.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] = json!([
        {"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]},
        {"platformName":"historical-platform","state":"unpublished"}
    ]);
    f.write(json!([item]));
    f
}
fn fake_worker(f: &Fixture, body: &str) -> PathBuf {
    let path = f.path("worker.sh");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then exit 0; fi\n{body}\n"),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}
#[tokio::test]
async fn mixed_case_webp_is_converted_before_worker_start_submission() {
    let f = Fixture::new();
    let image = f.image("1.WEBP", 4, 3);
    let mut item = row(1, "DESCRIBED");
    item["files"][0]["basename"] = json!("1.WEBP");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    f.write(json!([item]));
    std::fs::write(f.path("tags.txt"), "tag\n").unwrap();
    let log = f.path("upload-path.txt");
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) path=$(printf '%s' "$line" | jq -r .image_path); identify -format '%m' "$path" > '{}'; echo >> '{}'; echo "$path" >> '{}'; printf '%s' "$line" | jq -c '{{protocol:1,type:"scheduled",attempt_id,uuid}}' ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            log.display(),
            log.display(),
            log.display()
        ),
    );
    let report = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.published, 1);
    let lines = std::fs::read_to_string(log).unwrap();
    assert!(lines.lines().last().unwrap().ends_with(".png"));
    assert_eq!(lines.lines().next(), Some("PNG"));
}
#[tokio::test]
async fn unsupported_artwork_fails_preflight_before_worker_start() {
    let f = Fixture::new();
    let image = f.path("clip.mp4");
    std::fs::write(&image, b"not a supported media file").unwrap();
    let mut item = row(1, "DESCRIBED");
    item["files"][0]["basename"] = json!("clip.mp4");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    f.write(json!([item]));
    let marker = f.path("started");
    let worker = fake_worker(&f, &format!("touch '{}'", marker.display()));
    let result = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert!(result.is_err());
    assert!(!marker.exists());
}
#[tokio::test]
async fn step_check_failure_prevents_worker_session_start() {
    let f = configured_fixture();
    let checked = f.path("checked");
    let started = f.path("started");
    let worker = f.path("worker.sh");
    std::fs::write(
        &worker,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--check-steps\" ]; then touch '{}'; exit 1; fi\ntouch '{}'\n",
            checked.display(),
            started.display()
        ),
    ).unwrap();
    let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&worker, permissions).unwrap();
    let result = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert!(result.is_err());
    assert!(checked.exists());
    assert!(!started.exists());
}
#[tokio::test]
async fn publisher_uses_configured_timezone_for_session_and_schedule_parts() {
    let f = configured_fixture();
    let log = f.path("commands.jsonl");
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo "$line" >> '{}'; echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) echo "$line" >> '{}'; printf '%s' "$line" | jq -c '{{protocol:1,type:"scheduled",attempt_id,uuid}}' ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            log.display(),
            log.display()
        ),
    );
    let app = app_with_timezone(&f, Some("America/New_York"))
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let report = app
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.published, 1);
    let commands: Vec<serde_json::Value> = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(commands[0]["timezone"], "America/New_York");
    assert_eq!(
        commands[1]["schedule"],
        json!({
            "epoch": 1_700_002_800, "timezone": "America/New_York",
            "year": 2023, "month": 11, "day": 14, "hour": 18, "minute": 0
        })
    );
}
#[tokio::test]
async fn worker_stderr_is_drained_under_pressure_and_retained_in_report() {
    let f = configured_fixture();
    let worker = fake_worker(
        &f,
        r#"while IFS= read -r line; do
case "$line" in *'"type":"open"'*) head -c 200000 /dev/zero | tr '\000' E >&2; echo '{"protocol":1,"type":"ready"}' ;;
*'"type":"submit"'*) attempt=$(printf '%s' "$line" | sed -n 's/.*"attempt_id":"\([^"]*\)".*/\1/p'); uuid=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p'); printf '{"protocol":1,"type":"failed","attempt_id":"%s","uuid":"%s","error":"fake"}\n' "$attempt" "$uuid";;
*'"type":"close"'*) exit 0;; esac
done"#,
    );
    let report = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 1);
    let error = report.error.unwrap();
    assert!(error.contains('E'));
    assert!(error.len() <= 8300);
}
#[tokio::test]
async fn shutdown_terminates_worker_descendants() {
    let f = configured_fixture();
    let pid_file = f.path("descendant.pid");
    let worker = fake_worker(
        &f,
        &format!(
            r#"sleep 60 >/dev/null 2>&1 &
echo $! > '{}'
while IFS= read -r line; do
case "$line" in *'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) attempt=$(printf '%s' "$line" | sed -n 's/.*"attempt_id":"\([^"]*\)".*/\1/p'); uuid=$(printf '%s' "$line" | sed -n 's/.*"uuid":"\([^"]*\)".*/\1/p'); printf '{{"protocol":1,"type":"failed","attempt_id":"%s","uuid":"%s","error":"fake"}}\n' "$attempt" "$uuid";;
*'"type":"close"'*) exit 0;; esac
done"#,
            pid_file.display()
        ),
    );
    let report = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 1);
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
    assert!(
        stat.as_ref()
            .is_none_or(|stat| stat.split_whitespace().nth(2) == Some("Z")),
        "descendant still running: {stat:?}"
    );
}
#[tokio::test]
async fn publisher_persists_success_before_next_and_retry_skips_completed_delivery() {
    let f = configured_fixture();
    let second = f.image("2.png", 4, 3);
    let mut item = row(2, "DESCRIBED");
    item["files"][0]["basename"] = json!("2.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&second).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    let mut rows = f.read();
    rows.as_array_mut().unwrap().push(item);
    let third = f.image("3.png", 4, 3);
    let mut item = row(3, "DESCRIBED");
    item["files"][0]["basename"] = json!("3.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&third).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    rows.as_array_mut().unwrap().push(item);
    f.write(rows);
    let log = f.path("calls.log");
    let receipts = f.path(".publicator/publish");
    let first_uuid = common::id(1);
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
kind=$(printf '%s' "$line" | jq -r .type)
case "$kind" in
open) echo '{{"protocol":1,"type":"ready"}}' ;;
submit)
  uuid=$(printf '%s' "$line" | jq -r .uuid)
  attempt=$(printf '%s' "$line" | jq -r .attempt_id)
  echo "$uuid" >> '{}'
  if [ "$uuid" = '{}' ]; then
    printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s"}}\n' "$attempt" "$uuid"
  else
    for receipt in '{}'/*/receipt.json; do
      if jq -e --arg id '{}' '.applied == true and .job.uuid == $id' "$receipt" >/dev/null; then echo persisted-before-next >> '{}'; break; fi
    done
    printf '{{"protocol":1,"type":"failed","attempt_id":"%s","uuid":"%s","error":"safe failure"}}\n' "$attempt" "$uuid"
  fi ;;
close) exit 0 ;;
esac
done"#,
            log.display(),
            first_uuid,
            receipts.display(),
            first_uuid,
            log.display()
        ),
    );
    let app = app(&f).with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let request = || PublishRequest {
        uuids: None,
        headless: true,
    };
    let cancel = || Arc::new(AtomicBool::new(false));
    let first = app.publish(request(), cancel()).await.unwrap();
    assert_eq!(
        (first.published, first.failed, first.unattempted),
        (1, 1, 1)
    );
    assert_eq!(
        f.read()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert_eq!(f.read()[0]["apparitions"][1]["state"], "unpublished");
    assert_eq!(f.read()[1]["apparitions"][0]["state"], "unpublished");
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("persisted-before-next")
    );
    let retry = app.publish(request(), cancel()).await.unwrap();
    assert_eq!((retry.published, retry.failed), (0, 1));
    let calls: Vec<_> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        calls.iter().filter(|call| **call == common::id(1)).count(),
        1
    );
    assert_eq!(
        calls.iter().filter(|call| **call == common::id(2)).count(),
        2
    );
    assert_eq!(
        calls.iter().filter(|call| **call == common::id(3)).count(),
        0
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.as_str() == "persisted-before-next")
            .count(),
        2
    );
}
#[tokio::test]
async fn unknown_worker_outcome_is_not_resent_on_retry() {
    let f = configured_fixture();
    let second = f.image("2.png", 4, 3);
    let mut item = row(2, "DESCRIBED");
    item["files"][0]["basename"] = json!("2.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&second).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    let mut rows = f.read();
    rows.as_array_mut().unwrap().push(item);
    f.write(rows);
    let calls = f.path("calls.log");
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) echo sent >> '{}'; exit 0 ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            calls.display()
        ),
    );
    let app = app(&f).with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let request = || PublishRequest {
        uuids: None,
        headless: true,
    };
    let first = app
        .publish(request(), Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert_eq!(first.unknown, 1);
    assert_eq!(first.unattempted, 1);
    assert!(
        app.publish(request(), Arc::new(AtomicBool::new(false)))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(), 1);
}
#[tokio::test]
async fn duplicate_terminal_from_previous_job_blocks_the_next_job_as_unknown() {
    let f = configured_fixture();
    let second = f.image("2.png", 4, 3);
    let mut item = row(2, "DESCRIBED");
    item["files"][0]["basename"] = json!("2.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&second).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    let mut rows = f.read();
    rows.as_array_mut().unwrap().push(item);
    f.write(rows);
    let first_id = common::id(1);
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) uuid=$(printf '%s' "$line" | jq -r .uuid); event=$(printf '%s' "$line" | jq -c '{{protocol:1,type:"scheduled",attempt_id,uuid}}'); echo "$event"; if [ "$uuid" = '{}' ]; then echo "$event"; fi ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            first_id
        ),
    );
    let report = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
    assert_eq!(
        (report.published, report.unknown, report.unattempted),
        (1, 1, 0)
    );
    assert_eq!(
        f.read()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert_eq!(f.read()[1]["apparitions"][0]["state"], "unpublished");
}
#[tokio::test]
async fn publisher_store_failure_after_terminal_keeps_unapplied_receipt() {
    let f = configured_fixture();
    let database = f.store.paths().json.clone();
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) printf 'malformed' > '{}'; printf '%s' "$line" | jq -c '{{protocol:1,type:"scheduled",attempt_id,uuid}}' ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            database.display()
        ),
    );
    let result = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert!(result.is_err());
    let mut receipts = std::fs::read_dir(f.path(".publicator/publish")).unwrap();
    let receipt_file = receipts
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("receipt.json");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipt_file).unwrap()).unwrap();
    assert_eq!(receipt["terminal"]["type"], "scheduled");
    assert_eq!(receipt["applied"], false);
}
#[tokio::test]
async fn overlong_worker_terminal_is_unknown_and_not_retried() {
    let f = configured_fixture();
    let worker = fake_worker(
        &f,
        r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{"protocol":1,"type":"ready"}' ;;
*'"type":"submit"'*) head -c 1048577 /dev/zero | tr '\000' x ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
    );
    let app = app(&f).with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let request = || PublishRequest {
        uuids: None,
        headless: true,
    };
    assert_eq!(
        app.publish(request(), Arc::new(AtomicBool::new(false)))
            .await
            .unwrap()
            .unknown,
        1
    );
    assert!(
        app.publish(request(), Arc::new(AtomicBool::new(false)))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn malformed_worker_terminal_is_unknown_and_not_retried() {
    let f = configured_fixture();
    let calls = f.path("calls.log");
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) echo called >> '{}'; echo 'not-json' ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            calls.display()
        ),
    );
    let app = app(&f).with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let request = || PublishRequest {
        uuids: None,
        headless: true,
    };
    assert_eq!(
        app.publish(request(), Arc::new(AtomicBool::new(false)))
            .await
            .unwrap()
            .unknown,
        1
    );
    assert!(
        app.publish(request(), Arc::new(AtomicBool::new(false)))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(), 1);
}
#[tokio::test]
async fn mismatched_worker_terminal_is_unknown_and_not_retried() {
    let f = configured_fixture();
    let worker = fake_worker(
        &f,
        r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{"protocol":1,"type":"ready"}' ;;
*'"type":"submit"'*) echo '{"protocol":1,"type":"failed","attempt_id":"00000000-0000-4000-8000-000000000099","uuid":"00000000-0000-4000-8000-000000000001","error":"wrong attempt"}' ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
    );
    let app = app(&f).with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let request = || PublishRequest {
        uuids: None,
        headless: true,
    };
    let report = app
        .publish(request(), Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert_eq!(report.unknown, 1);
    assert!(
        app.publish(request(), Arc::new(AtomicBool::new(false)))
            .await
            .is_err()
    );
}
#[test]
fn publisher_lock_holder_child() {
    let Ok(root) = std::env::var("PUBLICATOR_PUBLISH_LOCK_ROOT") else {
        return;
    };
    let ready = PathBuf::from(std::env::var("PUBLICATOR_PUBLISH_LOCK_READY").unwrap());
    let release = PathBuf::from(std::env::var("PUBLICATOR_PUBLISH_LOCK_RELEASE").unwrap());
    let _lock = publicator::publish_journal::publisher_lock(std::path::Path::new(&root)).unwrap();
    std::fs::write(ready, "held").unwrap();
    while !release.exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[tokio::test]
async fn another_process_publisher_lock_prevents_worker_start() {
    let f = configured_fixture();
    let ready = f.path("lock-ready");
    let release = f.path("lock-release");
    let mut holder = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "publisher_lock_holder_child", "--nocapture"])
        .env("PUBLICATOR_PUBLISH_LOCK_ROOT", f.root.path())
        .env("PUBLICATOR_PUBLISH_LOCK_READY", &ready)
        .env("PUBLICATOR_PUBLISH_LOCK_RELEASE", &release)
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if ready.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let marker = f.path("started");
    let worker = fake_worker(&f, &format!("touch '{}'", marker.display()));
    let result = app(&f)
        .with_publisher(Publisher::new(worker, PathBuf::from("unused")))
        .publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    std::fs::write(release, "release").unwrap();
    assert!(holder.wait().unwrap().success());
    assert!(ready.exists());
    assert!(matches!(result, Err(publicator::error::Error::Busy(_))));
    assert!(!marker.exists());
}
#[tokio::test]
async fn cancellation_finishes_current_artwork_and_leaves_later_target_unattempted() {
    let f = configured_fixture();
    let second = f.image("2.png", 4, 3);
    let mut item = row(2, "DESCRIBED");
    item["files"][0]["basename"] = json!("2.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&second).unwrap());
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] =
        json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]);
    let mut rows = f.read();
    rows.as_array_mut().unwrap().push(item);
    f.write(rows);
    let marker = f.path("in-submit");
    let worker = fake_worker(
        &f,
        &format!(
            r#"while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) touch '{}'; sleep 0.2; printf '%s' "$line" | jq -c '{{protocol:1,type:"scheduled",attempt_id,uuid}}' ;;
*'"type":"close"'*) exit 0 ;;
esac
done"#,
            marker.display()
        ),
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let app = app(&f).with_publisher(Publisher::new(worker, PathBuf::from("unused")));
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        app.publish(
            PublishRequest {
                uuids: None,
                headless: true,
            },
            task_cancel,
        )
        .await
    });
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(marker.exists());
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let report = task.await.unwrap().unwrap();
    assert_eq!((report.published, report.unattempted), (1, 1));
}
#[tokio::test]
async fn publishing_requires_configured_publisher_on_application() {
    let f = Fixture::new();
    let result = app(&f)
        .publish(
            PublishRequest {
                uuids: None,
                headless: false,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert!(result.is_err());
}
