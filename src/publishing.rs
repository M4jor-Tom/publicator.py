use crate::{
    application::Application,
    error::{Error, Result},
    images::index_by_basename,
    model::{Phase, phase},
    publish_journal::{self, PublishJournal},
    publish_protocol::{PublishJob, ScheduleParts, SessionOpen, WorkerCommand, WorkerEvent},
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
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    time::Instant,
};

const READY_TIMEOUT: Duration = Duration::from_secs(90);
const ART_TIMEOUT: Duration = Duration::from_secs(600);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE: usize = 1024 * 1024;
// ponytail: fixed per-worker timeouts and a 1 MiB frame cap; tune only when real browser behavior requires it.

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
}
impl Publisher {
    pub fn new(worker: PathBuf, skill: PathBuf) -> Self {
        Self { worker, skill }
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
        let mut batch = Vec::new();
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
            if image
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("webp"))
            {
                return Err(Error::Validation(
                    "WebP is not accepted by DeviantArt".into(),
                ));
            }
            let job = make_job(&approved, image)?;
            job_command_valid(&job)?;
            batch.push((approved, job));
        }
        let mut child = start_worker(&self.worker)?;
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
                &cancel,
            )
            .await;
        shutdown(&mut child).await;
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
                let diagnostic = format!("worker stderr: {}", detail.trim());
                report.error = Some(
                    report
                        .error
                        .map_or(diagnostic.clone(), |error| format!("{error}; {diagnostic}")),
                );
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
            timezone: "Europe/Paris".into(),
        });
        send_line(&mut input, &open).await?;
        match tokio::time::timeout(READY_TIMEOUT, read_event(&mut output))
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
            send_line(&mut input, &WorkerCommand::Submit(job.clone())).await?;
            let event = match tokio::time::timeout(
                ART_TIMEOUT,
                read_matching_terminal(&mut output, job, Instant::now() + ART_TIMEOUT),
            )
            .await
            {
                Ok(Ok(event)) => event,
                Ok(Err(e)) => {
                    report.unknown += 1;
                    report.error = Some(e.to_string());
                    break;
                }
                Err(_) => {
                    report.unknown += 1;
                    report.error = Some("worker artwork timeout; result is unknown".into());
                    break;
                }
            };
            journal.terminal(&event)?;
            match event {
                WorkerEvent::Scheduled { attempt_id, .. } => {
                    journal.apply_scheduled(store, &attempt_id)?;
                    journal.applied(&attempt_id)?;
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
        let _ = send_line(&mut input, &WorkerCommand::Close { protocol: 1 }).await;
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
fn make_job(a: &publish_journal::ApprovedSnapshot, image_path: PathBuf) -> Result<PublishJob> {
    let zone: chrono_tz::Tz = "Europe/Paris".parse().unwrap();
    let dt = chrono::DateTime::from_timestamp(a.schedule_epoch, 0)
        .ok_or_else(|| Error::Validation("invalid schedule epoch".into()))?
        .with_timezone(&zone);
    let job = PublishJob {
        protocol: 1,
        attempt_id: uuid::Uuid::new_v4().to_string(),
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
    WorkerCommand::Submit(job.clone())
        .validate()
        .map_err(Error::Validation)
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
async fn send_line(w: &mut (impl AsyncWrite + Unpin), command: &WorkerCommand) -> Result<()> {
    let mut line = serde_json::to_vec(command)?;
    if line.len() + 1 > MAX_LINE {
        return Err(Error::Protocol("worker command exceeds 1 MiB".into()));
    }
    line.push(b'\n');
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
async fn shutdown(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // The child is the process-group leader; signal the group so browser descendants
        // receive the same bounded graceful-then-forced cleanup.
        unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        let _ = child.wait().await;
        return;
    }
    let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(CLEANUP_TIMEOUT, child.wait()).await;
    }
}
use chrono::{Datelike, Timelike};
