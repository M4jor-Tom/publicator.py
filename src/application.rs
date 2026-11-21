use crate::{
    artwork::Artwork,
    error::{Error, Result},
    images::{PreparedImage, prepare_ai_image_until},
    metadata::{Generated, GenerationRequest, MetadataClient, Source, VISION_MODELS},
    model::{GENERATE_MARKER, Phase, phase},
    store::Store,
    workflow,
};
use serde_json::Value;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordResult {
    pub uuid: String,
    pub saved: bool,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<RecordResult>,
}
/// A marked batch's progress, reported in order: the admitted ids once, then each record's start
/// and finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchEvent {
    Admitted(Vec<String>),
    Started(String),
    Finished(RecordResult),
}

pub struct Application {
    store: Arc<Store>,
    metadata: MetadataClient,
    operation: Mutex<()>,
    busy: Mutex<HashSet<String>>,
    publishing: Mutex<bool>,
    pub(crate) publisher: Option<crate::publishing::Publisher>,
}
impl Application {
    pub fn new(store: Arc<Store>, metadata: MetadataClient) -> Self {
        Self {
            store,
            metadata,
            operation: Mutex::new(()),
            busy: Mutex::new(HashSet::new()),
            publishing: Mutex::new(false),
            publisher: None,
        }
    }
    pub(crate) fn store(&self) -> &Store {
        &self.store
    }
    pub async fn local_status(
        &self,
        model: &crate::metadata::ModelOption,
        timeout: std::time::Duration,
    ) -> Result<crate::metadata::LocalStatus> {
        self.metadata
            .local_status(model, tokio::time::Instant::now() + timeout)
            .await
    }
    pub fn begin_publishing(&self) -> Result<PublishingLease<'_>> {
        let _op = self.operation.lock().unwrap();
        let mut publishing = self.publishing.lock().unwrap();
        if *publishing || !self.busy.lock().unwrap().is_empty() {
            return Err(Error::Busy(
                "publishing or metadata generation is already active".into(),
            ));
        }
        *publishing = true;
        Ok(PublishingLease { application: self })
    }
    pub fn mutate<R>(&self, uuid: Option<&str>, f: impl FnOnce(&Store) -> Result<R>) -> Result<R> {
        let _op = self.operation.lock().unwrap();
        if *self.publishing.lock().unwrap()
            || uuid.is_some_and(|id| self.busy.lock().unwrap().contains(id))
        {
            return Err(Error::Busy("publication is being processed".into()));
        }
        f(&self.store)
    }
    /// Deletes an undelivered publication entry. Holds the publisher lock so no batch or
    /// reconciliation runs meanwhile, and refuses a publication an unresolved attempt names:
    /// its receipt must stay reconcilable against the row.
    pub fn delete_publication(&self, uuid: &str, now: i64) -> Result<()> {
        self.unattempted(uuid, "deletion", || {
            self.mutate(Some(uuid), |store| {
                workflow::delete_publication(store, uuid, now)
            })
        })
    }
    /// Moves an undelivered publication back one state (DESCRIBED to SCHEDULED, SCHEDULED to
    /// PICKED) under the same guards as deletion, returning the new phase.
    pub fn roll_back(&self, uuid: &str) -> Result<Phase> {
        self.unattempted(uuid, "rollback", || {
            self.mutate(Some(uuid), |store| {
                match phase(lookup(&store.read()?, uuid)?)? {
                    Phase::Described => {
                        workflow::return_to_metadata(store, uuid).map(|()| Phase::Scheduled)
                    }
                    Phase::Scheduled => {
                        workflow::return_to_scheduling(store, uuid).map(|()| Phase::Picked)
                    }
                    Phase::Picked => Err(Error::InvalidState(
                        "PICKED has no earlier state; use delete-publication to unpick it".into(),
                    )),
                    Phase::Appeared => Err(Error::InvalidState(
                        "APPEARED publications cannot be rolled back".into(),
                    )),
                }
            })
        })
    }
    /// Runs `f` under the publisher lock unless an unresolved attempt names `uuid`.
    fn unattempted<R>(&self, uuid: &str, action: &str, f: impl FnOnce() -> Result<R>) -> Result<R> {
        let data_dir = &self.store.paths().data_dir;
        let _lock = crate::publish_journal::publisher_lock(data_dir)?;
        if crate::publish_journal::PublishJournal::new(data_dir)?
            .blocked_attempts_locked(&self.store)?
            .iter()
            .any(|attempt| attempt.uuid == uuid)
        {
            return Err(Error::ReconciliationRequired(format!(
                "unresolved publish attempt blocks {action}"
            )));
        }
        f()
    }
    pub async fn generate_one(&self, uuid: &str, request: GenerationRequest) -> Result<Generated> {
        validate_request(&request)?;
        let deadline = Instant::now() + request.timeout;
        let _op = self.operation.lock().unwrap();
        let rows = self.store.read()?;
        if *self.publishing.lock().unwrap() {
            return Err(Error::Busy("publishing is already active".into()));
        }
        let row = lookup(&rows, uuid)?;
        if phase(row)? != Phase::Scheduled {
            return Err(Error::InvalidState(
                "generation requires SCHEDULED publication".into(),
            ));
        }
        let image = crate::artwork::resolve(&self.store.paths().data_dir, row)?;
        // The draft generation started from; completion overwrites only fields still holding it.
        let admitted = WorkflowMetadata::of(row);
        self.reserve(vec![uuid.to_owned()])?;
        drop(_op);
        let _lease = Lease {
            busy: &self.busy,
            ids: vec![uuid.to_owned()],
        };
        // Like a batch, the row is validated and reserved first: invalid source data runs no preflight.
        self.preflight_local(&request, deadline).await?;
        self.generate_accepted(uuid, &image, &admitted, &request, deadline)
            .await
    }
    pub async fn generate_marked(&self, request: &GenerationRequest) -> Result<BatchReport> {
        self.generate_marked_with_events(request, None, |_| {})
            .await
    }

    pub async fn generate_marked_expected(
        &self,
        request: &GenerationRequest,
        expected_ids: &[String],
    ) -> Result<BatchReport> {
        self.generate_marked_with_events(request, Some(expected_ids), |_| {})
            .await
    }

    /// Runs the marked batch, reporting its progress to `on_event`. Each record's reservation is
    /// released as soon as it finishes, before its `Finished` event.
    pub async fn generate_marked_with_events(
        &self,
        request: &GenerationRequest,
        expected_ids: Option<&[String]>,
        mut on_event: impl FnMut(BatchEvent),
    ) -> Result<BatchReport> {
        validate_request(request)?;
        let ids = self.admit_marked(request.source, expected_ids)?;
        if ids.is_empty() {
            return Ok(BatchReport::default());
        }
        let mut lease = Lease {
            busy: &self.busy,
            ids: ids.clone(),
        };
        on_event(BatchEvent::Admitted(ids.clone()));
        self.preflight_local(request, Instant::now() + request.timeout)
            .await?;
        let mut report = BatchReport::default();
        let mut preferred: Option<String> = None;
        for uuid in ids {
            on_event(BatchEvent::Started(uuid.clone()));
            let result = self
                .generate_marked_record(&uuid, request, &mut preferred)
                .await;
            match result {
                Ok(()) => {
                    report.succeeded += 1;
                    report.results.push(RecordResult {
                        uuid: uuid.clone(),
                        saved: true,
                        error: None,
                    });
                }
                Err(error) => record_batch_error(&mut report, uuid.clone(), error)?,
            }
            lease.release(&uuid);
            on_event(BatchEvent::Finished(report.results.last().unwrap().clone()));
        }
        Ok(report)
    }
    fn admit_marked(&self, source: Source, expected_ids: Option<&[String]>) -> Result<Vec<String>> {
        let _op = self.operation.lock().unwrap();
        let rows = self.store.read()?;
        if *self.publishing.lock().unwrap() {
            return Err(Error::Busy("publishing is already active".into()));
        }
        let mut ids: Vec<String> = rows
            .iter()
            .filter(|row| {
                phase(row).ok() == Some(Phase::Scheduled)
                    && ["title", "description"]
                        .iter()
                        .any(|key| row.get(*key).and_then(Value::as_str) == Some(GENERATE_MARKER))
            })
            .filter_map(|row| row.get("uuid").and_then(Value::as_str).map(str::to_owned))
            .collect();
        if expected_ids.is_some_and(|expected| expected != ids) {
            return Err(Error::InvalidState(
                "marked batch changed while local install consent was pending".into(),
            ));
        }
        ids = batch_ids(ids, source);
        if ids.is_empty() {
            return Ok(ids);
        }
        self.reserve(ids.clone())?;
        Ok(ids)
    }
    async fn generate_marked_record(
        &self,
        uuid: &str,
        request: &GenerationRequest,
        preferred: &mut Option<String>,
    ) -> Result<()> {
        let deadline = Instant::now() + request.timeout;
        let rows = self.store.read()?;
        let row = lookup(&rows, uuid)?;
        let image = crate::artwork::resolve(&self.store.paths().data_dir, row)?;
        let generated = self
            .generate_provider(&image, request, preferred.as_deref(), deadline)
            .await?;
        if preferred.is_none() {
            *preferred = Some(generated.model_used.clone());
        }
        workflow::merge_generated_markers(
            &self.store,
            uuid,
            &WorkflowMetadata {
                title: generated.metadata.title,
                description: generated.metadata.description,
            },
        )
    }
    fn reserve(&self, ids: Vec<String>) -> Result<()> {
        let mut busy = self.busy.lock().unwrap();
        if ids.iter().any(|id| busy.contains(id)) {
            return Err(Error::Busy(
                "UUID metadata generation already active".into(),
            ));
        }
        busy.extend(ids);
        Ok(())
    }
    async fn generate_accepted(
        &self,
        uuid: &str,
        image: &Artwork,
        admitted: &WorkflowMetadata,
        request: &GenerationRequest,
        deadline: Instant,
    ) -> Result<Generated> {
        match self.generate_provider(image, request, None, deadline).await {
            Ok(g) => {
                let pair = WorkflowMetadata {
                    title: g.metadata.title.clone(),
                    description: g.metadata.description.clone(),
                };
                workflow::save_generated_pair(&self.store, uuid, admitted, &pair)?;
                Ok(g)
            }
            Err(error) => {
                workflow::fail_generation(&self.store, uuid, admitted)?;
                Err(error)
            }
        }
    }
    async fn preflight_local(&self, request: &GenerationRequest, deadline: Instant) -> Result<()> {
        if request.source != Source::Local {
            return Ok(());
        }
        let status = self.metadata.local_status(&request.model, deadline).await?;
        if !status.fit.fit {
            return Err(Error::Provider(format!(
                "Local model does not fit: {}",
                status.fit.reason
            )));
        }
        if !status.available {
            return Err(Error::Provider("Ollama is unavailable".into()));
        }
        if !status.installed && !request.install_missing {
            return Err(Error::Provider(format!(
                "Local model {} is not installed",
                request.model.ollama_name
            )));
        }
        Ok(())
    }
    /// Image preparation and every provider request share `deadline`.
    async fn generate_provider(
        &self,
        image: &Artwork,
        request: &GenerationRequest,
        preferred: Option<&str>,
        deadline: Instant,
    ) -> Result<Generated> {
        let prepared = self.prepare_image(image, deadline).await?;
        match request.source {
            Source::OpenRouter => {
                self.metadata
                    .openrouter_generate_until(&prepared, request, preferred, deadline)
                    .await
            }
            Source::Local => {
                self.metadata
                    .prepare_local(&request.model, request.install_missing, deadline)
                    .await?;
                self.metadata
                    .local_generate_until(&prepared, request, deadline)
                    .await
            }
        }
    }
    /// Prepares the recorded bytes off the async runtime; image tools die at `deadline`.
    async fn prepare_image(&self, image: &Artwork, deadline: Instant) -> Result<PreparedImage> {
        let (path, sha512) = (image.path.clone(), image.sha512.clone());
        let cache = self.store.paths().data_dir.join(".thumbs");
        let limit = deadline.into_std();
        let task = tokio::task::spawn_blocking(move || {
            prepare_ai_image_until(&path, &cache, &sha512, limit)
        });
        match tokio::time::timeout_at(deadline, task).await {
            Ok(joined) => joined
                .map_err(|error| Error::InvalidState(format!("AI image preparation: {error}")))?,
            Err(_) => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "AI image preparation exceeded the generation deadline",
            ))),
        }
    }
}
use workflow::Metadata as WorkflowMetadata;
pub struct PublishingLease<'a> {
    application: &'a Application,
}
impl Drop for PublishingLease<'_> {
    fn drop(&mut self) {
        *self.application.publishing.lock().unwrap() = false;
    }
}
struct Lease<'a> {
    busy: &'a Mutex<HashSet<String>>,
    ids: Vec<String>,
}
impl Lease<'_> {
    fn release(&mut self, id: &str) {
        self.ids.retain(|held| held != id);
        self.busy.lock().unwrap().remove(id);
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut busy = self.busy.lock().unwrap();
        for id in &self.ids {
            busy.remove(id);
        }
    }
}
fn lookup<'a>(rows: &'a [Value], uuid: &str) -> Result<&'a Value> {
    rows.iter()
        .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid))
        .ok_or_else(|| Error::NotFound(uuid.into()))
}
fn validate_request(request: &GenerationRequest) -> Result<()> {
    let allowed = VISION_MODELS.iter().any(|model| match request.source {
        Source::OpenRouter => {
            model.openrouter_name == request.model.openrouter_name && model == &request.model
        }
        Source::Local => model.ollama_name == request.model.ollama_name && model == &request.model,
    });
    if !allowed {
        return Err(Error::Validation(
            "model is not in the configured vision catalogue".into(),
        ));
    }
    if request.timeout.is_zero() {
        return Err(Error::Validation(
            "generation timeout must be positive".into(),
        ));
    }
    Ok(())
}
fn record_batch_error(report: &mut BatchReport, uuid: String, error: Error) -> Result<()> {
    if matches!(error, Error::DurabilityUncertain { .. }) {
        return Err(error);
    }
    report.failed += 1;
    report.results.push(RecordResult {
        uuid,
        saved: false,
        error: Some(error.to_string()),
    });
    Ok(())
}

fn batch_ids(mut ids: Vec<String>, source: Source) -> Vec<String> {
    if source == Source::OpenRouter {
        ids.reverse();
    }
    ids
}

#[cfg(test)]
mod ordering_tests {
    use super::{BatchReport, Lease, Source, batch_ids, record_batch_error};
    use crate::error::{Error, Result};
    use std::{collections::HashSet, io, path::PathBuf, sync::Mutex};
    #[test]
    fn uncertain_store_commit_is_propagated_without_recording_ordinary_failure() {
        let mut report = BatchReport::default();
        let error = Error::DurabilityUncertain {
            path: PathBuf::from("publications.json"),
            source: io::Error::other("directory sync failed"),
        };
        let result = record_batch_error(&mut report, "uuid".into(), error);
        assert!(
            matches!(result, Err(Error::DurabilityUncertain { path, .. }) if path == std::path::Path::new("publications.json"))
        );
        assert_eq!(report, BatchReport::default());
    }

    #[test]
    fn propagated_uncertainty_releases_batch_uuid_lease() {
        let busy = Mutex::new(HashSet::from(["uuid".to_string()]));
        let mut report = BatchReport::default();
        let outcome = (|| -> Result<()> {
            let _lease = Lease {
                busy: &busy,
                ids: vec!["uuid".into()],
            };
            record_batch_error(
                &mut report,
                "uuid".into(),
                Error::DurabilityUncertain {
                    path: PathBuf::from("publications.json"),
                    source: io::Error::other("sync failed"),
                },
            )?;
            Ok(())
        })();
        assert!(matches!(outcome, Err(Error::DurabilityUncertain { .. })));
        assert!(busy.lock().unwrap().is_empty());
        assert_eq!(report, BatchReport::default());
    }

    #[test]
    fn batch_order_is_source_specific() {
        let ids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        assert_eq!(
            batch_ids(ids.clone(), Source::OpenRouter),
            vec!["3", "2", "1"]
        );
        assert_eq!(batch_ids(ids, Source::Local), vec!["1", "2", "3"]);
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/common/http.rs"]
mod test_http;

/// Real Store commits fail through the Store's thread-local hook. The hook is one-shot and armed
/// before the call; admission never commits, and each test proves which commit it hit by the
/// provider requests made, the error surfaced and the bytes left behind.
#[cfg(test)]
mod persistence_tests {
    use super::{
        Application, GENERATE_MARKER, GenerationRequest, Source, Store, VISION_MODELS, Value,
        test_http::{Reply, Server, completion},
    };
    use crate::{
        config::{Config, Paths},
        error::Error,
        metadata::{Endpoints, MetadataClient},
        store::{CommitFailureForTest, fail_next_commit_for_test},
    };
    use serde_json::json;
    use std::{fs, sync::Arc, time::Duration};

    fn id(n: u64) -> String {
        format!("00000000-0000-4000-8000-{n:012}")
    }

    /// SCHEDULED rows 1 and 2 over distinct real PNGs and hashes; `marked` rows await generation.
    fn fixture(marked: bool) -> (tempfile::TempDir, Store) {
        let root = tempfile::tempdir().unwrap();
        let text = |plain: &str| if marked { GENERATE_MARKER } else { plain }.to_owned();
        let rows: Vec<Value> = [1u64, 2]
            .iter()
            .map(|n| {
                let image = root.path().join(format!("{n}.png"));
                crate::image_fixture::solid_image(&image, &format!("rgb({n},0,0)"), 2, 2);
                json!({"uuid": id(*n), "state": "SCHEDULED", "title": text("Title"),
                    "description": text("Description"), "submissionTimestamp": 1_700_000_000,
                    "files": [{"basename": format!("{n}.png"),
                        "sha512sum": crate::images::file_sha512(&image).unwrap()}],
                    "apparitions": []})
            })
            .collect();
        let paths = Paths::resolve(Some(root.path()), None).unwrap();
        fs::write(&paths.json, serde_json::to_vec(&rows).unwrap()).unwrap();
        (root, Store::new(paths, Config::default()).unwrap())
    }

    fn app(store: &Store, server: &Server) -> Application {
        let endpoints = Endpoints {
            openrouter: server.url().to_owned(),
            ollama: server.url().to_owned(),
        };
        let metadata = MetadataClient::new(reqwest::Client::new(), endpoints, "llmfit".into());
        Application::new(
            Arc::new(store.clone()),
            metadata.with_openrouter_key("fixture"),
        )
    }

    fn request() -> GenerationRequest {
        GenerationRequest {
            source: Source::OpenRouter,
            model: VISION_MODELS[0].clone(),
            timeout: Duration::from_secs(5),
            install_missing: false,
        }
    }

    fn rows(store: &Store) -> Vec<Value> {
        serde_json::from_slice(&fs::read(&store.paths().json).unwrap()).unwrap()
    }

    /// No UUID stays reserved: both rows are editable and publishing can start.
    fn assert_released(app: &Application) {
        for n in [1, 2] {
            app.mutate(Some(&id(n)), |_| Ok(())).unwrap();
        }
        drop(app.begin_publishing().unwrap());
    }

    #[tokio::test]
    async fn single_success_with_failed_save_keeps_original_bytes_and_surfaces_the_error() {
        let (_root, store) = fixture(false);
        let server = Server::start(vec![completion("Generated")]);
        let app = app(&store, &server);
        let before = fs::read(&store.paths().json).unwrap();
        fail_next_commit_for_test(CommitFailureForTest::BeforeRename);
        let error = app.generate_one(&id(1), request()).await.unwrap_err();
        assert_eq!(server.requests().len(), 1);
        assert!(matches!(&error, Error::Io(e) if e.to_string() == "injected pre-rename failure"));
        assert_eq!(fs::read(&store.paths().json).unwrap(), before);
        assert_released(&app);
    }

    #[tokio::test]
    async fn provider_failure_with_failed_marker_save_surfaces_the_persistence_failure() {
        let (_root, store) = fixture(false);
        let server = Server::start(vec![
            Reply::new(
                "/chat/completions",
                500,
                json!({"error":{"message":"fixture failure"}}),
            ),
            Reply::new("/models", 200, json!({"data": []})),
        ]);
        let app = app(&store, &server);
        let before = fs::read(&store.paths().json).unwrap();
        fail_next_commit_for_test(CommitFailureForTest::BeforeRename);
        let error = app.generate_one(&id(1), request()).await.unwrap_err();
        assert_eq!(server.requests().len(), 2);
        // The markers were not saved, so the error is the save failure, not the provider failure.
        assert!(matches!(&error, Error::Io(e) if e.to_string() == "injected pre-rename failure"));
        assert_eq!(fs::read(&store.paths().json).unwrap(), before);
        assert_released(&app);
    }

    #[tokio::test]
    async fn batch_uncertain_commit_keeps_new_bytes_and_stops_before_the_next_request() {
        let (_root, store) = fixture(true);
        let server = Server::start(vec![completion("Second"), completion("First")]);
        let app = app(&store, &server);
        fail_next_commit_for_test(CommitFailureForTest::AfterRename);
        let error = app.generate_marked(&request()).await.unwrap_err();
        assert!(matches!(error, Error::DurabilityUncertain { .. }));
        assert_eq!(
            server.requests().len(),
            1,
            "no request after an uncertain commit"
        );
        let saved = rows(&store);
        assert_eq!(
            saved[1]["title"], "Second",
            "OpenRouter batches start from the last row"
        );
        assert_eq!(saved[1]["state"], "DESCRIBED");
        assert_eq!(saved[0]["title"], GENERATE_MARKER);
        assert_released(&app);
    }

    #[tokio::test]
    async fn batch_failed_save_is_an_ordinary_failure_and_the_batch_continues() {
        let (_root, store) = fixture(true);
        let original = rows(&store);
        let server = Server::start(vec![completion("Second"), completion("First")]);
        let app = app(&store, &server);
        fail_next_commit_for_test(CommitFailureForTest::BeforeRename);
        let report = app.generate_marked(&request()).await.unwrap();
        assert_eq!((report.succeeded, report.failed), (1, 1));
        let failed = &report.results[0];
        assert_eq!(
            (failed.uuid.as_str(), failed.saved),
            (id(2).as_str(), false)
        );
        assert!(
            failed
                .error
                .as_deref()
                .unwrap()
                .contains("injected pre-rename failure")
        );
        assert_eq!(server.requests().len(), 2);
        let saved = rows(&store);
        assert_eq!(
            saved[1], original[1],
            "the failed save left its row unchanged"
        );
        assert_eq!(saved[0]["title"], "First");
        assert_released(&app);
    }
}
