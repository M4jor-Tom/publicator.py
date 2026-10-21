use crate::{
    error::{Error, Result},
    images::{file_sha512, index_by_basename},
    model::{Phase, phase, validate_metadata},
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

pub struct PublishJournal {
    data_dir: PathBuf,
    root: PathBuf,
}

/// Held across the browser batch or explicit human reconciliation.
pub struct PublisherLock {
    _file: File,
}

pub fn publisher_lock(data_dir: &Path) -> Result<PublisherLock> {
    let dir = data_dir.join(".publicator");
    match fs::create_dir(&dir) {
        Ok(()) => File::open(data_dir)?.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&dir)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::Validation(
                    "publisher lock directory must be a real directory".into(),
                ));
            }
        }
        Err(error) => return Err(error.into()),
    }
    let file = OpenOptions::new()
        .create(true)
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
    let title = string(row, "title")?;
    let description = string(row, "description")?;
    validate_metadata(title, description)?;
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
    let schedule_epoch = row
        .get("apparitionTimestampIfDifferentThanSubmission")
        .and_then(Value::as_i64)
        .or_else(|| row.get("submissionTimestamp").and_then(Value::as_i64))
        .ok_or_else(|| Error::Validation("approved schedule is missing".into()))?;
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
            || job.uuid != approved.uuid
            || approved.platform != "deviantart"
            || job.title != approved.title
            || job.description != approved.description
            || job.tags != approved.tags
            || job.schedule.epoch != approved.schedule_epoch
            || job.price != approved.price
            || job.tiers != approved.tiers
            || job.galleries != approved.galleries
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
        let (attempt_id, uuid, protocol) = match event {
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
            } => (attempt_id, uuid, protocol),
            _ => return Err(Error::Protocol("event is not a publish terminal".into())),
        };
        validate_attempt(attempt_id)?;
        if *protocol != 1 {
            return Err(Error::Protocol("unsupported worker protocol".into()));
        }
        let mut receipt = self.read_receipt(attempt_id)?;
        if receipt.job.attempt_id != *attempt_id || receipt.job.uuid != *uuid {
            return Err(Error::Protocol(
                "terminal event identity does not match intent".into(),
            ));
        }
        match &receipt.terminal {
            Some(previous) if previous == event => return Ok(()),
            Some(_) => return Err(Error::Protocol("conflicting terminal receipt".into())),
            None if receipt.not_submitted => {
                return Err(Error::Protocol(
                    "attempt was already reconciled as not submitted".into(),
                ));
            }
            None => {}
        }
        receipt.terminal = Some(event.clone());
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
        if receipt.approved.database_path != store.paths().json {
            return Err(Error::ReconciliationRequired(
                "receipt belongs to a different publication database".into(),
            ));
        }
        let Some(WorkerEvent::Scheduled { uuid, url, .. }) = &receipt.terminal else {
            return Err(Error::InvalidState("scheduled receipt is required".into()));
        };
        if *uuid != receipt.approved.uuid {
            return Err(Error::Protocol(
                "receipt UUID differs from approved snapshot".into(),
            ));
        }
        // Hashing/art lookup is deliberately outside Store::mutate; the row fields are checked again atomically below.
        let rows = store.read()?;
        let current = snapshot_row(store, find(&rows, uuid)?)?;
        if current != receipt.approved {
            return Err(Error::ReconciliationRequired(
                "approved content/options changed since submission".into(),
            ));
        }
        after_snapshot_check()?;
        store.mutate(|rows| {
            let row = find_mut(rows, uuid)?;
            if !row_matches_snapshot(row, &receipt.approved)? {
                return Err(Error::ReconciliationRequired(
                    "approved content/options changed since submission".into(),
                ));
            }
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
                        app["url"] = Value::String(url.clone());
                    }
                    let complete = apps.iter().all(|a| {
                        a.get("state").and_then(Value::as_str) == Some("published_or_scheduled")
                    });
                    row["state"] =
                        Value::String(if complete { "APPEARED" } else { "DESCRIBED" }.into());
                }
                Some("published_or_scheduled") => {}
                _ => return Err(Error::InvalidState("delivery target is not pending".into())),
            }
            Ok(())
        })?;
        self.applied(attempt_id)
    }

    pub fn recover(&self, store: &Store) -> Result<RecoveryReport> {
        let mut report = RecoveryReport::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(error) => return Err(error.into()),
        };
        let mut paths = entries.collect::<std::io::Result<Vec<_>>>()?;
        paths.sort_by_key(|entry| entry.file_name());
        for entry in paths {
            let attempt_id = entry.file_name().to_string_lossy().into_owned();
            validate_attempt(&attempt_id)?;
            if !entry.file_type()?.is_dir() {
                return Err(Error::Protocol(
                    "publish journal entry is not a directory".into(),
                ));
            }
            let receipt = self.read_receipt(&attempt_id)?;
            if receipt.approved.database_path != store.paths().json {
                report
                    .blocked
                    .push(unresolved(&receipt, "different publication database"));
            } else if receipt.applied
                || receipt.not_submitted
                || matches!(receipt.terminal, Some(WorkerEvent::Failed { .. }))
            {
                continue;
            } else if matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. })) {
                match self.apply_scheduled(store, &attempt_id) {
                    Ok(()) => report.replayed.push(attempt_id),
                    Err(Error::ReconciliationRequired(reason)) => {
                        report.blocked.push(unresolved(&receipt, &reason))
                    }
                    Err(error) => return Err(error),
                }
            } else {
                let reason = if matches!(receipt.terminal, Some(WorkerEvent::Unknown { .. })) {
                    "submission outcome is unknown"
                } else {
                    "submission intent has no terminal receipt"
                };
                report.blocked.push(unresolved(&receipt, reason));
            }
        }
        Ok(report)
    }

    pub fn reconcile(
        &self,
        store: &Store,
        attempt_id: &str,
        outcome: ReconcileOutcome,
    ) -> Result<()> {
        validate_attempt(attempt_id)?;
        let _publisher_lock = publisher_lock(&self.data_dir)?;
        let mut receipt = self.read_receipt(attempt_id)?;
        if receipt.approved.database_path != store.paths().json {
            return Err(Error::ReconciliationRequired(
                "receipt belongs to a different publication database".into(),
            ));
        }
        if receipt.applied || receipt.not_submitted {
            return Ok(());
        }
        match outcome {
            ReconcileOutcome::Scheduled { actual_url } => {
                let rows = store.read()?;
                let current = snapshot_row(store, find(&rows, &receipt.job.uuid)?)?;
                if current != receipt.approved {
                    return Err(Error::ReconciliationRequired(
                        "approved content/options changed; human correction is required".into(),
                    ));
                }
                let event = WorkerEvent::Scheduled {
                    protocol: 1,
                    attempt_id: attempt_id.to_owned(),
                    uuid: receipt.job.uuid.clone(),
                    url: actual_url,
                };
                match &receipt.terminal {
                    Some(WorkerEvent::Scheduled { .. })
                        if receipt.terminal.as_ref() != Some(&event) =>
                    {
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
            ReconcileOutcome::NotSubmitted => {
                if matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. })) {
                    return Err(Error::ReconciliationRequired(
                        "a scheduled receipt cannot be released as not submitted".into(),
                    ));
                }
                receipt.not_submitted = true;
                self.save_receipt(attempt_id, &receipt)
            }
        }
    }

    fn attempt_path(&self, attempt_id: &str) -> Result<PathBuf> {
        validate_attempt(attempt_id)?;
        Ok(self.root.join(attempt_id))
    }
    fn read_receipt(&self, attempt_id: &str) -> Result<Receipt> {
        let attempt = self.attempt_path(attempt_id)?;
        for directory in [
            self.data_dir.join(".publicator"),
            self.root.clone(),
            attempt.clone(),
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
        let bytes = fs::read(path)?;
        if bytes.is_empty() {
            return Err(Error::Protocol("empty publish receipt".into()));
        }
        let receipt: Receipt = serde_json::from_slice(&bytes)?;
        validate_attempt(&receipt.job.attempt_id)?;
        validate_attempt(&receipt.job.uuid)?;
        if receipt.job.attempt_id != attempt_id
            || receipt.job.uuid != receipt.approved.uuid
            || receipt.approved.platform != "deviantart"
            || receipt.approved.database_path
                != normalize_database_path(&receipt.approved.database_path)?
            || receipt.applied && !matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. }))
            || receipt.not_submitted
                && matches!(receipt.terminal, Some(WorkerEvent::Scheduled { .. }))
        {
            return Err(Error::Protocol(
                "publish receipt identity or state is invalid".into(),
            ));
        }
        if let Some(event) = &receipt.terminal {
            match event {
                WorkerEvent::Scheduled {
                    protocol,
                    attempt_id: event_id,
                    uuid,
                    ..
                }
                | WorkerEvent::Failed {
                    protocol,
                    attempt_id: event_id,
                    uuid,
                    ..
                }
                | WorkerEvent::Unknown {
                    protocol,
                    attempt_id: event_id,
                    uuid,
                    ..
                } if *protocol == 1 && event_id == attempt_id && uuid == &receipt.job.uuid => {}
                _ => {
                    return Err(Error::Protocol(
                        "publish receipt terminal identity is invalid".into(),
                    ));
                }
            }
        }
        Ok(receipt)
    }
    fn save_receipt(&self, attempt_id: &str, receipt: &Receipt) -> Result<()> {
        write_atomic(
            &self.attempt_path(attempt_id)?.join("receipt.json"),
            &serde_json::to_vec_pretty(receipt)?,
        )
    }
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
        && strings(app, "tiers")? == approved.tiers
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
        match fs::create_dir(dir) {
            Ok(()) => created.push(dir.to_path_buf()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && dir == attempt => {
                return Err(Error::InvalidState("publish attempt already exists".into()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(dir)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(Error::Validation(
                        "publish journal ancestors must be real directories".into(),
                    ));
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    for dir in created {
        sync(dir.parent().unwrap())?;
    }
    sync(attempt)?;
    Ok(())
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

    #[test]
    fn approved_edit_after_hash_check_is_rejected_inside_the_fresh_store_mutation() {
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
                url: None,
            })
            .unwrap();
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
