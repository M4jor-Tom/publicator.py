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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PublishCell {
    pub uuid: String,
    pub platform: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublishCellState {
    Success,
    SafeFailed,
    Unknown,
    Unattempted,
    ConfirmedNotSubmitted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishCellOutcome {
    pub cell: PublishCell,
    pub state: PublishCellState,
    pub detail: Option<String>,
}

impl PublishCellOutcome {
    pub fn retryable(&self) -> bool {
        matches!(
            &self.state,
            PublishCellState::SafeFailed
                | PublishCellState::Unattempted
                | PublishCellState::ConfirmedNotSubmitted
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishCompletion {
    pub result: std::result::Result<PublishReport, String>,
    pub diagnostics: Option<String>,
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
    pub outcomes: Vec<PublishCellOutcome>,
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
        self.publish_inner(app, request, cancel, None).await
    }

    async fn publish_inner(
        &self,
        app: &Application,
        request: PublishRequest,
        cancel: Arc<AtomicBool>,
        expected_cells: Option<&[PublishCell]>,
    ) -> Result<PublishReport> {
        let execution = self
            .publish_inner_with_diagnostics(app, request, cancel, expected_cells)
            .await;
        if let (Err(_), Some(detail)) = (&execution.result, &execution.diagnostics) {
            eprintln!("publisher worker stderr (original error preserved): {detail}");
        }
        execution.result
    }

    async fn publish_inner_with_diagnostics(
        &self,
        app: &Application,
        request: PublishRequest,
        cancel: Arc<AtomicBool>,
        expected_cells: Option<&[PublishCell]>,
    ) -> PublishExecution {
        let mut worker_diagnostics = None;
        let result = async {
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
            if let Some(expected) = expected_cells {
                let actual = targets
                    .iter()
                    .filter_map(|row| row.get("uuid").and_then(Value::as_str))
                    .map(|uuid| PublishCell {
                        uuid: uuid.to_owned(),
                        platform: "deviantart".into(),
                    })
                    .collect::<std::collections::HashSet<_>>();
                let expected_set = expected
                    .iter()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                if actual != expected_set || expected_set.len() != expected.len() {
                    return Err(Error::InvalidState(
                        "selected publishing cells changed before publisher admission".into(),
                    ));
                }
            }
            if targets.is_empty() {
                return Ok(PublishReport::default());
            }
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
            let (result, diagnostics) = with_worker_diagnostics(result, &detail);
            worker_diagnostics = diagnostics;
            result
        }
        .await;
        PublishExecution {
            result,
            diagnostics: worker_diagnostics,
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
        for (index, (approved, job)) in batch.iter().enumerate() {
            let cell = PublishCell {
                uuid: job.uuid.clone(),
                platform: "deviantart".into(),
            };
            if cancel.load(Ordering::SeqCst) {
                record_unattempted(&mut report, batch, index);
                break;
            }
            let fresh = publish_journal::snapshot(store, &job.uuid)?;
            if fresh != *approved {
                record_unattempted(&mut report, batch, index);
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
                let detail = error.to_string();
                report.error = Some(detail.clone());
                record_outcome(&mut report, cell, PublishCellState::Unknown, Some(detail));
                record_unattempted(&mut report, batch, index + 1);
                break;
            }
            let event = match tokio::time::timeout_at(
                deadline,
                read_matching_terminal(&mut output, job, deadline),
            )
            .await
            {
                Ok(Ok(event)) => event,
                Ok(Err(error)) => {
                    let detail = error.to_string();
                    report.error = Some(detail.clone());
                    record_outcome(&mut report, cell, PublishCellState::Unknown, Some(detail));
                    record_unattempted(&mut report, batch, index + 1);
                    break;
                }
                Err(_) => {
                    let detail = "worker artwork timeout; result is unknown".to_owned();
                    report.error = Some(detail.clone());
                    record_outcome(&mut report, cell, PublishCellState::Unknown, Some(detail));
                    record_unattempted(&mut report, batch, index + 1);
                    break;
                }
            };
            journal.terminal(&event)?;
            match event {
                WorkerEvent::Scheduled { attempt_id, .. } => {
                    journal.apply_scheduled(store, &attempt_id)?;
                    record_outcome(&mut report, cell, PublishCellState::Success, None);
                }
                WorkerEvent::Failed { error, .. } => {
                    report.error = Some(error.clone());
                    record_outcome(&mut report, cell, PublishCellState::SafeFailed, Some(error));
                    record_unattempted(&mut report, batch, index + 1);
                    break;
                }
                WorkerEvent::Unknown { error, .. } => {
                    report.error = Some(error.clone());
                    record_outcome(&mut report, cell, PublishCellState::Unknown, Some(error));
                    record_unattempted(&mut report, batch, index + 1);
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
struct PublishExecution {
    result: Result<PublishReport>,
    diagnostics: Option<String>,
}

fn record_outcome(
    report: &mut PublishReport,
    cell: PublishCell,
    state: PublishCellState,
    detail: Option<String>,
) {
    match state {
        PublishCellState::Success => report.published += 1,
        PublishCellState::SafeFailed => report.failed += 1,
        PublishCellState::Unknown => report.unknown += 1,
        PublishCellState::Unattempted | PublishCellState::ConfirmedNotSubmitted => {
            report.unattempted += 1
        }
    }
    report.outcomes.push(PublishCellOutcome {
        cell,
        state,
        detail,
    });
}

fn record_unattempted(
    report: &mut PublishReport,
    batch: &[(publish_journal::ApprovedSnapshot, PublishJob)],
    start: usize,
) {
    for (_, job) in batch.iter().skip(start) {
        record_outcome(
            report,
            PublishCell {
                uuid: job.uuid.clone(),
                platform: "deviantart".into(),
            },
            PublishCellState::Unattempted,
            None,
        );
    }
}

fn with_worker_diagnostics(
    result: Result<PublishReport>,
    detail: &str,
) -> (Result<PublishReport>, Option<String>) {
    let detail = detail.trim();
    if detail.is_empty() {
        return (result, None);
    }
    let result = match result {
        Ok(mut report) => {
            if let Some(error) = &mut report.error {
                error.push_str(&format!("; worker stderr: {detail}"));
            }
            Ok(report)
        }
        Err(error) => Err(error),
    };
    (result, Some(detail.to_owned()))
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

    pub async fn publish_selected(
        &self,
        cells: Vec<PublishCell>,
        cancel: Arc<AtomicBool>,
    ) -> Result<PublishReport> {
        let publisher = self
            .publisher
            .as_ref()
            .ok_or_else(|| Error::InvalidState("publisher is not configured".into()))?;
        let uuids = cells
            .iter()
            .map(|cell| cell.uuid.clone())
            .collect::<Vec<_>>();
        publisher
            .publish_inner(
                self,
                PublishRequest {
                    uuids: Some(uuids),
                    headless: false,
                },
                cancel,
                Some(&cells),
            )
            .await
    }

    pub async fn publish_selected_with_diagnostics(
        &self,
        cells: Vec<PublishCell>,
        cancel: Arc<AtomicBool>,
    ) -> PublishCompletion {
        let Some(publisher) = self.publisher.as_ref() else {
            return PublishCompletion {
                result: Err("invalid state: publisher is not configured".into()),
                diagnostics: None,
            };
        };
        let uuids = cells.iter().map(|cell| cell.uuid.clone()).collect();
        let execution = publisher
            .publish_inner_with_diagnostics(
                self,
                PublishRequest {
                    uuids: Some(uuids),
                    headless: false,
                },
                cancel,
                Some(&cells),
            )
            .await;
        PublishCompletion {
            result: execution.result.map_err(|error| error.to_string()),
            diagnostics: execution.diagnostics,
        }
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

    fn publishing_boundary_fixture() -> (
        TempDir,
        Application,
        crate::store::Store,
        Publisher,
        PathBuf,
    ) {
        let root = tempfile::tempdir().unwrap();
        let mut rows = Vec::new();
        for n in 1..=2 {
            let basename = format!("{n}.png");
            let image = root.path().join(&basename);
            assert!(
                std::process::Command::new("magick")
                    .args(["-size", "4x3", "xc:white"])
                    .arg(&image)
                    .status()
                    .unwrap()
                    .success()
            );
            let hash = crate::images::file_sha512(&image).unwrap();
            rows.push(json!({
                "uuid": format!("00000000-0000-4000-8000-{n:012}"),
                "state": "DESCRIBED",
                "title": "Title",
                "description": "Description",
                "submissionTimestamp": 1_700_002_800,
                "apparitionTimestampIfDifferentThanSubmission": 1_700_002_800,
                "files": [{"basename": basename, "sha512sum": hash}],
                "apparitions": [{"platformName": "deviantart", "state": "unpublished", "tiers": [], "galleries": []}]
            }));
        }
        std::fs::write(
            root.path().join("publications.json"),
            serde_json::to_vec(&rows).unwrap(),
        )
        .unwrap();
        std::fs::write(root.path().join("tags.txt"), "tag\n").unwrap();
        let store = crate::store::Store::new(
            Paths::resolve(Some(root.path()), None).unwrap(),
            Config {
                tags: Some("tags.txt".into()),
                ..Config::default()
            },
        )
        .unwrap();
        let app_store = Arc::new(store.clone());
        let app = Application::new(
            app_store,
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
        let calls = root.path().join("calls.log");
        let worker = root.path().join("worker.sh");
        std::fs::write(
            &worker,
            format!(
                r#"#!/bin/sh
[ "$1" = "--check-steps" ] && exit 0
while IFS= read -r line; do
  kind=$(printf '%s' "$line" | jq -r .type)
  case "$kind" in
    open) echo '{{"protocol":1,"type":"ready"}}' ;;
    submit)
      uuid=$(printf '%s' "$line" | jq -r .uuid)
      attempt=$(printf '%s' "$line" | jq -r .attempt_id)
      echo "$uuid" >> '{}'
      printf 'simulated worker stderr\n' >&2
      printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.invalid/work"}}\n' "$attempt" "$uuid"
      ;;
    close) exit 0 ;;
  esac
done"#,
                calls.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&worker, permissions).unwrap();
        let publisher = Publisher::new(worker, root.path().join("skill.md"));
        (root, app, store, publisher, calls)
    }

    #[tokio::test]
    async fn pre_rename_failure_with_worker_stderr_remains_a_safe_io_error() {
        let (root, app, store, publisher, calls) = publishing_boundary_fixture();
        crate::store::fail_next_commit_for_test(crate::store::CommitFailureForTest::BeforeRename);
        let error = publisher
            .publish(
                &app,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap_err();
        let displayed = error.to_string();
        match error {
            Error::Io(source) => assert_eq!(source.to_string(), "injected pre-rename failure"),
            other => panic!("expected safe pre-rename I/O failure, got {other:?}"),
        }
        assert!(!displayed.contains("may be visible but is not confirmed durable"));
        assert_eq!(
            std::fs::read_to_string(calls)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["00000000-0000-4000-8000-000000000001"]
        );
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "unpublished"
        );
        let journal = PublishJournal::new(root.path()).unwrap();
        let attempt = std::fs::read_dir(root.path().join(".publicator/publish"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .into_string()
            .unwrap();
        let receipt_path = root
            .path()
            .join(".publicator/publish")
            .join(&attempt)
            .join("receipt.json");
        let receipt: Value =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(receipt["terminal"]["type"], "scheduled");
        assert_eq!(receipt["applied"], false);
        assert_eq!(journal.recover(&store).unwrap().replayed, vec![attempt]);
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
    }

    #[tokio::test]
    async fn post_rename_failure_with_worker_stderr_keeps_typed_recoverable_receipt() {
        let (root, app, store, publisher, calls) = publishing_boundary_fixture();
        crate::store::fail_next_commit_for_test(crate::store::CommitFailureForTest::AfterRename);
        let error = publisher
            .publish(
                &app,
                PublishRequest {
                    uuids: None,
                    headless: true,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap_err();
        let displayed = error.to_string();
        match error {
            Error::DurabilityUncertain { path, source } => {
                assert_eq!(path, store.paths().json);
                assert_eq!(
                    source.to_string(),
                    "injected post-rename directory sync failure"
                );
            }
            other => panic!("expected typed uncertain commit, got {other:?}"),
        }
        assert!(displayed.contains("may be visible but is not confirmed durable"));
        assert_eq!(
            std::fs::read_to_string(calls)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["00000000-0000-4000-8000-000000000001"]
        );

        let fresh_rows = store.read().unwrap();
        assert_eq!(
            fresh_rows[0]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
        assert_eq!(
            fresh_rows[0]["apparitions"][0]["url"],
            "https://example.invalid/work"
        );
        assert_eq!(fresh_rows[1]["apparitions"][0]["state"], "unpublished");

        let journal = PublishJournal::new(root.path()).unwrap();
        let attempt = std::fs::read_dir(root.path().join(".publicator/publish"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .into_string()
            .unwrap();
        let receipt_path = root
            .path()
            .join(".publicator/publish")
            .join(&attempt)
            .join("receipt.json");
        let receipt: Value =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(receipt["terminal"]["type"], "scheduled");
        assert_eq!(receipt["applied"], false);
        assert_eq!(journal.recover(&store).unwrap().replayed, vec![attempt]);
        let receipt: Value =
            serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
        assert_eq!(receipt["applied"], true);
    }
}
