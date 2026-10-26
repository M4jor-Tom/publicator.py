use crate::{
    error::{Error, Result},
    images::{file_sha512, index_by_basename},
    model::{Phase, deliveries_complete, phase, validate_metadata},
    publish_protocol::{PublishJob, WorkerEvent},
    store::Store,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApprovedSnapshot {
    pub database_path: PathBuf,
    pub uuid: String,
    pub platform: String,
    pub title: String,
    pub description: String,
    pub source_hash: String,
    pub schedule_epoch: i64,
    pub tags: Vec<String>,
    pub tiers: Vec<String>,
    pub galleries: Vec<String>,
    pub price: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receipt {
    job: PublishJob,
    approved: ApprovedSnapshot,
    terminal: Option<WorkerEvent>,
    not_submitted: bool,
    applied: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReconcileOutcome {
    Scheduled { actual_url: Option<String> },
    NotSubmitted,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecoveryReport {
    pub replayed: Vec<String>,
    pub blocked: Vec<UnresolvedAttempt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnresolvedAttempt {
    pub attempt_id: String,
    pub uuid: String,
    pub reason: String,
    pub approved: ApprovedSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttemptState {
    Pending,
    ScheduledUnapplied { actual_url: Option<String> },
    SafeFailed { error: String },
    Unknown { error: String },
    Applied { actual_url: Option<String> },
    NotSubmitted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttemptStatus {
    pub attempt_id: String,
    pub uuid: String,
    pub approved: ApprovedSnapshot,
    pub state: AttemptState,
    pub database_matches: bool,
    pub current_matches: Option<bool>,
    pub current_error: Option<String>,
}

impl AttemptStatus {
    pub fn blocks_retry(&self) -> bool {
        !self.database_matches
            || matches!(
                &self.state,
                AttemptState::Pending
                    | AttemptState::ScheduledUnapplied { .. }
                    | AttemptState::Unknown { .. }
            )
    }

    pub fn reason(&self) -> &'static str {
        match &self.state {
            AttemptState::Pending => "publish intent has no terminal receipt",
            AttemptState::ScheduledUnapplied { .. } => "scheduled receipt awaits application",
            AttemptState::SafeFailed { .. } => "worker confirmed a safe failure",
            AttemptState::Unknown { .. } => "submission outcome is unknown",
            AttemptState::Applied { .. } => "scheduled delivery is applied",
            AttemptState::NotSubmitted => "human confirmed not submitted",
        }
    }
}

pub struct PublishJournal {
    data_dir: PathBuf,
    root: PathBuf,
}

/// Held across the browser batch or explicit human reconciliation.
pub struct PublisherLock {
    _file: File,
}

impl Drop for PublisherLock {
    fn drop(&mut self) {
        // Close alone leaves flock held by concurrent fork children until exec/exit.
        let _ = FileExt::unlock(&self._file);
    }
}

pub fn publisher_lock(data_dir: &Path) -> Result<PublisherLock> {
    let dir = data_dir.join(".publicator");
    if create_real_directory(&dir, "publisher lock directory must be a real directory")? {
        File::open(data_dir)?.sync_all()?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("publish.lock"))?;
    file.try_lock_exclusive().map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Error::Busy("a publish batch or reconciliation is active".into())
        } else {
            Error::Io(error)
        }
    })?;
    Ok(PublisherLock { _file: file })
}

pub fn snapshot(store: &Store, uuid: &str) -> Result<ApprovedSnapshot> {
    let rows = store.read()?;
    let row = find(&rows, uuid)?;
    snapshot_row(store, row)
}

fn snapshot_row(store: &Store, row: &Value) -> Result<ApprovedSnapshot> {
    let uuid = string(row, "uuid")?;
    if !matches!(phase(row)?, Phase::Described | Phase::Appeared) {
        return Err(Error::InvalidState(
            "publishing requires DESCRIBED publication".into(),
        ));
    }
    let (title, description) = approved_metadata(row)?;
    let app = approved_delivery(row, uuid)?;
    let schedule_epoch = row
        .get("apparitionTimestampIfDifferentThanSubmission")
        .and_then(Value::as_i64)
        .or_else(|| row.get("submissionTimestamp").and_then(Value::as_i64))
        .ok_or_else(|| Error::Validation("approved schedule is missing".into()))?;
    let source_hash = approved_source_hash(store, row)?;
    let tags = approved_tags(store)?;
    Ok(ApprovedSnapshot {
        database_path: store.paths().json.clone(),
        uuid: uuid.to_owned(),
        platform: "deviantart".into(),
        title: title.to_owned(),
        description: description.to_owned(),
        source_hash,
        schedule_epoch,
        tags,
        tiers: strings(app, "tiers")?,
        galleries: strings(app, "galleries")?,
        price: app.get("priceIfNotFree").and_then(Value::as_f64),
    })
}

fn approved_metadata(row: &Value) -> Result<(&str, &str)> {
    let title = string(row, "title")?;
    let description = string(row, "description")?;
    validate_metadata(title, description)?;
    Ok((title, description))
}

fn approved_delivery<'a>(row: &'a Value, uuid: &str) -> Result<&'a Value> {
    let apps = row
        .get("apparitions")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Validation("apparitions must be an array".into()))?;
    let app = apps
        .iter()
        .find(|a| a.get("platformName").and_then(Value::as_str) == Some("deviantart"))
        .ok_or_else(|| Error::NotFound(format!("{uuid}:deviantart")))?;
    if app.get("state").and_then(Value::as_str) != Some("unpublished")
        && app.get("state").and_then(Value::as_str) != Some("published_or_scheduled")
    {
        return Err(Error::InvalidState(
            "invalid DeviantArt delivery state".into(),
        ));
    }
    Ok(app)
}

fn approved_source_hash(store: &Store, row: &Value) -> Result<String> {
    let basename = row
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.get("basename"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation("artwork basename is missing".into()))?;
    let source = index_by_basename(&store.paths().data_dir)?
        .remove(basename)
        .ok_or_else(|| Error::NotFound(format!("artwork {basename}")))?;
    let source_hash = file_sha512(&source)?;
    let stored_hash = row
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.get("sha512sum").or_else(|| file.get("fileSha512sum")))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation("artwork hash is missing".into()))?;
    if !stored_hash.eq_ignore_ascii_case(&source_hash) {
        return Err(Error::ReconciliationRequired(
            "artwork content no longer matches the approved record".into(),
        ));
    }
    Ok(source_hash)
}

fn approved_tags(store: &Store) -> Result<Vec<String>> {
    let tags_path = store
        .config()
        .tags
        .as_ref()
        .ok_or_else(|| Error::Config("no 'tags' path configured".into()))?;
    let tags_path = store.paths().data_dir.join(tags_path);
    let tags = fs::read_to_string(tags_path)?
        .lines()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(tags)
}

impl PublishJournal {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let data_dir = fs::canonicalize(data_dir)?;
        Ok(Self {
            root: data_dir.join(".publicator/publish"),
            data_dir,
        })
    }

    pub fn begin(&self, job: &PublishJob, approved: &ApprovedSnapshot) -> Result<()> {
        validate_attempt(&job.attempt_id)?;
        validate_attempt(&job.uuid)?;
        if approved.database_path != normalize_database_path(&approved.database_path)?
            || crate::publish_protocol::WorkerCommand::Submit(job.clone())
                .validate()
                .is_err()
            || !job_matches_approval(job, approved)
        {
            return Err(Error::Validation(
                "publish job differs from approved snapshot".into(),
            ));
        }
        let path = self.attempt_path(&job.attempt_id)?;
        create_durable_dirs(&self.data_dir, &path)?;
        let receipt = Receipt {
            job: job.clone(),
            approved: approved.clone(),
            terminal: None,
            not_submitted: false,
            applied: false,
        };
        write_atomic(
            &path.join("receipt.json"),
            &serde_json::to_vec_pretty(&receipt)?,
        )
    }

    pub fn terminal(&self, event: &WorkerEvent) -> Result<()> {
        let (attempt_id, uuid, protocol) = terminal_identity(event)?;
        validate_attempt(attempt_id)?;
        if protocol != 1 {
            return Err(Error::Protocol("unsupported worker protocol".into()));
        }
        let mut receipt = self.read_receipt(attempt_id)?;
        if receipt.job.attempt_id != attempt_id || receipt.job.uuid != uuid {
            return Err(Error::Protocol(
                "terminal event identity does not match intent".into(),
            ));
        }
        if !receipt.record_terminal(event)? {
            return Ok(());
        }
        self.save_receipt(attempt_id, &receipt)
    }

    pub fn applied(&self, attempt_id: &str) -> Result<()> {
        validate_attempt(attempt_id)?;
        let mut receipt = self.read_receipt(attempt_id)?;
        if !matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. })) {
            return Err(Error::InvalidState(
                "only scheduled receipts can be applied".into(),
            ));
        }
        if !receipt.applied {
            receipt.applied = true;
            self.save_receipt(attempt_id, &receipt)?;
        }
        Ok(())
    }

    pub fn apply_scheduled(&self, store: &Store, attempt_id: &str) -> Result<()> {
        self.apply_scheduled_with(store, attempt_id, || Ok(()))
    }

    fn apply_scheduled_with(
        &self,
        store: &Store,
        attempt_id: &str,
        after_snapshot_check: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        validate_attempt(attempt_id)?;
        let receipt = self.read_receipt(attempt_id)?;
        receipt.require_database(store)?;
        let Some(WorkerEvent::Scheduled { uuid, url, .. }) = &receipt.terminal else {
            return Err(Error::InvalidState("scheduled receipt is required".into()));
        };
        if *uuid != receipt.approved.uuid {
            return Err(Error::Protocol(
                "receipt UUID differs from approved snapshot".into(),
            ));
        }
        // Hashing/art lookup is deliberately outside Store::mutate; the row fields are checked again atomically below.
        ensure_current_approval(
            store,
            uuid,
            &receipt.approved,
            "approved content/options changed since submission",
        )?;
        after_snapshot_check()?;
        apply_delivery(store, uuid, url.as_deref(), &receipt.approved)?;
        self.applied(attempt_id)
    }

    /// Read attempt outcomes without replaying or mutating receipts. Caller holds `publisher_lock`.
    pub fn attempt_statuses_locked(&self, store: &Store) -> Result<Vec<AttemptStatus>> {
        let rows = store.read()?;
        let mut attempts = Vec::new();
        for receipt in self.receipts()? {
            let receipt = receipt?;
            let attempt_id = receipt.job.attempt_id.clone();
            let state = receipt.state()?;
            let database_matches = receipt.approved.database_path == store.paths().json;
            let needs_comparison = database_matches
                && matches!(
                    &state,
                    AttemptState::Pending
                        | AttemptState::ScheduledUnapplied { .. }
                        | AttemptState::Unknown { .. }
                );
            let (current_matches, current_error) =
                compare_current(store, &rows, &receipt, needs_comparison);
            attempts.push(AttemptStatus {
                attempt_id,
                uuid: receipt.job.uuid,
                approved: receipt.approved,
                state,
                database_matches,
                current_matches,
                current_error,
            });
        }
        Ok(attempts)
    }

    /// Read unresolved option locks without replaying or mutating any receipt.
    /// Caller holds `publisher_lock` so the result stays stable through the guarded edit.
    pub fn blocked_attempts_locked(&self, store: &Store) -> Result<Vec<UnresolvedAttempt>> {
        let mut blocked = Vec::new();
        for receipt in self.receipts()? {
            let receipt = receipt?;
            if receipt.approved.database_path != store.paths().json {
                blocked.push(unresolved(&receipt, "different publication database"));
            } else if receipt.retry_resolved() {
                continue;
            } else {
                let reason = match &receipt.terminal {
                    Some(WorkerEvent::Scheduled { .. }) => "scheduled receipt awaits application",
                    Some(WorkerEvent::Unknown { .. }) => "submission outcome is unknown",
                    _ => "publish intent has no terminal receipt",
                };
                blocked.push(unresolved(&receipt, reason));
            }
        }
        Ok(blocked)
    }

    pub fn recover(&self, store: &Store) -> Result<RecoveryReport> {
        let mut report = RecoveryReport::default();
        for receipt in self.receipts()? {
            let receipt = receipt?;
            self.recover_receipt(store, &receipt, &mut report)?;
        }
        Ok(report)
    }

    fn recover_receipt(
        &self,
        store: &Store,
        receipt: &Receipt,
        report: &mut RecoveryReport,
    ) -> Result<()> {
        let attempt_id = &receipt.job.attempt_id;
        if receipt.approved.database_path != store.paths().json {
            report
                .blocked
                .push(unresolved(receipt, "different publication database"));
        } else if receipt.retry_resolved() {
            return Ok(());
        } else if matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. })) {
            match self.apply_scheduled(store, attempt_id) {
                Ok(()) => report.replayed.push(attempt_id.clone()),
                Err(Error::ReconciliationRequired(reason)) => {
                    report.blocked.push(unresolved(receipt, &reason))
                }
                Err(error) => return Err(error),
            }
        } else {
            let reason = if matches!(receipt.terminal, Some(WorkerEvent::Unknown { .. })) {
                "submission outcome is unknown"
            } else {
                "submission intent has no terminal receipt"
            };
            report.blocked.push(unresolved(receipt, reason));
        }
        Ok(())
    }

    pub fn reconcile(
        &self,
        store: &Store,
        attempt_id: &str,
        outcome: ReconcileOutcome,
    ) -> Result<()> {
        validate_attempt(attempt_id)?;
        outcome.validate()?;
        let _publisher_lock = publisher_lock(&self.data_dir)?;
        let receipt = self.read_receipt(attempt_id)?;
        receipt.require_database(store)?;
        if receipt.applied || receipt.not_submitted {
            return Ok(());
        }
        match outcome {
            ReconcileOutcome::Scheduled { actual_url } => {
                self.reconcile_scheduled(store, attempt_id, receipt, actual_url)
            }
            ReconcileOutcome::NotSubmitted => self.reconcile_not_submitted(attempt_id, receipt),
        }
    }

    fn reconcile_not_submitted(&self, attempt_id: &str, mut receipt: Receipt) -> Result<()> {
        if matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. })) {
            return Err(Error::ReconciliationRequired(
                "a scheduled receipt cannot be released as not submitted".into(),
            ));
        }
        receipt.not_submitted = true;
        self.save_receipt(attempt_id, &receipt)
    }

    fn reconcile_scheduled(
        &self,
        store: &Store,
        attempt_id: &str,
        mut receipt: Receipt,
        actual_url: Option<String>,
    ) -> Result<()> {
        ensure_current_approval(
            store,
            &receipt.job.uuid,
            &receipt.approved,
            "approved content/options changed; human correction is required",
        )?;
        let event = WorkerEvent::Scheduled {
            protocol: 1,
            attempt_id: attempt_id.to_owned(),
            uuid: receipt.job.uuid.clone(),
            url: actual_url,
        };
        match &receipt.terminal {
            Some(WorkerEvent::Scheduled { .. }) if receipt.terminal.as_ref() != Some(&event) => {
                return Err(Error::Protocol("conflicting scheduled receipt".into()));
            }
            Some(WorkerEvent::Scheduled { .. }) => {}
            _ => {
                receipt.terminal = Some(event);
                self.save_receipt(attempt_id, &receipt)?;
            }
        }
        self.apply_scheduled(store, attempt_id)
    }

    fn receipts(&self) -> Result<impl Iterator<Item = Result<Receipt>> + '_> {
        let mut paths = match fs::read_dir(&self.root) {
            Ok(entries) => entries.collect::<std::io::Result<Vec<_>>>()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        paths.sort_by_key(|entry| entry.file_name());
        Ok(paths.into_iter().map(|entry| self.receipt_entry(entry)))
    }

    fn receipt_entry(&self, entry: fs::DirEntry) -> Result<Receipt> {
        let attempt_id = entry.file_name().to_string_lossy().into_owned();
        validate_attempt(&attempt_id)?;
        if !entry.file_type()?.is_dir() {
            return Err(Error::Protocol(
                "publish journal entry is not a directory".into(),
            ));
        }
        self.read_receipt(&attempt_id)
    }

    fn attempt_path(&self, attempt_id: &str) -> Result<PathBuf> {
        validate_attempt(attempt_id)?;
        Ok(self.root.join(attempt_id))
    }
    fn read_receipt(&self, attempt_id: &str) -> Result<Receipt> {
        let attempt = self.attempt_path(attempt_id)?;
        let path = self.receipt_file(&attempt)?;
        let bytes = fs::read(path)?;
        if bytes.is_empty() {
            return Err(Error::Protocol("empty publish receipt".into()));
        }
        let receipt: Receipt = serde_json::from_slice(&bytes)?;
        receipt.validate(attempt_id)?;
        Ok(receipt)
    }
    fn receipt_file(&self, attempt: &Path) -> Result<PathBuf> {
        for directory in [
            self.data_dir.join(".publicator"),
            self.root.clone(),
            attempt.to_owned(),
        ] {
            let metadata = fs::symlink_metadata(&directory)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::Validation(
                    "publish receipt path must contain real directories".into(),
                ));
            }
        }
        let path = attempt.join("receipt.json");
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::Validation(
                "publish receipt must be a regular file".into(),
            ));
        }
        Ok(path)
    }

    fn save_receipt(&self, attempt_id: &str, receipt: &Receipt) -> Result<()> {
        write_atomic(
            &self.attempt_path(attempt_id)?.join("receipt.json"),
            &serde_json::to_vec_pretty(receipt)?,
        )
    }
}

impl ReconcileOutcome {
    fn validate(&self) -> Result<()> {
        if let Self::Scheduled {
            actual_url: Some(url),
        } = self
        {
            validate_actual_url(url)?;
        }
        Ok(())
    }
}

impl Receipt {
    fn require_database(&self, store: &Store) -> Result<()> {
        if self.approved.database_path != store.paths().json {
            return Err(Error::ReconciliationRequired(
                "receipt belongs to a different publication database".into(),
            ));
        }
        Ok(())
    }

    fn retry_resolved(&self) -> bool {
        self.applied
            || self.not_submitted
            || matches!(self.terminal, Some(WorkerEvent::Failed { .. }))
    }

    fn record_terminal(&mut self, event: &WorkerEvent) -> Result<bool> {
        match &self.terminal {
            Some(previous) if previous == event => return Ok(false),
            Some(_) => return Err(Error::Protocol("conflicting terminal receipt".into())),
            None if self.not_submitted => {
                return Err(Error::Protocol(
                    "attempt was already reconciled as not submitted".into(),
                ));
            }
            None => {}
        }
        self.terminal = Some(event.clone());
        Ok(true)
    }

    fn validate(&self, attempt_id: &str) -> Result<()> {
        validate_attempt(&self.job.attempt_id)?;
        validate_attempt(&self.job.uuid)?;
        if self.job.attempt_id != attempt_id
            || self.job.uuid != self.approved.uuid
            || self.approved.platform != "deviantart"
            || self.approved.database_path != normalize_database_path(&self.approved.database_path)?
            || !self.flags_valid()
        {
            return Err(Error::Protocol(
                "publish receipt identity or state is invalid".into(),
            ));
        }
        self.validate_terminal(attempt_id)
    }

    fn flags_valid(&self) -> bool {
        let scheduled = matches!(self.terminal, Some(WorkerEvent::Scheduled { .. }));
        (!self.applied || scheduled) && (!self.not_submitted || !scheduled)
    }

    fn validate_terminal(&self, attempt_id: &str) -> Result<()> {
        if let Some(event) = &self.terminal {
            let (event_id, uuid, protocol) = terminal_identity(event).map_err(|_| {
                Error::Protocol("publish receipt terminal identity is invalid".into())
            })?;
            if protocol != 1 || event_id != attempt_id || uuid != self.job.uuid {
                return Err(Error::Protocol(
                    "publish receipt terminal identity is invalid".into(),
                ));
            }
        }
        Ok(())
    }

    fn state(&self) -> Result<AttemptState> {
        Ok(if self.not_submitted {
            AttemptState::NotSubmitted
        } else if self.applied {
            match &self.terminal {
                Some(WorkerEvent::Scheduled { url, .. }) => AttemptState::Applied {
                    actual_url: url.clone(),
                },
                _ => return Err(Error::Protocol("applied receipt is not scheduled".into())),
            }
        } else {
            match &self.terminal {
                Some(WorkerEvent::Scheduled { url, .. }) => AttemptState::ScheduledUnapplied {
                    actual_url: url.clone(),
                },
                Some(WorkerEvent::Failed { error, .. }) => AttemptState::SafeFailed {
                    error: error.clone(),
                },
                Some(WorkerEvent::Unknown { error, .. }) => AttemptState::Unknown {
                    error: error.clone(),
                },
                None => AttemptState::Pending,
                _ => return Err(Error::Protocol("invalid publish terminal receipt".into())),
            }
        })
    }
}

fn ensure_current_approval(
    store: &Store,
    uuid: &str,
    approved: &ApprovedSnapshot,
    reason: &str,
) -> Result<()> {
    let rows = store.read()?;
    let current = snapshot_row(store, find(&rows, uuid)?)?;
    if current != *approved {
        return Err(Error::ReconciliationRequired(reason.into()));
    }
    Ok(())
}

fn compare_current(
    store: &Store,
    rows: &[Value],
    receipt: &Receipt,
    needs_comparison: bool,
) -> (Option<bool>, Option<String>) {
    if needs_comparison {
        match find(rows, &receipt.job.uuid).and_then(|row| snapshot_row(store, row)) {
            Ok(current) => (Some(current == receipt.approved), None),
            Err(error) => (Some(false), Some(error.to_string())),
        }
    } else {
        (None, None)
    }
}

fn apply_delivery(
    store: &Store,
    uuid: &str,
    url: Option<&str>,
    approved: &ApprovedSnapshot,
) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        if !row_matches_snapshot(row, approved)? {
            return Err(Error::ReconciliationRequired(
                "approved content/options changed since submission".into(),
            ));
        }
        {
            let Some(apps) = row.get_mut("apparitions").and_then(Value::as_array_mut) else {
                return Err(Error::Validation("apparitions must be an array".into()));
            };
            let app = apps
                .iter_mut()
                .find(|app| app.get("platformName").and_then(Value::as_str) == Some("deviantart"))
                .ok_or_else(|| Error::NotFound(format!("{uuid}:deviantart")))?;
            match app.get("state").and_then(Value::as_str) {
                Some("unpublished") => {
                    app["state"] = Value::String("published_or_scheduled".into());
                    if let Some(url) = url {
                        app["url"] = Value::String(url.to_owned());
                    }
                }
                Some("published_or_scheduled") => {}
                _ => {
                    return Err(Error::InvalidState("delivery target is not pending".into()));
                }
            }
        }
        row["state"] = Value::String(
            if deliveries_complete(row) {
                "APPEARED"
            } else {
                "DESCRIBED"
            }
            .into(),
        );
        Ok(())
    })
}

fn job_matches_approval(job: &PublishJob, approved: &ApprovedSnapshot) -> bool {
    job.uuid == approved.uuid
        && approved.platform == "deviantart"
        && job.title == approved.title
        && job.description == approved.description
        && job.tags == approved.tags
        && job.schedule.epoch == approved.schedule_epoch
        && job_options_match(job, approved)
}

fn job_options_match(job: &PublishJob, approved: &ApprovedSnapshot) -> bool {
    job.price == approved.price
        && job.tiers == approved.tiers
        && job.galleries == approved.galleries
}

fn terminal_identity(event: &WorkerEvent) -> Result<(&str, &str, u8)> {
    Ok(match event {
        WorkerEvent::Scheduled {
            attempt_id,
            uuid,
            protocol,
            ..
        }
        | WorkerEvent::Failed {
            attempt_id,
            uuid,
            protocol,
            ..
        }
        | WorkerEvent::Unknown {
            attempt_id,
            uuid,
            protocol,
            ..
        } => (attempt_id.as_str(), uuid.as_str(), *protocol),
        _ => return Err(Error::Protocol("event is not a publish terminal".into())),
    })
}

fn unresolved(receipt: &Receipt, reason: &str) -> UnresolvedAttempt {
    UnresolvedAttempt {
        attempt_id: receipt.job.attempt_id.clone(),
        uuid: receipt.job.uuid.clone(),
        reason: reason.into(),
        approved: receipt.approved.clone(),
    }
}
fn validate_attempt(attempt_id: &str) -> Result<()> {
    if uuid::Uuid::parse_str(attempt_id).is_err() || attempt_id.contains(['/', '\\']) {
        return Err(Error::Validation("unsafe publish attempt ID".into()));
    }
    Ok(())
}
fn validate_actual_url(value: &str) -> Result<()> {
    let url = url::Url::parse(value)
        .map_err(|_| Error::Validation("actual URL must be a valid HTTP(S) URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Validation(
            "actual URL must be a valid HTTP(S) URL".into(),
        ));
    }
    Ok(())
}
fn find<'a>(rows: &'a [Value], uuid: &str) -> Result<&'a Value> {
    rows.iter()
        .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid))
        .ok_or_else(|| Error::NotFound(uuid.into()))
}
fn find_mut<'a>(rows: &'a mut [Value], uuid: &str) -> Result<&'a mut Value> {
    rows.iter_mut()
        .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid))
        .ok_or_else(|| Error::NotFound(uuid.into()))
}
fn string<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation(format!("publication {key} is missing")))
}
fn strings(row: &Value, key: &str) -> Result<Vec<String>> {
    match row.get(key) {
        None => Ok(Vec::new()),
        Some(value) => value
            .as_array()
            .ok_or_else(|| Error::Validation(format!("{key} must be an array")))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| Error::Validation(format!("{key} entries must be strings")))
            })
            .collect(),
    }
}
fn row_matches_snapshot(row: &Value, approved: &ApprovedSnapshot) -> Result<bool> {
    let phase_valid = matches!(phase(row)?, Phase::Described | Phase::Appeared);
    let apps = row
        .get("apparitions")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Validation("apparitions must be an array".into()))?;
    let Some(app) = apps
        .iter()
        .find(|app| app.get("platformName").and_then(Value::as_str) == Some("deviantart"))
    else {
        return Ok(false);
    };
    Ok(phase_valid
        && row.get("uuid").and_then(Value::as_str) == Some(&approved.uuid)
        && row.get("title").and_then(Value::as_str) == Some(&approved.title)
        && row.get("description").and_then(Value::as_str) == Some(&approved.description)
        && row
            .get("apparitionTimestampIfDifferentThanSubmission")
            .and_then(Value::as_i64)
            .or_else(|| row.get("submissionTimestamp").and_then(Value::as_i64))
            == Some(approved.schedule_epoch)
        && row_options_match(row, app, approved)?)
}

fn row_options_match(row: &Value, app: &Value, approved: &ApprovedSnapshot) -> Result<bool> {
    Ok(strings(app, "tiers")? == approved.tiers
        && strings(app, "galleries")? == approved.galleries
        && app.get("priceIfNotFree").and_then(Value::as_f64) == approved.price
        && row
            .get("files")
            .and_then(Value::as_array)
            .and_then(|files| files.first())
            .and_then(|f| f.get("sha512sum").or_else(|| f.get("fileSha512sum")))
            .and_then(Value::as_str)
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&approved.source_hash)))
}

fn normalize_database_path(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(target) => Ok(target),
        Err(error) => {
            if fs::symlink_metadata(path).is_ok() {
                return Err(error.into());
            }
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            Ok(fs::canonicalize(parent)?.join(
                path.file_name()
                    .ok_or_else(|| Error::Config("database path has no filename".into()))?,
            ))
        }
    }
}
fn create_durable_dirs(root: &Path, attempt: &Path) -> Result<()> {
    create_durable_dirs_with(root, attempt, |path| File::open(path)?.sync_all())
}
fn create_durable_dirs_with(
    root: &Path,
    attempt: &Path,
    mut sync: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<()> {
    let publicator = root.join(".publicator");
    let publish = publicator.join("publish");
    let mut created = Vec::new();
    for dir in [&publicator, &publish, attempt] {
        if dir == attempt {
            create_attempt_directory(dir)?;
            created.push(dir.to_path_buf());
        } else if create_real_directory(dir, "publish journal ancestors must be real directories")?
        {
            created.push(dir.to_path_buf());
        }
    }
    for dir in created {
        sync(dir.parent().unwrap())?;
    }
    sync(attempt)?;
    Ok(())
}
fn create_attempt_directory(path: &Path) -> Result<()> {
    fs::create_dir(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::AlreadyExists => {
            Error::InvalidState("publish attempt already exists".into())
        }
        _ => error.into(),
    })
}

fn create_real_directory(path: &Path, message: &str) -> Result<bool> {
    match fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::Validation(message.into()));
            }
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Config("receipt path has no parent".into()))?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|error| Error::Io(error.error))?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|source| Error::DurabilityUncertain {
            path: path.to_owned(),
            source,
        })
}

#[cfg(test)]
mod durability_tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn dropping_publisher_lock_releases_it_while_a_forked_child_retains_the_file() {
        use std::os::fd::AsRawFd;
        let root = tempfile::tempdir().unwrap();
        let lock = publisher_lock(root.path()).unwrap();
        let (parent, child) = std::os::unix::net::UnixStream::pair().unwrap();
        // A concurrent worker/fixture fork inherits the locked open file description
        // until exec, even though Rust opens files with CLOEXEC.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            let mut byte = 0u8;
            unsafe {
                libc::close(parent.as_raw_fd());
                libc::read(child.as_raw_fd(), (&mut byte as *mut u8).cast(), 1);
                libc::_exit(0);
            }
        }
        drop(child);
        let assertions = std::panic::catch_unwind(|| {
            assert!(matches!(publisher_lock(root.path()), Err(Error::Busy(_))));
            drop(lock);
            let next = publisher_lock(root.path());
            assert!(
                next.is_ok(),
                "inherited descriptor retained publisher lock: {:?}",
                next.err()
            );
        });
        // EOF releases the child only after both assertions, including on failure.
        drop(parent);
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
        if let Err(panic) = assertions {
            std::panic::resume_unwind(panic);
        }
    }

    #[test]
    fn intent_directory_creation_syncs_each_parent_before_the_attempt_directory() {
        let root = tempfile::tempdir().unwrap();
        let attempt = root
            .path()
            .join(".publicator/publish/00000000-0000-4000-8000-000000000001");
        let mut synced = Vec::new();
        create_durable_dirs_with(root.path(), &attempt, |path| {
            synced.push(path.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            synced,
            vec![
                root.path().to_owned(),
                root.path().join(".publicator"),
                root.path().join(".publicator/publish"),
                attempt
            ]
        );
    }

    #[test]
    fn failed_parent_sync_prevents_confirming_new_intent_directories() {
        let root = tempfile::tempdir().unwrap();
        let attempt = root
            .path()
            .join(".publicator/publish/00000000-0000-4000-8000-000000000001");
        let error = create_durable_dirs_with(root.path(), &attempt, |_| {
            Err(std::io::Error::other("injected sync failure"))
        });
        assert!(
            matches!(error, Err(Error::Io(ref cause)) if cause.to_string() == "injected sync failure")
        );
        assert!(attempt.exists());
    }

    fn scheduled_fixture(
        url: Option<&str>,
    ) -> (tempfile::TempDir, Store, PublishJournal, PublishJob) {
        use crate::{
            config::{Config, Paths},
            publish_protocol::ScheduleParts,
        };
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let data = root.path();
        let image = data.join("art.png");
        assert!(
            std::process::Command::new("magick")
                .args(["-size", "3x2", "xc:white"])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(data.join("tags.txt"), "tag\\n").unwrap();
        let publication = json!({"uuid":"00000000-0000-4000-8000-000000000001", "state":"DESCRIBED", "title":"Title", "description":"Description", "submissionTimestamp":1700002800, "apparitionTimestampIfDifferentThanSubmission":1700002800, "files":[{"basename":"art.png","sha512sum":crate::images::file_sha512(&image).unwrap()}], "apparitions":[{"platformName":"deviantart","state":"unpublished"}]});
        std::fs::write(
            data.join("publications.json"),
            serde_json::to_vec(&json!([publication])).unwrap(),
        )
        .unwrap();
        let paths = Paths::resolve(Some(data), None).unwrap();
        let store = Store::new(
            paths,
            Config {
                tags: Some("tags.txt".into()),
                ..Config::default()
            },
        )
        .unwrap();
        let approved = snapshot(&store, "00000000-0000-4000-8000-000000000001").unwrap();
        let job = PublishJob {
            protocol: 1,
            attempt_id: "00000000-0000-4000-8000-000000000002".into(),
            uuid: approved.uuid.clone(),
            image_path: image,
            title: approved.title.clone(),
            description: approved.description.clone(),
            tags: approved.tags.clone(),
            schedule: ScheduleParts {
                epoch: 1700002800,
                timezone: "UTC".into(),
                year: 2023,
                month: 11,
                day: 14,
                hour: 23,
                minute: 0,
            },
            price: None,
            tiers: vec![],
            galleries: vec![],
        };
        let journal = PublishJournal::new(data).unwrap();
        journal.begin(&job, &approved).unwrap();
        journal
            .terminal(&WorkerEvent::Scheduled {
                protocol: 1,
                attempt_id: job.attempt_id.clone(),
                uuid: job.uuid.clone(),
                url: url.map(str::to_owned),
            })
            .unwrap();
        (root, store, journal, job)
    }

    #[test]
    fn store_pre_rename_failure_keeps_scheduled_receipt_for_recovery() {
        let (_root, store, journal, job) = scheduled_fixture(Some("https://example.invalid/work"));
        crate::store::fail_next_commit_for_test(crate::store::CommitFailureForTest::BeforeRename);
        assert!(matches!(
            journal.apply_scheduled(&store, &job.attempt_id),
            Err(Error::Io(_))
        ));
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "unpublished"
        );
        let receipt = journal.read_receipt(&job.attempt_id).unwrap();
        assert!(matches!(
            receipt.terminal,
            Some(WorkerEvent::Scheduled { .. })
        ));
        assert!(!receipt.applied);
        assert_eq!(
            journal.recover(&store).unwrap().replayed,
            vec![job.attempt_id]
        );
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
    }

    #[test]
    fn store_post_rename_durability_failure_keeps_receipt_and_recovers_idempotently() {
        let (_root, store, journal, job) = scheduled_fixture(Some("https://example.invalid/work"));
        crate::store::fail_next_commit_for_test(crate::store::CommitFailureForTest::AfterRename);
        assert!(matches!(
            journal.apply_scheduled(&store, &job.attempt_id),
            Err(Error::DurabilityUncertain { .. })
        ));
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
        let receipt = journal.read_receipt(&job.attempt_id).unwrap();
        assert!(matches!(
            receipt.terminal,
            Some(WorkerEvent::Scheduled { .. })
        ));
        assert!(!receipt.applied);
        assert_eq!(
            journal.recover(&store).unwrap().replayed,
            vec![job.attempt_id.clone()]
        );
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["url"],
            "https://example.invalid/work"
        );
        assert!(journal.read_receipt(&job.attempt_id).unwrap().applied);
    }

    #[test]
    fn approved_edit_after_hash_check_is_rejected_inside_the_fresh_store_mutation() {
        use serde_json::json;
        let (_root, store, journal, job) = scheduled_fixture(None);
        let result = journal.apply_scheduled_with(&store, &job.attempt_id, || {
            store.mutate(|rows| {
                rows[0]["title"] = json!("changed after verification");
                Ok(())
            })?;
            Ok(())
        });
        assert!(matches!(result, Err(Error::ReconciliationRequired(_))));
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "unpublished"
        );
        assert_eq!(
            store.read().unwrap()[0]["title"],
            "changed after verification"
        );
        assert_eq!(journal.recover(&store).unwrap().blocked.len(), 1);
    }
}
