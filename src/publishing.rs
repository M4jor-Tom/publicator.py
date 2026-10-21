use crate::{
    application::Application,
    error::{Error, Result},
    images::{index_by_basename, prepare_upload, validate_upload},
    model::{Phase, phase},
    publish_journal::{self, PublishJournal},
    publish_protocol::{PublishJob, ScheduleParts, SessionOpen, WorkerCommand, WorkerEvent},
    scheduling,
    store::Store,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    time::Instant,
};

const MAX_LINE: usize = 1024 * 1024;
// ponytail: fixed per-worker timeouts and a 1 MiB frame cap; tune only when real browser behavior requires it.
#[derive(Clone, Copy)]
struct Timeouts {
    ready: Duration,
    artwork: Duration,
    cleanup: Duration,
}
impl Default for Timeouts {
    fn default() -> Self {
        Self {
            ready: Duration::from_secs(90),
            artwork: Duration::from_secs(600),
            cleanup: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PublishRequest {
    pub uuids: Option<Vec<String>>,
    pub headless: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishReport {
    pub published: usize,
    pub failed: usize,
    pub unknown: usize,
    pub unattempted: usize,
    pub error: Option<String>,
}
pub struct Publisher {
    worker: PathBuf,
    skill: PathBuf,
    timeouts: Timeouts,
}
impl Publisher {
    pub fn new(worker: PathBuf, skill: PathBuf) -> Self {
        Self {
            worker,
            skill,
            timeouts: Timeouts::default(),
        }
    }
    #[cfg(test)]
    fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
    pub async fn check_steps(&self) -> Result<()> {
        let status = Command::new(&self.worker)
            .arg("--check-steps")
            .arg("--skill")
            .arg(&self.skill)
            .status()
            .await?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Validation(
                "publishing steps differ from skill".into(),
            ))
        }
    }
    pub async fn publish(
        &self,
        app: &Application,
        request: PublishRequest,
        cancel: Arc<AtomicBool>,
    ) -> Result<PublishReport> {
        let _lease = app.begin_publishing()?;
        let store = app.store();
        let journal = PublishJournal::new(&store.paths().data_dir)?;
        let _lock = publish_journal::publisher_lock(&store.paths().data_dir)?;
        let recovery = journal.recover(store)?;
        if !recovery.blocked.is_empty() {
            return Err(Error::ReconciliationRequired(
                "unresolved publishing attempts require reconciliation".into(),
            ));
        }
        let rows = store.read()?;
        let targets = pending_targets(&rows, request.uuids.as_deref())?;
        if targets.is_empty() {
            return Ok(PublishReport::default());
        }
        // Validate every selected record and file before opening a browser process.
        let zone = scheduling::timezone(store.config())?;
        let mut batch = Vec::new();
        let mut staged_uploads: Vec<TempDir> = Vec::new();
        for row in targets {
            let uuid = row
                .get("uuid")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Validation("publication UUID missing".into()))?;
            let approved = publish_journal::snapshot(store, uuid)?;
            let basename = row
                .get("files")
                .and_then(Value::as_array)
                .and_then(|f| f.first())
                .and_then(|f| f.get("basename"))
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Validation("artwork basename missing".into()))?;
            let image = index_by_basename(&store.paths().data_dir)?
                .remove(basename)
                .ok_or_else(|| Error::NotFound(format!("artwork {basename}")))?;
            let staging = tempfile::tempdir_in(store.paths().data_dir.join(".publicator"))?;
            let upload = prepare_upload(&image, staging.path())?;
            validate_upload(&upload)?;
            let attempt_id = uuid::Uuid::new_v4().to_string();
            let job = make_job(&approved, attempt_id, upload, zone)?;
            staged_uploads.push(staging);
            job_command_valid(&job)?;
            batch.push((approved, job));
        }
        self.check_steps().await?;
        let mut child = start_worker(&self.worker)?;
        let process_group = child.id();
        let diagnostics = child.stderr.take().map(|mut stderr| {
            tokio::spawn(async move {
                let mut retained = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let count = match stderr.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(count) => count,
                    };
                    let remaining = 8192usize.saturating_sub(retained.len());
                    retained.extend_from_slice(&chunk[..count.min(remaining)]);
                }
                retained
            })
        });
        let result = self
            .run_batch(
                &mut child,
                &journal,
                store,
                &batch,
                request.headless,
                zone,
                self.timeouts,
                &cancel,
            )
            .await;
        shutdown(&mut child, self.timeouts.cleanup, process_group).await;
        let diagnostics = match diagnostics {
            Some(task) => task.await.unwrap_or_default(),
            None => Vec::new(),
        };
        if diagnostics.is_empty() {
            return result;
        }
        let detail = String::from_utf8_lossy(&diagnostics);
        match result {
            Ok(mut report) => {
                if let Some(error) = &mut report.error {
                    error.push_str(&format!("; worker stderr: {}", detail.trim()));
                }
                Ok(report)
            }
            Err(error) => Err(Error::Protocol(format!(
                "{error}; worker stderr: {}",
                detail.trim()
            ))),
        }
    }
    async fn run_batch(
        &self,
        child: &mut Child,
        journal: &PublishJournal,
        store: &Store,
        batch: &[(publish_journal::ApprovedSnapshot, PublishJob)],
        headless: bool,
        zone: chrono_tz::Tz,
        timeouts: Timeouts,
        cancel: &AtomicBool,
    ) -> Result<PublishReport> {
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| Error::Protocol("worker stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Protocol("worker stdout unavailable".into()))?;
        let mut output = BufReader::new(stdout);
        let data = store.paths().data_dir.clone();
        let open = WorkerCommand::Open(SessionOpen {
            protocol: 1,
            login_profile: data.join(".deviantart-login"),
            work_profile: data.join(".deviantart-session"),
            headless,
            timezone: zone.to_string(),
        });
        let ready_deadline = Instant::now() + timeouts.ready;
        tokio::time::timeout_at(ready_deadline, send_line(&mut input, &open))
            .await
            .map_err(|_| Error::Protocol("worker open timeout".into()))??;
        match tokio::time::timeout_at(ready_deadline, read_event(&mut output))
            .await
            .map_err(|_| Error::Protocol("worker ready timeout".into()))??
        {
            WorkerEvent::Ready { protocol: 1 } => {}
            WorkerEvent::Fatal { error, .. } => return Err(Error::Protocol(error)),
            _ => return Err(Error::Protocol("worker did not become ready".into())),
        }
        let mut report = PublishReport::default();
        for (approved, job) in batch {
            if cancel.load(Ordering::SeqCst) {
                report.unattempted +=
                    batch.len() - report.published - report.failed - report.unknown;
                break;
            }
            let fresh = publish_journal::snapshot(store, &job.uuid)?;
            if fresh != *approved {
                report.unattempted +=
                    batch.len() - report.published - report.failed - report.unknown;
                break;
            }
            journal.begin(job, approved)?;
            let deadline = Instant::now() + timeouts.artwork;
            let send_result = match tokio::time::timeout_at(
                deadline,
                send_line(&mut input, &WorkerCommand::Submit(job.clone())),
            )
            .await
            {
                Ok(result) => result,
                Err(error) => Err(Error::Protocol(format!(
                    "worker submit write timeout; result is unknown: {error}"
                ))),
            };
            if let Err(error) = send_result {
                report.unknown += 1;
                report.error = Some(error.to_string());
                report.unattempted +=
                    batch.len() - report.published - report.failed - report.unknown;
                break;
            }
            let event = match tokio::time::timeout_at(
                deadline,
                read_matching_terminal(&mut output, job, deadline),
            )
            .await
            {
                Ok(Ok(event)) => event,
                Ok(Err(e)) => {
                    report.unknown += 1;
                    report.error = Some(e.to_string());
                    report.unattempted +=
                        batch.len() - report.published - report.failed - report.unknown;
                    break;
                }
                Err(_) => {
                    report.unknown += 1;
                    report.error = Some("worker artwork timeout; result is unknown".into());
                    report.unattempted +=
                        batch.len() - report.published - report.failed - report.unknown;
                    break;
                }
            };
            journal.terminal(&event)?;
            match event {
                WorkerEvent::Scheduled { attempt_id, .. } => {
                    journal.apply_scheduled(store, &attempt_id)?;
                    report.published += 1;
                }
                WorkerEvent::Failed { error, .. } => {
                    report.failed += 1;
                    report.error = Some(error);
                    report.unattempted +=
                        batch.len() - report.published - report.failed - report.unknown;
                    break;
                }
                WorkerEvent::Unknown { error, .. } => {
                    report.unknown += 1;
                    report.error = Some(error);
                    report.unattempted +=
                        batch.len() - report.published - report.failed - report.unknown;
                    break;
                }
                _ => return Err(Error::Protocol("unexpected worker terminal".into())),
            }
        }
        let _ = tokio::time::timeout(
            timeouts.cleanup,
            send_line(&mut input, &WorkerCommand::Close { protocol: 1 }),
        )
        .await;
        Ok(report)
    }
}
impl Application {
    pub fn with_publisher(mut self, publisher: Publisher) -> Self {
        self.publisher = Some(publisher);
        self
    }
    pub async fn publish(
        &self,
        request: PublishRequest,
        cancel: Arc<AtomicBool>,
    ) -> Result<PublishReport> {
        let publisher = self
            .publisher
            .as_ref()
            .ok_or_else(|| Error::InvalidState("publisher is not configured".into()))?;
        publisher.publish(self, request, cancel).await
    }
}
pub fn first_pending_uuid(rows: &[Value]) -> Result<Option<String>> {
    Ok(pending_targets(rows, None)?
        .first()
        .and_then(|row| row.get("uuid"))
        .and_then(Value::as_str)
        .map(str::to_owned))
}

fn pending_targets<'a>(rows: &'a [Value], uuids: Option<&[String]>) -> Result<Vec<&'a Value>> {
    let mut selected = Vec::new();
    for row in rows {
        let uuid = row
            .get("uuid")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Validation("publication UUID missing".into()))?;
        if uuids.is_some_and(|ids| !ids.iter().any(|id| id == uuid)) {
            continue;
        }
        let executable = matches!(phase(row)?, Phase::Described | Phase::Appeared)
            && row
                .get("apparitions")
                .and_then(Value::as_array)
                .is_some_and(|apps| {
                    apps.iter().any(|a| {
                        a.get("platformName").and_then(Value::as_str) == Some("deviantart")
                            && a.get("state").and_then(Value::as_str) == Some("unpublished")
                    })
                });
        if executable {
            selected.push(row);
        }
    }
    if let Some(ids) = uuids {
        for id in ids {
            if !rows
                .iter()
                .any(|r| r.get("uuid").and_then(Value::as_str) == Some(id))
            {
                return Err(Error::NotFound(id.clone()));
            }
        }
    }
    Ok(selected)
}
fn make_job(
    a: &publish_journal::ApprovedSnapshot,
    attempt_id: String,
    image_path: PathBuf,
    zone: chrono_tz::Tz,
) -> Result<PublishJob> {
    let dt = chrono::DateTime::from_timestamp(a.schedule_epoch, 0)
        .ok_or_else(|| Error::Validation("invalid schedule epoch".into()))?
        .with_timezone(&zone);
    let job = PublishJob {
        protocol: 1,
        attempt_id,
        uuid: a.uuid.clone(),
        image_path,
        title: a.title.clone(),
        description: a.description.clone(),
        tags: a.tags.clone(),
        schedule: ScheduleParts {
            epoch: a.schedule_epoch,
            timezone: zone.to_string(),
            year: dt.year(),
            month: dt.month(),
            day: dt.day(),
            hour: dt.hour(),
            minute: dt.minute(),
        },
        price: a.price,
        tiers: a.tiers.clone(),
        galleries: a.galleries.clone(),
    };
    Ok(job)
}
fn job_command_valid(job: &PublishJob) -> Result<()> {
    let command = WorkerCommand::Submit(job.clone());
    command.validate().map_err(Error::Validation)?;
    encode_line(&command).map(|_| ())
}
fn start_worker(path: &Path) -> Result<Child> {
    let mut c = Command::new(path);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    c.process_group(0);
    Ok(c.spawn()?)
}
fn encode_line(command: &WorkerCommand) -> Result<Vec<u8>> {
    let mut line = serde_json::to_vec(command)?;
    if line.len() + 1 > MAX_LINE {
        return Err(Error::Protocol("worker command exceeds 1 MiB".into()));
    }
    line.push(b'\n');
    Ok(line)
}
async fn send_line(w: &mut (impl AsyncWrite + Unpin), command: &WorkerCommand) -> Result<()> {
    let line = encode_line(command)?;
    w.write_all(&line).await?;
    w.flush().await?;
    Ok(())
}
async fn read_event(r: &mut (impl AsyncBufRead + Unpin)) -> Result<WorkerEvent> {
    let mut line = Vec::new();
    let n = r
        .take((MAX_LINE + 1) as u64)
        .read_until(b'\n', &mut line)
        .await?;
    if n == 0 || n > MAX_LINE || line.last() != Some(&b'\n') {
        return Err(Error::Protocol("invalid or overlong worker frame".into()));
    }
    serde_json::from_slice(&line).map_err(|e| Error::Protocol(e.to_string()))
}
async fn read_matching_terminal(
    r: &mut (impl AsyncBufRead + Unpin),
    job: &PublishJob,
    deadline: Instant,
) -> Result<WorkerEvent> {
    loop {
        let event = tokio::time::timeout_at(deadline, read_event(r))
            .await
            .map_err(|_| Error::Protocol("worker artwork timeout".into()))??;
        match &event {
            WorkerEvent::Scheduled {
                protocol,
                attempt_id,
                uuid,
                ..
            }
            | WorkerEvent::Failed {
                protocol,
                attempt_id,
                uuid,
                ..
            }
            | WorkerEvent::Unknown {
                protocol,
                attempt_id,
                uuid,
                ..
            } if *protocol == 1 && attempt_id == &job.attempt_id && uuid == &job.uuid => {
                return Ok(event);
            }
            _ => {
                return Err(Error::Protocol(
                    "worker terminal identity/version mismatch".into(),
                ));
            }
        }
    }
}
async fn shutdown(child: &mut Child, cleanup_timeout: Duration, process_group: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = process_group {
        // The child is the process-group leader; signal the group so browser descendants
        // receive the same bounded graceful-then-forced cleanup.
        unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        let _ = tokio::time::timeout(cleanup_timeout, child.wait()).await;
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        let _ = child.wait().await;
        return;
    }
    let _ = tokio::time::timeout(cleanup_timeout, child.wait()).await;
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(cleanup_timeout, child.wait()).await;
    }
}
use chrono::{Datelike, Timelike};

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use crate::{
        config::{Config, Paths},
        metadata::{Endpoints, MetadataClient},
    };
    use serde_json::json;
    use std::{os::unix::fs::PermissionsExt, sync::Arc};

    #[tokio::test]
    async fn cleanup_force_kills_worker_that_ignores_termination() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("art.png");
        assert!(
            std::process::Command::new("magick")
                .args(["-size", "4x3", "xc:white"])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
        let sha = crate::images::file_sha512(&image).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        std::fs::write(root.path().join("publications.json"), serde_json::to_vec(&json!([{
            "uuid":id,"state":"DESCRIBED","title":"Title","description":"Description",
            "submissionTimestamp":1_700_002_800,"apparitionTimestampIfDifferentThanSubmission":1_700_002_800,
            "files":[{"basename":"art.png","sha512sum":sha}],
            "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
        }])).unwrap()).unwrap();
        std::fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
        let store = Arc::new(
            crate::store::Store::new(
                Paths::resolve(Some(root.path()), None).unwrap(),
                Config {
                    tags: Some("tags.txt".into()),
                    ..Config::default()
                },
            )
            .unwrap(),
        );
        let application = Application::new(
            store,
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints {
                    openrouter: "http://127.0.0.1:1".into(),
                    ollama: "http://127.0.0.1:1".into(),
                },
                "llmfit".into(),
            ),
            || 0,
        );
        let worker = root.path().join("worker.sh");
        let pid_file = root.path().join("pid");
        std::fs::write(&worker, format!(r#"#!/bin/sh
[ "$1" = "--check-steps" ] && exit 0
echo $$ > '{}'
while IFS= read -r line; do
case "$line" in
*'"type":"open"'*) echo '{{"protocol":1,"type":"ready"}}' ;;
*'"type":"submit"'*) printf '%s' "$line" | jq -c '{{protocol:1,type:"failed",attempt_id,uuid,error:"safe"}}'; trap '' TERM; while :; do :; done ;;
esac
done"#, pid_file.display())).unwrap();
        let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&worker, permissions).unwrap();
        let publisher =
            Publisher::new(worker, root.path().join("skill.md")).with_timeouts(Timeouts {
                ready: Duration::from_millis(250),
                artwork: Duration::from_millis(250),
                cleanup: Duration::from_millis(30),
            });
        let report = publisher
            .publish(
                &application,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
        assert_eq!(report.failed, 1);
        let pid = std::fs::read_to_string(pid_file).unwrap();
        assert!(std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())).is_err());
    }

    #[tokio::test]
    async fn worker_that_never_becomes_ready_obeys_ready_deadline() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("art.png");
        assert!(
            std::process::Command::new("magick")
                .args(["-size", "4x3", "xc:white"])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
        let sha = crate::images::file_sha512(&image).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        std::fs::write(root.path().join("publications.json"), serde_json::to_vec(&json!([{
            "uuid":id,"state":"DESCRIBED","title":"Title","description":"Description",
            "submissionTimestamp":1_700_002_800,"apparitionTimestampIfDifferentThanSubmission":1_700_002_800,
            "files":[{"basename":"art.png","sha512sum":sha}],
            "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
        }])).unwrap()).unwrap();
        std::fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
        let store = Arc::new(
            crate::store::Store::new(
                Paths::resolve(Some(root.path()), None).unwrap(),
                Config {
                    tags: Some("tags.txt".into()),
                    ..Config::default()
                },
            )
            .unwrap(),
        );
        let application = Application::new(
            store,
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints {
                    openrouter: "http://127.0.0.1:1".into(),
                    ollama: "http://127.0.0.1:1".into(),
                },
                "llmfit".into(),
            ),
            || 0,
        );
        let worker = root.path().join("worker.sh");
        std::fs::write(&worker, "#!/bin/sh\n[ \"$1\" = \"--check-steps\" ] && exit 0\nIFS= read -r open || exit 1\nsleep 10\n").unwrap();
        let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&worker, permissions).unwrap();
        let publisher =
            Publisher::new(worker, root.path().join("skill.md")).with_timeouts(Timeouts {
                ready: Duration::from_millis(50),
                artwork: Duration::from_millis(250),
                cleanup: Duration::from_millis(50),
            });
        let error = publisher
            .publish(
                &application,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("worker ready timeout"));
    }

    #[tokio::test]
    async fn terminal_read_timeout_keeps_attempt_unknown_and_blocks_retry() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("art.png");
        assert!(
            std::process::Command::new("magick")
                .args(["-size", "4x3", "xc:white"])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
        let sha = crate::images::file_sha512(&image).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        std::fs::write(root.path().join("publications.json"), serde_json::to_vec(&json!([{
            "uuid":id,"state":"DESCRIBED","title":"Title","description":"Description",
            "submissionTimestamp":1_700_002_800,"apparitionTimestampIfDifferentThanSubmission":1_700_002_800,
            "files":[{"basename":"art.png","sha512sum":sha}],
            "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
        }])).unwrap()).unwrap();
        std::fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
        let store = Arc::new(
            crate::store::Store::new(
                Paths::resolve(Some(root.path()), None).unwrap(),
                Config {
                    tags: Some("tags.txt".into()),
                    ..Config::default()
                },
            )
            .unwrap(),
        );
        let application = Application::new(
            store,
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints {
                    openrouter: "http://127.0.0.1:1".into(),
                    ollama: "http://127.0.0.1:1".into(),
                },
                "llmfit".into(),
            ),
            || 0,
        );
        let worker = root.path().join("worker.sh");
        std::fs::write(&worker, "#!/bin/sh\n[ \"$1\" = \"--check-steps\" ] && exit 0\nIFS= read -r open || exit 1\necho '{\"protocol\":1,\"type\":\"ready\"}'\nIFS= read -r submit || exit 1\nsleep 10\n").unwrap();
        let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&worker, permissions).unwrap();
        let publisher =
            Publisher::new(worker, root.path().join("skill.md")).with_timeouts(Timeouts {
                ready: Duration::from_millis(250),
                artwork: Duration::from_millis(50),
                cleanup: Duration::from_millis(50),
            });
        let report = publisher
            .publish(
                &application,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
        assert_eq!(report.unknown, 1);
        assert!(report.error.unwrap().contains("artwork timeout"));
        let retry = publisher
            .publish(
                &application,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await;
        assert!(matches!(retry, Err(Error::ReconciliationRequired(_))));
    }

    #[tokio::test]
    async fn large_submit_to_nonreading_worker_times_out_with_unknown_intent() {
        let root = tempfile::tempdir().unwrap();
        let image = root.path().join("art.png");
        assert!(
            std::process::Command::new("magick")
                .args(["-size", "4x3", "xc:white"])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
        let sha = crate::images::file_sha512(&image).unwrap();
        let id = "00000000-0000-4000-8000-000000000001";
        let publications = root.path().join("publications.json");
        std::fs::write(&publications, serde_json::to_vec(&json!([{
            "uuid": id, "state":"DESCRIBED", "title":"Title",
            "description":"d".repeat(900_000), "submissionTimestamp":1_700_002_800,
            "apparitionTimestampIfDifferentThanSubmission":1_700_002_800,
            "files":[{"basename":"art.png","sha512sum":sha}],
            "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
        }])).unwrap()).unwrap();
        std::fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
        let store = Arc::new(
            crate::store::Store::new(
                Paths::resolve(Some(root.path()), None).unwrap(),
                Config {
                    tags: Some("tags.txt".into()),
                    ..Config::default()
                },
            )
            .unwrap(),
        );
        let application = Application::new(
            store,
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints {
                    openrouter: "http://127.0.0.1:1".into(),
                    ollama: "http://127.0.0.1:1".into(),
                },
                "llmfit".into(),
            ),
            || 0,
        );
        let worker = root.path().join("worker.sh");
        std::fs::write(&worker, "#!/bin/sh\n[ \"$1\" = \"--check-steps\" ] && exit 0\nIFS= read -r open || exit 1\necho '{\"protocol\":1,\"type\":\"ready\"}'\nsleep 10\n").unwrap();
        let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&worker, permissions).unwrap();
        let publisher =
            Publisher::new(worker, root.path().join("skill.md")).with_timeouts(Timeouts {
                ready: Duration::from_millis(250),
                artwork: Duration::from_millis(50),
                cleanup: Duration::from_millis(100),
            });
        let report = publisher
            .publish(
                &application,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
        assert_eq!(report.unknown, 1);
        assert!(report.error.unwrap().contains("write timeout"));
    }
}
