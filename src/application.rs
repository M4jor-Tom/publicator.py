use crate::{
    error::{Error, Result},
    images::prepare_ai_image,
    metadata::{Generated, GenerationRequest, MetadataClient, Source, VISION_MODELS},
    model::{GENERATE_MARKER, Phase, phase},
    store::Store,
    workflow,
};
use serde_json::Value;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

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

pub struct Application {
    store: Arc<Store>,
    metadata: MetadataClient,
    _clock: fn() -> i64,
    operation: Mutex<()>,
    busy: Mutex<HashSet<String>>,
    publishing: Mutex<bool>,
    pub(crate) publisher: Option<crate::publishing::Publisher>,
}
impl Application {
    pub fn new(store: Arc<Store>, metadata: MetadataClient, clock: fn() -> i64) -> Self {
        Self {
            store,
            metadata,
            _clock: clock,
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
    pub async fn generate_one(&self, uuid: &str, request: GenerationRequest) -> Result<Generated> {
        validate_request(&request)?;
        self.preflight_local(&request).await?;
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
        let image = artwork(&self.store, row)?;
        self.reserve(vec![uuid.to_owned()])?;
        drop(_op);
        let _lease = Lease {
            busy: &self.busy,
            ids: vec![uuid.to_owned()],
        };
        self.generate_accepted(uuid, image, &request).await
    }
    pub async fn generate_marked(&self, request: &GenerationRequest) -> Result<BatchReport> {
        validate_request(request)?;
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
        ids = batch_ids(ids, request.source);
        if ids.is_empty() {
            return Ok(BatchReport::default());
        }
        self.reserve(ids.clone())?;
        drop(_op);
        let _lease = Lease {
            busy: &self.busy,
            ids: ids.clone(),
        };
        self.preflight_local(request).await?;
        let mut report = BatchReport::default();
        let mut preferred: Option<String> = None;
        for uuid in ids {
            let result = async {
                let rows = self.store.read()?;
                let row = lookup(&rows, &uuid)?;
                let image = artwork(&self.store, row)?;
                let generated = self
                    .generate_provider(&image, request, preferred.as_deref())
                    .await;
                match generated {
                    Ok(g) => {
                        if preferred.is_none() {
                            preferred = Some(g.model_used.clone());
                        }
                        workflow::merge_generated_markers(
                            &self.store,
                            &uuid,
                            &WorkflowMetadata {
                                title: g.metadata.title,
                                description: g.metadata.description,
                            },
                        )?;
                        Ok(())
                    }
                    Err(provider_error) => Err(provider_error),
                }
            }
            .await;
            match result {
                Ok(()) => {
                    report.succeeded += 1;
                    report.results.push(RecordResult {
                        uuid,
                        saved: true,
                        error: None,
                    });
                }
                Err(error) => record_batch_error(&mut report, uuid, error)?,
            }
        }
        Ok(report)
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
        image: PathBuf,
        request: &GenerationRequest,
    ) -> Result<Generated> {
        match self.generate_provider(&image, request, None).await {
            Ok(g) => {
                let pair = WorkflowMetadata {
                    title: g.metadata.title.clone(),
                    description: g.metadata.description.clone(),
                };
                if let Err(error) = workflow::save_generated_pair(&self.store, uuid, &pair) {
                    return Err(error);
                }
                Ok(g)
            }
            Err(error) => {
                workflow::fail_generation(&self.store, uuid)?;
                Err(error)
            }
        }
    }
    async fn preflight_local(&self, request: &GenerationRequest) -> Result<()> {
        if request.source != Source::Local {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + request.timeout;
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
    async fn generate_provider(
        &self,
        image: &PathBuf,
        request: &GenerationRequest,
        preferred: Option<&str>,
    ) -> Result<Generated> {
        let prepared = prepare_ai_image(image, &self.store.paths().data_dir.join(".thumbs"), None)?;
        match request.source {
            Source::OpenRouter => {
                self.metadata
                    .openrouter_generate(&prepared, request, preferred)
                    .await
            }
            Source::Local => {
                let deadline = tokio::time::Instant::now() + request.timeout;
                self.metadata
                    .prepare_local(&request.model, request.install_missing, deadline)
                    .await?;
                self.metadata
                    .local_generate_until(&prepared, request, deadline)
                    .await
            }
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
fn artwork(store: &Store, row: &Value) -> Result<PathBuf> {
    let basename = row
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|f| f.get("basename"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation("artwork basename missing".into()))?;
    let name = std::path::Path::new(basename);
    if name.components().count() != 1 || name.file_name().and_then(|n| n.to_str()) != Some(basename)
    {
        return Err(Error::Validation("invalid artwork basename".into()));
    }
    let root = &store.paths().data_dir;
    let found = crate::images::index_by_basename(root)?
        .remove(basename)
        .ok_or_else(|| Error::NotFound(format!("artwork {basename}")))?;
    Ok(found)
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
            matches!(result, Err(Error::DurabilityUncertain { path, .. }) if path == PathBuf::from("publications.json"))
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
