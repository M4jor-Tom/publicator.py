pub mod calendar;
pub mod describe;
pub mod drafts;
pub mod pick;
mod preview;
pub mod prompt;
pub mod publish;
pub mod schedule;

use crate::{
    application::Application as CoreApplication,
    images::Candidate,
    metadata::{GenerationRequest, LocalStatus, ModelOption, Source, VISION_MODELS},
    model::{Phase, phase},
    prompts::{PromptArchive, PromptMatch},
    publish_journal::{AttemptState, AttemptStatus, ReconcileOutcome},
    publishing::{PublishCell, PublishCellOutcome, PublishCellState, PublishCompletion},
    workflow::{self, MetadataPatch},
};
use iced::{
    Element, Subscription, Task, Theme,
    widget::{button, column, container, row, text},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use drafts::{Draft, Field, SentDraft};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tab {
    Pick,
    Schedule,
    Describe,
    Publish,
    Calendar,
}

impl Tab {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Pick, "Pick"),
        (Self::Schedule, "Schedule"),
        (Self::Describe, "Describe"),
        (Self::Publish, "Publish"),
        (Self::Calendar, "Calendar"),
    ];
}

#[derive(Clone, Debug)]
pub struct UiOptions {
    pub openrouter_model: Option<String>,
    pub ai_timeout: Duration,
}

pub fn run(application: Arc<CoreApplication>, options: UiOptions) -> iced::Result {
    iced::application(
        move || App::new(application.clone(), options.clone()),
        App::update,
        App::view,
    )
    .subscription(App::subscription)
    .theme(app_theme)
    .window_size((1100.0, 760.0))
    .centered()
    .exit_on_close_request(false)
    .run()
}

#[derive(Debug, Clone)]
pub enum Message {
    LoadFinished(Result<Vec<Value>, String>),
    Navigate(Tab),
    Edited {
        uuid: String,
        field: Field,
        text: String,
    },
    DescriptionAction {
        uuid: String,
        action: iced::widget::text_editor::Action,
    },
    Tick,
    SaveFinished {
        uuid: String,
        sent: SentDraft,
        result: Result<(), String>,
    },
    CloseRequested,
    ConfirmDiscard,
    KeepEditing,
    Refresh,
    RetrySave,
    Search,
    SearchLimit(usize),
    SearchPath(String),
    SearchFinished(Result<Vec<Candidate>, String>),
    PreviewFinished {
        source: std::path::PathBuf,
        result: Result<std::path::PathBuf, String>,
    },
    PreviewViewport {
        offset: f32,
        height: f32,
    },
    DescribeViewport {
        offset: f32,
        height: f32,
    },
    DescribePreviewFinished {
        uuid: String,
        result: Result<std::path::PathBuf, String>,
    },
    Pick(Candidate),
    PickFinished(Result<(), String>),
    AskDelete(Candidate),
    ConfirmDelete,
    CancelDelete,
    WorkflowFinished {
        message: String,
        result: Result<(), String>,
    },
    CustomDate {
        uuid: String,
        value: String,
    },
    CustomTime {
        uuid: String,
        value: String,
    },
    SaveCustomDate(String),
    PresetDate {
        uuid: String,
        profile: String,
    },
    SuggestedDate(String),
    ApproveDate(String),
    ReturnToPick(String),
    SaveDraft(String),
    ApproveMetadata(String),
    ReturnToSchedule(String),
    GenerateOne(String),
    GenerateMarked,
    ConfirmLocalInstall,
    CancelLocalInstall,
    SelectSource(Source),
    SelectModel(ModelOption),
    GenerationFinished {
        uuid: Option<String>,
        result: Result<String, String>,
    },
    LocalFitFinished {
        request_id: u64,
        model: ModelOption,
        selection: Option<String>,
        batch_selection: Option<Vec<String>>,
        generate: bool,
        result: Result<LocalStatus, String>,
    },
    SelectCell {
        key: String,
        selected: bool,
    },
    SelectAll(bool),
    OptionPriceChanged {
        uuid: String,
        value: String,
    },
    OptionTierChanged {
        uuid: String,
        value: String,
        selected: bool,
    },
    OptionGalleryChanged {
        uuid: String,
        value: String,
        selected: bool,
    },
    SaveOptions(String),
    OptionsSaved {
        uuid: String,
        result: Result<(), String>,
    },
    PublishLocksFinished(Result<HashSet<String>, String>),
    PublishStatusesFinished(Result<Vec<AttemptStatus>, String>),
    PublishSelected,
    CancelAfterCurrent,
    ReconcileUrlChanged {
        attempt_id: String,
        value: String,
    },
    ReconcileAttempt {
        attempt_id: String,
        outcome: ReconcileOutcome,
    },
    ReconcileFinished {
        attempt_id: String,
        result: Result<(), String>,
    },
    OpenActualUrl(String),
    JumpToPublication(String),
    PublishFinished(PublishCompletion),
    PromptMatchesFinished(Vec<(PathBuf, PromptMatch)>),
    CalendarPrepared(Result<calendar::Preparation, String>),
    CalendarPreviewFinished {
        basename: String,
        result: Result<PathBuf, String>,
    },
    CalendarViewport {
        offset: f32,
        height: f32,
    },
    UrlOpened {
        url: String,
        result: Result<(), String>,
    },
    CloseWait,
    CloseCancelPublish,
}

#[derive(Clone, Debug)]
struct PublishOptionsDraft {
    price: String,
    tiers: Vec<String>,
    galleries: Vec<String>,
    dirty: bool,
    error: Option<String>,
}

struct LocalInstallConsent {
    request_id: u64,
    model: ModelOption,
    selection: Option<String>,
    batch_selection: Option<Vec<String>>,
}

pub struct App {
    application: Arc<CoreApplication>,
    _options: UiOptions,
    active_tab: Tab,
    publications: Option<Vec<Value>>,
    load_error: Option<String>,
    search_authorized: bool,
    drafts: HashMap<String, Draft>,
    busy: bool,
    result: Option<String>,
    pending_navigation: Option<Tab>,
    pending_close: bool,
    pending_jump: Option<String>,
    revealed_publication: Option<String>,
    started: Instant,
    search_limit: usize,
    search_paths: Vec<String>,
    candidates: Vec<Candidate>,
    previews: HashMap<std::path::PathBuf, Option<Result<std::path::PathBuf, String>>>,
    describe_previews: HashMap<String, Option<Result<std::path::PathBuf, String>>>,
    description_editors: HashMap<String, iced::widget::text_editor::Content>,

    pending_delete: Option<Candidate>,
    custom_dates: HashMap<String, (String, String)>,
    source: Source,
    model: ModelOption,
    busy_uuids: HashSet<String>,
    pending_generation: Option<String>,
    pending_generation_install: bool,
    pending_generation_batch: Option<Vec<String>>,
    pending_approval: Option<String>,
    pending_return: Option<String>,
    pending_local_consent: Option<LocalInstallConsent>,
    local_fit: Option<LocalStatus>,
    local_status_request: u64,
    publish_result: Option<String>,
    publish_diagnostic: Option<String>,
    publish_cancel: Arc<std::sync::atomic::AtomicBool>,
    publish_active: bool,
    pending_publish: bool,
    publish_options: HashMap<String, PublishOptionsDraft>,
    publish_locks: Option<HashSet<String>>,
    publish_lock_error: Option<String>,
    publish_attempts: Vec<AttemptStatus>,
    publish_outcomes: HashMap<PublishCell, PublishCellOutcome>,
    last_publish_cells: HashSet<PublishCell>,
    reconcile_urls: HashMap<String, String>,
    prompt_archive: Option<Arc<PromptArchive>>,
    prompt_archive_error: Option<String>,
    prompt_matches: HashMap<PathBuf, PromptMatch>,
    calendar_preparing: bool,
    calendar_prepared: bool,
    calendar_error: Option<String>,
    calendar_artwork: HashMap<String, Option<PathBuf>>,
    calendar_previews: HashMap<String, Option<Result<PathBuf, String>>>,
    url_opener: fn(&str) -> Result<(), String>,
}

impl App {
    fn new(application: Arc<CoreApplication>, options: UiOptions) -> (Self, Task<Message>) {
        Self::new_with_url_opener(application, options, calendar::system_open_url)
    }

    fn new_with_url_opener(
        application: Arc<CoreApplication>,
        options: UiOptions,
        url_opener: fn(&str) -> Result<(), String>,
    ) -> (Self, Task<Message>) {
        let (prompt_archive, prompt_archive_error) =
            match application.store().config().prompts.as_ref() {
                Some(config) => {
                    match PromptArchive::new(&application.store().paths().data_dir, config) {
                        Ok(archive) => (Some(Arc::new(archive)), None),
                        Err(error) => (None, Some(error.to_string())),
                    }
                }
                None => (None, None),
            };
        let app = Self {
            application: application.clone(),
            _options: options,
            active_tab: Tab::Pick,
            publications: None,
            load_error: None,
            search_authorized: false,
            drafts: HashMap::new(),
            busy: false,
            result: None,
            pending_navigation: None,
            pending_close: false,
            pending_jump: None,
            revealed_publication: None,
            started: Instant::now(),
            search_limit: 25,
            search_paths: application.store().config().publicable.clone(),
            candidates: Vec::new(),
            previews: HashMap::new(),
            describe_previews: HashMap::new(),
            description_editors: HashMap::new(),
            pending_delete: None,
            custom_dates: HashMap::new(),
            source: Source::OpenRouter,
            model: VISION_MODELS[0].clone(),
            busy_uuids: HashSet::new(),
            pending_generation: None,
            pending_generation_install: false,
            pending_generation_batch: None,
            pending_approval: None,
            pending_return: None,
            pending_local_consent: None,
            local_fit: None,
            local_status_request: 0,
            publish_result: None,
            publish_diagnostic: None,
            publish_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            publish_active: false,
            pending_publish: false,
            publish_options: HashMap::new(),
            publish_locks: None,
            publish_lock_error: None,
            publish_attempts: Vec::new(),
            publish_outcomes: HashMap::new(),
            last_publish_cells: HashSet::new(),
            reconcile_urls: HashMap::new(),
            prompt_archive,
            prompt_archive_error,
            prompt_matches: HashMap::new(),
            calendar_preparing: false,
            calendar_prepared: false,
            calendar_error: None,
            calendar_artwork: HashMap::new(),
            calendar_previews: HashMap::new(),
            url_opener,
        };
        (app, Self::load_task(application))
    }

    pub fn from_application(
        application: Arc<CoreApplication>,
        options: UiOptions,
    ) -> (Self, Task<Message>) {
        Self::new(application, options)
    }

    pub fn from_application_with_url_opener(
        application: Arc<CoreApplication>,
        options: UiOptions,
        opener: fn(&str) -> Result<(), String>,
    ) -> (Self, Task<Message>) {
        Self::new_with_url_opener(application, options, opener)
    }

    pub fn search_limit(&self) -> usize {
        self.search_limit
    }
    pub fn search_paths(&self) -> &[String] {
        &self.search_paths
    }
    pub fn current_tab(&self) -> Tab {
        self.active_tab
    }
    pub fn local_fit_status(&self) -> Option<&LocalStatus> {
        self.local_fit.as_ref()
    }
    pub fn local_install_consent_pending(&self) -> bool {
        self.pending_local_consent.is_some()
    }
    pub fn publish_attempts(&self) -> &[AttemptStatus] {
        &self.publish_attempts
    }
    pub fn prompt_match(&self, path: &Path) -> Option<&PromptMatch> {
        self.prompt_matches.get(path)
    }
    pub fn calendar_prepared(&self) -> bool {
        self.calendar_prepared
    }
    pub fn calendar_art_path(&self, basename: &str) -> Option<&Path> {
        self.calendar_artwork.get(basename)?.as_deref()
    }
    pub fn revealed_publication(&self) -> Option<&str> {
        self.revealed_publication.as_deref()
    }
    pub fn unresolved_publish_attempts(&self) -> impl Iterator<Item = &AttemptStatus> {
        self.publish_attempts
            .iter()
            .filter(|attempt| attempt.blocks_retry())
    }
    pub fn publish_cell_outcome(&self, uuid: &str) -> Option<PublishCellOutcome> {
        let row = self.publication(uuid)?;
        let apparition = row
            .get("apparitions")
            .and_then(Value::as_array)?
            .iter()
            .find(|app| app.get("platformName").and_then(Value::as_str) == Some("deviantart"))?;
        let cell = PublishCell {
            uuid: uuid.to_owned(),
            platform: "deviantart".into(),
        };
        let outcome = if let Some(attempt) = self
            .publish_attempts
            .iter()
            .find(|attempt| attempt.uuid == uuid && attempt.blocks_retry())
        {
            let detail = if !attempt.database_matches {
                "receipt belongs to a different publication database".into()
            } else if let Some(error) = &attempt.current_error {
                format!("{}: {error}", attempt.reason())
            } else if attempt.current_matches == Some(false) {
                format!(
                    "{}; approved snapshot differs from current publication",
                    attempt.reason()
                )
            } else {
                match &attempt.state {
                    AttemptState::Unknown { error } => error.clone(),
                    _ => attempt.reason().into(),
                }
            };
            PublishCellOutcome {
                cell,
                state: PublishCellState::Unknown,
                detail: Some(detail),
            }
        } else if apparition.get("state").and_then(Value::as_str) == Some("published_or_scheduled")
        {
            PublishCellOutcome {
                cell,
                state: PublishCellState::Success,
                detail: None,
            }
        } else if let Some(attempt) = self.publish_attempts.iter().find(|attempt| {
            attempt.uuid == uuid && matches!(&attempt.state, AttemptState::SafeFailed { .. })
        }) {
            let detail = match &attempt.state {
                AttemptState::SafeFailed { error } => error.clone(),
                _ => unreachable!(),
            };
            PublishCellOutcome {
                cell,
                state: PublishCellState::SafeFailed,
                detail: Some(detail),
            }
        } else if self.publish_attempts.iter().any(|attempt| {
            attempt.uuid == uuid && matches!(&attempt.state, AttemptState::NotSubmitted)
        }) {
            PublishCellOutcome {
                cell,
                state: PublishCellState::ConfirmedNotSubmitted,
                detail: None,
            }
        } else if let Some(outcome) = self.publish_outcomes.get(&cell) {
            outcome.clone()
        } else if self.last_publish_cells.contains(&cell) {
            PublishCellOutcome {
                cell,
                state: PublishCellState::Unattempted,
                detail: None,
            }
        } else {
            PublishCellOutcome {
                cell,
                state: PublishCellState::Unattempted,
                detail: None,
            }
        };
        Some(outcome)
    }

    fn load_task(application: Arc<CoreApplication>) -> Task<Message> {
        let lock_application = application.clone();
        Task::batch([
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        application
                            .mutate(None, |store| store.read())
                            .map_err(|error| error.to_string())
                    })
                    .await
                    .unwrap_or_else(|error| Err(format!("Could not load publications: {error}")))
                },
                Message::LoadFinished,
            ),
            Self::publish_locks_task(lock_application),
        ])
    }

    fn publish_locks_task(application: Arc<CoreApplication>) -> Task<Message> {
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let data_dir = application.store().paths().data_dir.clone();
                    let _lock = crate::publish_journal::publisher_lock(&data_dir)?;
                    crate::publish_journal::PublishJournal::new(&data_dir)?
                        .attempt_statuses_locked(application.store())
                })
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())
            },
            Message::PublishStatusesFinished,
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoadFinished(Ok(rows)) => {
                self.description_editors.retain(|uuid, _| {
                    rows.iter()
                        .any(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid))
                });
                for row in &rows {
                    let Some(uuid) = row.get("uuid").and_then(Value::as_str) else {
                        continue;
                    };
                    let description = row.get("description").and_then(Value::as_str).unwrap_or("");
                    let editor = self
                        .description_editors
                        .entry(uuid.to_owned())
                        .or_insert_with(|| {
                            iced::widget::text_editor::Content::with_text(description)
                        });
                    if !self
                        .drafts
                        .get(uuid)
                        .is_some_and(|draft| draft.is_field_dirty(Field::Description))
                    {
                        *editor = iced::widget::text_editor::Content::with_text(description);
                    }
                }
                self.publications = Some(rows);
                self.load_error = None;
                self.search_authorized = true;
                self.calendar_preparing = false;
                self.calendar_prepared = false;
                self.calendar_error = None;
                self.calendar_artwork.clear();
                self.calendar_previews.clear();
                if self.active_tab == Tab::Describe {
                    return describe::preview_tasks(self, 0, 4);
                }
                if self.active_tab == Tab::Calendar {
                    return calendar::prepare_task(self);
                }
            }
            Message::LoadFinished(Err(error)) => {
                self.publications = None;
                self.load_error = Some(error);
                self.search_authorized = false;
            }
            Message::PublishLocksFinished(Ok(uuids)) => {
                self.publish_locks = Some(uuids);
                self.publish_lock_error = None;
            }
            Message::PublishLocksFinished(Err(error)) => {
                self.publish_locks = None;
                self.publish_lock_error = Some(error);
            }
            Message::PublishStatusesFinished(Ok(attempts)) => {
                self.publish_locks = Some(
                    attempts
                        .iter()
                        .filter(|attempt| attempt.blocks_retry())
                        .map(|attempt| attempt.uuid.clone())
                        .collect(),
                );
                self.publish_lock_error = None;
                self.publish_attempts = attempts;
                if self.pending_close {
                    if self.any_dirty() {
                        return self.save_next_dirty();
                    }
                    if self.publish_options.values().any(|options| options.dirty) {
                        return self.save_next_options();
                    }
                    self.pending_close = false;
                    return iced::exit();
                }
            }
            Message::PublishStatusesFinished(Err(error)) => {
                self.publish_locks = None;
                self.publish_lock_error = Some(error);
                self.publish_attempts.clear();
                if self.pending_close {
                    if self.any_dirty() {
                        return self.save_next_dirty();
                    }
                    if self.publish_options.values().any(|options| options.dirty) {
                        return self.save_next_options();
                    }
                    self.pending_close = false;
                    return iced::exit();
                }
            }
            Message::Navigate(tab) => {
                self.revealed_publication = None;
                self.pending_jump = None;
                return self.navigate(tab);
            }
            Message::SelectSource(source)
                if self.pending_local_consent.is_none() && self.pending_generation.is_none() =>
            {
                self.local_status_request = self.local_status_request.wrapping_add(1);
                self.source = source;
                self.local_fit = None;
                if source == Source::Local {
                    return self.local_fit_task(None, None, false);
                }
            }
            Message::SelectSource(_) => {}
            Message::SelectModel(model)
                if self.pending_local_consent.is_none()
                    && self.pending_generation.is_none()
                    && VISION_MODELS.contains(&model) =>
            {
                self.local_status_request = self.local_status_request.wrapping_add(1);
                self.model = model;
                self.local_fit = None;
                if self.source == Source::Local {
                    return self.local_fit_task(None, None, false);
                }
            }
            Message::SelectModel(_) => {}
            Message::SaveDraft(uuid) => return self.save_uuid(&uuid),
            Message::ApproveMetadata(uuid) => return self.approve_metadata(uuid),
            Message::ReturnToSchedule(uuid) => return self.return_to_schedule(uuid),
            Message::GenerateOne(uuid) => return self.request_generation(uuid),
            Message::GenerateMarked => return self.request_batch_generation(),
            Message::ConfirmLocalInstall => {
                if let Some(consent) = self.pending_local_consent.take()
                    && consent.request_id == self.local_status_request
                    && self.source == Source::Local
                    && self.model == consent.model
                {
                    if let Some(batch_selection) = consent.batch_selection {
                        if self.marked_generation_ids() == batch_selection {
                            return self
                                .confirm_generation(consent.selection, Some(batch_selection));
                        }
                        self.pending_local_consent = None;
                        self.result = Some(
                            "Marked batch changed while local install consent was pending".into(),
                        );
                        return Task::none();
                    } else {
                        return self.confirm_generation(consent.selection, None);
                    }
                }
            }
            Message::CancelLocalInstall => {
                self.pending_local_consent = None;
                self.local_status_request = self.local_status_request.wrapping_add(1);
            }
            Message::LocalFitFinished {
                request_id,
                model,
                selection,
                batch_selection,
                generate,
                result,
            } => {
                if request_id != self.local_status_request
                    || self.source != Source::Local
                    || self.model != model
                {
                    return Task::none();
                }
                match result {
                    Ok(status) => {
                        let needs_consent = crate::metadata::consent_required(&status);
                        let can_generate = status.fit.fit && status.available;
                        self.local_fit = Some(status);
                        if generate && needs_consent {
                            self.pending_local_consent = Some(LocalInstallConsent {
                                request_id,
                                model,
                                selection,
                                batch_selection,
                            });
                        } else if generate && can_generate {
                            return self.continue_local_generation(selection, batch_selection);
                        }
                    }
                    Err(error) => {
                        self.local_fit = None;
                        self.result = Some(format!("Could not assess local model fit: {error}"));
                    }
                }
            }
            Message::GenerationFinished { uuid, result } => {
                if let Some(uuid) = uuid {
                    self.busy_uuids.remove(&uuid);
                } else {
                    self.busy_uuids.clear();
                }
                self.busy = false;
                self.result = Some(match result {
                    Ok(message) => message,
                    Err(error) => format!("Generation failed: {error}"),
                });
                return Self::load_task(self.application.clone());
            }
            Message::DescriptionAction { uuid, action } => {
                if self.busy_uuids.contains(&uuid) {
                    return Task::none();
                }
                let Some(editor) = self.description_editors.get_mut(&uuid) else {
                    return Task::none();
                };
                editor.perform(action);
                let text = editor.text();
                return self.update(Message::Edited {
                    uuid,
                    field: Field::Description,
                    text,
                });
            }
            Message::Edited { uuid, field, text } => {
                if self.busy_uuids.contains(&uuid) {
                    return Task::none();
                }
                let current = self
                    .drafts
                    .get(&uuid)
                    .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
                    .or_else(|| self.publication(&uuid).map(row_metadata));
                if let Some((title, description)) = current {
                    let draft = self
                        .drafts
                        .entry(uuid)
                        .or_insert_with(|| Draft::new(title, description));
                    draft.edit(field, text, self.started.elapsed().as_millis() as u64);
                }
            }
            Message::Tick => return self.autosave_due(),
            Message::SaveFinished { uuid, sent, result } => {
                if let Some(draft) = self.drafts.get_mut(&uuid) {
                    match result {
                        Ok(()) => {
                            draft.saved(&sent);
                            self.result = Some(format!("Saved metadata for {uuid}"));
                        }
                        Err(error) => {
                            draft.failed(&sent);
                            self.result = Some(format!("Save failed for {uuid}: {error}"));
                            self.pending_navigation = None;
                            self.pending_jump = None;
                            self.pending_publish = false;
                            self.pending_generation = None;
                            self.pending_generation_batch = None;
                            self.pending_approval = None;
                            self.pending_return = None;
                            self.pending_close = false;
                            self.busy = false;
                            return Task::none();
                        }
                    }
                }
                self.busy = false;
                if self.drafts.values().any(Draft::is_saving) {
                    return Task::none();
                }
                if self.any_dirty() {
                    return self.save_next_dirty();
                }
                if let Some(uuid) = self.pending_generation.take() {
                    let install = std::mem::take(&mut self.pending_generation_install);
                    let batch = self.pending_generation_batch.take();
                    return self.start_generation(
                        (!uuid.is_empty()).then_some(uuid),
                        install,
                        batch,
                    );
                }
                if let Some(uuid) = self.pending_approval.take() {
                    return self.approve_metadata(uuid);
                }
                if let Some(uuid) = self.pending_return.take() {
                    return self.return_to_schedule(uuid);
                }
                if (self.pending_navigation.is_some() || self.pending_publish || self.pending_close)
                    && self.publish_options.values().any(|options| options.dirty)
                {
                    return self.save_next_options();
                }
                if let Some(tab) = self.pending_navigation.take() {
                    if self.any_dirty() {
                        self.pending_navigation = Some(tab);
                        return self.save_next_dirty();
                    }
                    let reveal = self.pending_jump.take();
                    return self.activate_tab(tab, reveal);
                }
                if self.pending_publish && !self.any_dirty() {
                    self.pending_publish = false;
                    return self.publish_selected_task();
                }
                if self.pending_close && !self.any_dirty() {
                    return iced::exit();
                }
                if self.any_dirty() {
                    return self.save_next_dirty();
                }
            }
            Message::SelectCell { key, selected } => {
                let Some((uuid, platform)) = key.rsplit_once(':') else {
                    return Task::none();
                };
                if platform != "deviantart" {
                    return Task::none();
                }
                if !selected && self.selection_locked(Some(uuid)) {
                    self.publish_result = Some(
                        "Reconcile the unresolved attempt before deselecting this delivery".into(),
                    );
                    return Task::none();
                }
                return Self::select_publish_task(
                    self.application.clone(),
                    Some(uuid.to_owned()),
                    Some(platform.to_owned()),
                    selected,
                );
            }
            Message::SelectAll(selected) => {
                if !selected && self.selection_locked(None) {
                    self.publish_result = Some(
                        "Reconcile unresolved attempts before changing platform selections".into(),
                    );
                    return Task::none();
                }
                return Self::select_publish_task(self.application.clone(), None, None, selected);
            }
            Message::PublishSelected => {
                if self.publish_locks.is_none() || self.publish_lock_error.is_some() {
                    self.publish_result = Some(
                        self.publish_lock_error
                            .clone()
                            .unwrap_or_else(|| "Checking unresolved publish attempts".into()),
                    );
                    return Task::none();
                }
                if self
                    .publish_locks
                    .as_ref()
                    .is_some_and(|locks| !locks.is_empty())
                {
                    self.publish_result =
                        Some("Reconcile unresolved attempts before retrying".into());
                    return Task::none();
                }
                if self.any_dirty() {
                    self.pending_publish = true;
                    return self.save_next_dirty();
                }
                if self.publish_options.values().any(|options| options.dirty) {
                    self.pending_publish = true;
                    return self.save_next_options();
                }
                return self.publish_selected_task();
            }
            Message::CancelAfterCurrent | Message::CloseCancelPublish => self
                .publish_cancel
                .store(true, std::sync::atomic::Ordering::SeqCst),
            Message::CloseWait => {}
            Message::PublishFinished(completion) => {
                self.publish_active = false;
                self.publish_diagnostic = completion.diagnostics;
                match completion.result {
                    Ok(report) => {
                        self.publish_outcomes = report
                            .outcomes
                            .iter()
                            .cloned()
                            .map(|outcome| (outcome.cell.clone(), outcome))
                            .collect();
                        self.publish_result = Some(format!(
                            "Published {}; failed {}; unknown {}; unattempted{}{}",
                            report.published,
                            report.failed,
                            report.unknown,
                            report.unattempted,
                            report
                                .error
                                .map(|error| format!(" — {error}"))
                                .unwrap_or_default()
                        ));
                    }
                    Err(error) => {
                        self.publish_result = Some(format!("Publishing stopped: {error}"));
                    }
                }
                self.publish_locks = None;
                self.publish_lock_error = Some("Refreshing publication and receipt state".into());
                return Self::load_task(self.application.clone());
            }
            Message::OptionPriceChanged { uuid, value } => {
                if self.options_locked(&uuid) {
                    return Task::none();
                }
                self.ensure_publish_options(&uuid);
                if let Some(options) = self.publish_options.get_mut(&uuid) {
                    options.price = value;
                    options.dirty = true;
                    options.error = None;
                }
            }
            Message::OptionTierChanged {
                uuid,
                value,
                selected,
            } => {
                if self.options_locked(&uuid)
                    || !self.application.store().config().tiers.contains(&value)
                {
                    return Task::none();
                }
                self.ensure_publish_options(&uuid);
                if let Some(options) = self.publish_options.get_mut(&uuid) {
                    set_option(&mut options.tiers, value, selected);
                    options.dirty = true;
                    options.error = None;
                }
            }
            Message::OptionGalleryChanged {
                uuid,
                value,
                selected,
            } => {
                if self.options_locked(&uuid)
                    || !self.application.store().config().galleries.contains(&value)
                {
                    return Task::none();
                }
                self.ensure_publish_options(&uuid);
                if let Some(options) = self.publish_options.get_mut(&uuid) {
                    set_option(&mut options.galleries, value, selected);
                    options.dirty = true;
                    options.error = None;
                }
            }
            Message::SaveOptions(uuid) => return self.save_options_task(uuid),
            Message::OptionsSaved { uuid, result } => {
                self.busy = false;
                if let Some(options) = self.publish_options.get_mut(&uuid) {
                    match result {
                        Ok(()) => {
                            self.publish_options.remove(&uuid);
                            self.result = Some(format!("Saved publishing options for {uuid}"));
                        }
                        Err(error) => {
                            options.error = Some(error.clone());
                            self.result =
                                Some(format!("Publishing options were not saved: {error}"));
                            self.pending_publish = false;
                            self.pending_navigation = None;
                            self.pending_close = false;
                            self.pending_jump = None;
                            return Task::none();
                        }
                    }
                }
                if self.publish_options.values().any(|options| options.dirty) {
                    return self.save_next_options();
                }
                if self.pending_publish {
                    self.pending_publish = false;
                    return self.publish_selected_task();
                }
                if let Some(tab) = self.pending_navigation.take() {
                    let reveal = self.pending_jump.take();
                    return self.activate_tab(tab, reveal);
                }
                if self.pending_close {
                    self.pending_close = false;
                    return iced::exit();
                }
                return Self::load_task(self.application.clone());
            }
            Message::ReconcileUrlChanged { attempt_id, value } => {
                self.reconcile_urls.insert(attempt_id, value);
            }
            Message::ReconcileAttempt {
                attempt_id,
                outcome,
            } => {
                if self.publish_active || self.busy {
                    return Task::done(Message::ReconcileFinished {
                        attempt_id,
                        result: Err("reconciliation is busy".into()),
                    });
                }
                if !self
                    .publish_attempts
                    .iter()
                    .any(|attempt| attempt.attempt_id == attempt_id && attempt.blocks_retry())
                {
                    return Task::done(Message::ReconcileFinished {
                        attempt_id,
                        result: Err("publish attempt is no longer unresolved".into()),
                    });
                }
                self.busy = true;
                self.publish_result = Some("Reconciling the verified publishing outcome…".into());
                let application = self.application.clone();
                let reconcile_id = attempt_id.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let journal = crate::publish_journal::PublishJournal::new(
                                &application.store().paths().data_dir,
                            )?;
                            journal.reconcile(application.store(), &reconcile_id, outcome)
                        })
                        .await
                        .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                        .map_err(|error| error.to_string())
                    },
                    move |result| Message::ReconcileFinished { attempt_id, result },
                );
            }
            Message::ReconcileFinished { attempt_id, result } => {
                self.busy = false;
                self.publish_locks = None;
                self.publish_attempts.clear();
                self.publish_result = Some(match result {
                    Ok(()) => format!("Reconciled publish attempt {attempt_id}"),
                    Err(error) => format!("Reconciliation refused; receipt retained: {error}"),
                });
                return Self::load_task(self.application.clone());
            }
            Message::OpenActualUrl(url) => {
                let opener = self.url_opener;
                let application = self.application.clone();
                let result_url = url.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let rows = application
                                .store()
                                .read()
                                .map_err(|error| error.to_string())?;
                            calendar::open_stored_actual_url_with(&rows, &url, opener)
                        })
                        .await
                        .unwrap_or_else(|error| Err(format!("could not open actual URL: {error}")))
                    },
                    move |result| Message::UrlOpened {
                        url: result_url,
                        result,
                    },
                );
            }
            Message::UrlOpened { url, result } => {
                self.result = Some(match result {
                    Ok(()) => format!("Opened actual URL: {url}"),
                    Err(error) => format!("Could not open actual URL: {error}"),
                });
            }
            Message::JumpToPublication(uuid) => return self.jump_to_publication(uuid),
            Message::CloseRequested => {
                if self.publish_active {
                    self.pending_close = true;
                    self.result = Some("Publishing is active. Wait or cancel after the current delivery before closing.".into());
                    return Task::none();
                }
                if self.any_dirty() {
                    self.pending_close = true;
                    self.result = Some("Saving drafts before close…".into());
                    return self.save_next_dirty();
                }
                if self.publish_options.values().any(|options| options.dirty) {
                    self.pending_close = true;
                    return self.save_next_options();
                }
                return iced::exit();
            }
            Message::ConfirmDiscard => {
                self.drafts.clear();
                self.publish_options.clear();
                self.pending_publish = false;
                self.pending_navigation = None;
                self.pending_jump = None;
                self.pending_close = false;
                return iced::exit();
            }
            Message::KeepEditing => self.pending_close = false,
            Message::Refresh => {
                if self.any_dirty() || self.publish_options.values().any(|options| options.dirty) {
                    self.result = Some("Save or explicitly discard drafts before refresh.".into());
                } else {
                    return Self::load_task(self.application.clone());
                }
            }
            Message::RetrySave => return self.save_next_dirty(),
            Message::SearchLimit(limit) => {
                if [0, 25, 50, 75, 100].contains(&limit) {
                    self.search_limit = limit;
                }
            }
            Message::SearchPath(path) => {
                if self.search_paths.contains(&path) {
                    self.search_paths.retain(|p| p != &path);
                } else if self.application.store().config().publicable.contains(&path) {
                    self.search_paths.push(path);
                }
            }
            Message::Search => {
                if self.search_authorized {
                    return pick::search_task(self);
                }
            }
            Message::SearchFinished(Ok(candidates)) => {
                self.candidates = candidates;
                self.pending_delete = None;
                self.previews.clear();
                self.result = Some(
                    if self.candidates.len() == self.search_limit && self.search_limit > 0 {
                        format!(
                            "Found at least {} candidates; search limit reached",
                            self.search_limit
                        )
                    } else {
                        format!("Found {} candidates", self.candidates.len())
                    },
                );
                return Task::batch([pick::preview_tasks(self, 0, 4), prompt_tasks(self)]);
            }
            Message::PromptMatchesFinished(matches) => {
                self.prompt_matches.extend(matches);
            }
            Message::CalendarPrepared(Ok(prepared)) => {
                self.calendar_preparing = false;
                self.calendar_prepared = true;
                self.calendar_error = None;
                self.calendar_artwork = prepared.artwork;
                self.prompt_matches.extend(prepared.prompts);
                return calendar::preview_tasks(self, 0.0, 560.0);
            }
            Message::CalendarPrepared(Err(error)) => {
                self.calendar_preparing = false;
                self.calendar_prepared = true;
                self.calendar_error = Some(error);
            }
            Message::CalendarPreviewFinished { basename, result } => {
                self.calendar_previews.insert(basename, Some(result));
            }
            Message::CalendarViewport { offset, height } => {
                return calendar::preview_tasks(self, offset, height);
            }
            Message::PreviewFinished { source, result } => {
                self.previews.insert(source, Some(result));
            }
            Message::PreviewViewport { offset, height } => {
                let start = (offset.max(0.0) / 250.0) as usize;
                let end = ((offset.max(0.0) + height.max(0.0)) / 250.0).ceil() as usize + 2;
                return pick::preview_tasks(self, start, end);
            }
            Message::DescribeViewport { offset, height } => {
                let start = (offset.max(0.0) / 300.0) as usize;
                let end = ((offset.max(0.0) + height.max(0.0)) / 300.0).ceil() as usize + 2;
                return describe::preview_tasks(self, start, end);
            }
            Message::DescribePreviewFinished { uuid, result } => {
                self.describe_previews.insert(uuid, Some(result));
            }
            Message::SearchFinished(Err(error)) => {
                self.result = Some(format!("Search failed: {error}"))
            }
            Message::Pick(candidate) => return pick::pick_task(self, candidate),
            Message::PickFinished(Ok(())) => {
                self.busy = false;
                self.active_tab = Tab::Schedule;
                self.result = Some("Picked candidate".into());
                return Self::load_task(self.application.clone());
            }
            Message::PickFinished(Err(error)) => {
                self.busy = false;
                self.result = Some(format!("Pick failed: {error}"));
            }
            Message::AskDelete(candidate) => {
                if self
                    .candidates
                    .iter()
                    .any(|authorized| authorized == &candidate)
                {
                    self.pending_delete = Some(candidate);
                }
            }
            Message::ConfirmDelete => return pick::delete_task(self),
            Message::CancelDelete => self.pending_delete = None,
            Message::WorkflowFinished { message, result } => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.result = Some(message);
                        return Self::load_task(self.application.clone());
                    }
                    Err(error) => self.result = Some(format!("{message}: {error}")),
                }
            }
            Message::CustomDate { uuid, value } => {
                let initial = self
                    .publication(&uuid)
                    .map(|row| schedule::date_parts(self, row))
                    .unwrap_or_default();
                let (date, _) = self.custom_dates.entry(uuid).or_insert(initial);
                *date = value;
            }
            Message::CustomTime { uuid, value } => {
                let initial = self
                    .publication(&uuid)
                    .map(|row| schedule::date_parts(self, row))
                    .unwrap_or_default();
                let (_, time) = self.custom_dates.entry(uuid).or_insert(initial);
                *time = value;
            }
            Message::SaveCustomDate(uuid) => return schedule::custom_task(self, uuid),
            Message::PresetDate { uuid, profile } => {
                return schedule::date_task(self, uuid, workflow::DateEdit::Preset(profile));
            }
            Message::SuggestedDate(uuid) => {
                return schedule::date_task(self, uuid, workflow::DateEdit::Suggested);
            }
            Message::ApproveDate(uuid) => return schedule::approve_task(self, uuid),
            Message::ReturnToPick(uuid) => return schedule::return_task(self, uuid),
        }
        Task::none()
    }

    fn navigate(&mut self, tab: Tab) -> Task<Message> {
        if tab == self.active_tab {
            return Task::none();
        }
        if self.any_dirty() {
            self.pending_navigation = Some(tab);
            return self.save_next_dirty();
        }
        if self.publish_options.values().any(|options| options.dirty) {
            self.pending_navigation = Some(tab);
            return self.save_next_options();
        }
        self.activate_tab(tab, None)
    }

    fn activate_tab(&mut self, tab: Tab, reveal: Option<String>) -> Task<Message> {
        self.active_tab = tab;
        self.revealed_publication = reveal.clone();
        let mut tasks = vec![match tab {
            Tab::Describe => describe::preview_tasks(self, 0, 4),
            Tab::Calendar => calendar::prepare_task(self),
            _ => Task::none(),
        }];
        if reveal.is_some() {
            tasks.push(match tab {
                Tab::Schedule => schedule::scroll_to_target(),
                Tab::Describe => describe::scroll_to_target(),
                Tab::Publish => publish::scroll_to_target(),
                _ => Task::none(),
            });
        }
        Task::batch(tasks)
    }

    fn jump_to_publication(&mut self, uuid: String) -> Task<Message> {
        let Some(row) = self.publication(&uuid) else {
            self.result = Some(format!("Publication not found: {uuid}"));
            return Task::none();
        };
        let target = match phase(row) {
            Ok(Phase::Picked) => Tab::Schedule,
            Ok(Phase::Scheduled) => Tab::Describe,
            Ok(Phase::Described | Phase::Appeared) => Tab::Publish,
            Err(error) => {
                self.result = Some(format!("Publication cannot be opened: {error}"));
                return Task::none();
            }
        };
        self.pending_jump = Some(uuid.clone());
        if self.any_dirty() {
            self.pending_navigation = Some(target);
            return self.save_next_dirty();
        }
        if self.publish_options.values().any(|options| options.dirty) {
            self.pending_navigation = Some(target);
            return self.save_next_options();
        }
        self.pending_jump = None;
        self.activate_tab(target, Some(uuid))
    }

    fn select_publish_task(
        application: Arc<CoreApplication>,
        uuid: Option<String>,
        platform: Option<String>,
        selected: bool,
    ) -> Task<Message> {
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let data_dir = application.store().paths().data_dir.clone();
                    let _lock = crate::publish_journal::publisher_lock(&data_dir)?;
                    let journal = crate::publish_journal::PublishJournal::new(&data_dir)?;
                    if !selected
                        && journal
                            .blocked_attempts_locked(application.store())?
                            .iter()
                            .any(|attempt| uuid.as_deref().is_none_or(|id| id == attempt.uuid))
                    {
                        return Err(crate::error::Error::ReconciliationRequired(
                            "unresolved publish attempt cannot be deselected".into(),
                        ));
                    }
                    application.mutate(uuid.as_deref(), |store| {
                        workflow::select(store, uuid.as_deref(), platform.as_deref(), selected)
                    })
                })
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())
            },
            |result| Message::WorkflowFinished {
                message: "Saved platform selection".into(),
                result,
            },
        )
    }

    fn selection_locked(&self, uuid: Option<&str>) -> bool {
        self.publish_locks.is_none()
            || self.publish_lock_error.is_some()
            || self.publish_locks.as_ref().is_some_and(|locks| match uuid {
                Some(uuid) => locks.contains(uuid),
                None => !locks.is_empty(),
            })
    }

    fn options_locked(&self, uuid: &str) -> bool {
        self.busy
            || self.publish_active
            || self.publish_locks.is_none()
            || self.publish_lock_error.is_some()
            || self
                .publish_locks
                .as_ref()
                .is_some_and(|locks| locks.contains(uuid))
    }

    fn ensure_publish_options(&mut self, uuid: &str) {
        if self.publish_options.contains_key(uuid) {
            return;
        }
        let Some(row) = self.publication(uuid) else {
            return;
        };
        let apparition = row
            .get("apparitions")
            .and_then(Value::as_array)
            .and_then(|apps| {
                apps.iter().find(|app| {
                    app.get("platformName").and_then(Value::as_str) == Some("deviantart")
                })
            });
        let draft = PublishOptionsDraft {
            price: apparition
                .and_then(|app| app.get("priceIfNotFree"))
                .and_then(Value::as_f64)
                .map(|price| price.to_string())
                .unwrap_or_default(),
            tiers: apparition
                .and_then(|app| app.get("tiers"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            galleries: apparition
                .and_then(|app| app.get("galleries"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            dirty: false,
            error: None,
        };
        self.publish_options.insert(uuid.to_owned(), draft);
    }

    fn save_next_options(&mut self) -> Task<Message> {
        let uuid = self
            .publish_options
            .iter()
            .find(|(_, options)| options.dirty)
            .map(|(uuid, _)| uuid.clone());
        uuid.map(|uuid| self.save_options_task(uuid))
            .unwrap_or_else(|| Task::none())
    }

    fn save_options_task(&mut self, uuid: String) -> Task<Message> {
        if self.busy {
            if let Some(options) = self.publish_options.get_mut(&uuid) {
                options.error = Some("Another operation is in progress".into());
            }
            self.pending_publish = false;
            self.pending_navigation = None;
            return Task::none();
        }
        if self.publish_active {
            if let Some(options) = self.publish_options.get_mut(&uuid) {
                options.error = Some("Publishing options are busy".into());
            }
            self.pending_publish = false;
            self.pending_navigation = None;
            return Task::done(Message::OptionsSaved {
                uuid,
                result: Err("Publishing options are busy".into()),
            });
        }
        self.ensure_publish_options(&uuid);
        let Some(options) = self.publish_options.get(&uuid) else {
            return Task::none();
        };
        if !options.dirty {
            return Task::none();
        }
        let price = if options.price.trim().is_empty() {
            Some(None)
        } else {
            match options.price.trim().parse::<f64>() {
                Ok(price) if price.is_finite() && price >= 0.0 => Some(Some(price)),
                _ => {
                    if let Some(options) = self.publish_options.get_mut(&uuid) {
                        options.error =
                            Some("Price must be blank or a finite nonnegative number".into());
                    }
                    self.pending_publish = false;
                    self.pending_navigation = None;
                    return Task::done(Message::OptionsSaved {
                        uuid,
                        result: Err("Price must be blank or a finite nonnegative number".into()),
                    });
                }
            }
        };
        let options = crate::workflow::PlatformOptions {
            price,
            tiers: Some(options.tiers.clone()),
            galleries: Some(options.galleries.clone()),
        };
        if self.publish_locks.is_none() || self.publish_lock_error.is_some() {
            if let Some(draft) = self.publish_options.get_mut(&uuid) {
                draft.error = Some(
                    self.publish_lock_error
                        .clone()
                        .unwrap_or_else(|| "Checking unresolved publish attempts".into()),
                );
            }
            self.pending_publish = false;
            self.pending_navigation = None;
            return Task::done(Message::OptionsSaved {
                uuid,
                result: Err(self
                    .publish_lock_error
                    .clone()
                    .unwrap_or_else(|| "Checking unresolved publish attempts".into())),
            });
        }
        if self
            .publish_locks
            .as_ref()
            .is_some_and(|locks| locks.contains(&uuid))
        {
            if let Some(draft) = self.publish_options.get_mut(&uuid) {
                draft.error = Some(
                    "Unresolved publish attempt must be reconciled before editing options".into(),
                );
            }
            self.pending_publish = false;
            self.pending_navigation = None;
            return Task::done(Message::OptionsSaved {
                uuid,
                result: Err(
                    "Unresolved publish attempt must be reconciled before editing options".into(),
                ),
            });
        }
        self.busy = true;
        let application = self.application.clone();
        let save_uuid = uuid.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let data_dir = application.store().paths().data_dir.clone();
                    let _lock = crate::publish_journal::publisher_lock(&data_dir)?;
                    let journal = crate::publish_journal::PublishJournal::new(&data_dir)?;
                    if journal
                        .blocked_attempts_locked(application.store())?
                        .iter()
                        .any(|attempt| attempt.uuid == save_uuid)
                    {
                        return Err(crate::error::Error::ReconciliationRequired(
                            "unresolved publish attempt blocks option edits".into(),
                        ));
                    }
                    application.mutate(Some(&save_uuid), |store| {
                        crate::workflow::save_options(store, &save_uuid, options)
                    })
                })
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())
            },
            move |result| Message::OptionsSaved { uuid, result },
        )
    }

    fn publish_selected_task(&mut self) -> Task<Message> {
        let cells = self
            .publications
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|row| {
                if !matches!(phase(row), Ok(Phase::Described | Phase::Appeared)) {
                    return None;
                }
                let uuid = row.get("uuid").and_then(Value::as_str)?;
                let pending = row
                    .get("apparitions")
                    .and_then(Value::as_array)?
                    .iter()
                    .any(|apparition| {
                        apparition.get("platformName").and_then(Value::as_str) == Some("deviantart")
                            && apparition.get("state").and_then(Value::as_str)
                                == Some("unpublished")
                    });
                pending.then(|| crate::publishing::PublishCell {
                    uuid: uuid.to_owned(),
                    platform: "deviantart".into(),
                })
            })
            .collect::<Vec<_>>();
        if cells.is_empty() {
            self.publish_result = Some("No pending registered deliveries selected".into());
            return Task::none();
        }
        self.publish_cancel
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.publish_active = true;
        self.last_publish_cells = cells.iter().cloned().collect();
        self.publish_outcomes.clear();
        self.publish_diagnostic = None;
        let application = self.application.clone();
        let cancel = self.publish_cancel.clone();
        Task::perform(
            async move {
                application
                    .publish_selected_with_diagnostics(cells, cancel)
                    .await
            },
            Message::PublishFinished,
        )
    }

    fn autosave_due(&mut self) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let now = self.started.elapsed().as_millis() as u64;
        if let Some((uuid, sent)) = self
            .drafts
            .iter_mut()
            .find_map(|(uuid, draft)| draft.due(now).then(|| (uuid.clone(), draft.begin_save())))
        {
            return self.save_task(uuid, sent);
        }
        Task::none()
    }

    fn save_next_dirty(&mut self) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        if let Some((uuid, sent)) = self.drafts.iter_mut().find_map(|(uuid, draft)| {
            (draft.is_dirty() && !draft.is_saving()).then(|| (uuid.clone(), draft.begin_save()))
        }) {
            return self.save_task(uuid, sent);
        }
        if let Some(tab) = self.pending_navigation.take() {
            self.active_tab = tab;
        }
        Task::none()
    }

    fn save_task(&mut self, uuid: String, sent: SentDraft) -> Task<Message> {
        self.busy = true;
        let application = self.application.clone();
        let patch = MetadataPatch {
            title: sent.title.as_ref().map(|field| field.text.clone()),
            description: sent.description.as_ref().map(|field| field.text.clone()),
        };
        let save_uuid = uuid.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    application
                        .mutate(Some(&save_uuid), |store| {
                            workflow::save_metadata(store, &save_uuid, &patch)
                        })
                        .map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(format!("Could not save metadata: {error}")))
            },
            move |result| Message::SaveFinished { uuid, sent, result },
        )
    }

    fn save_uuid(&mut self, uuid: &str) -> Task<Message> {
        if self.busy_uuids.contains(uuid) {
            return Task::none();
        }
        let Some(draft) = self.drafts.get_mut(uuid) else {
            return Task::none();
        };
        if !draft.is_dirty() {
            return Task::none();
        }
        let sent = draft.begin_save();
        self.save_task(uuid.to_owned(), sent)
    }

    fn approve_metadata(&mut self, uuid: String) -> Task<Message> {
        if self.busy_uuids.contains(&uuid) {
            return Task::none();
        }
        if self.drafts.get(&uuid).is_some_and(Draft::is_dirty) {
            self.pending_approval = Some(uuid.clone());
            return self.save_uuid(&uuid);
        }
        let application = self.application.clone();
        self.busy = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    application.mutate(Some(&uuid), |store| {
                        workflow::approve_metadata(store, &uuid)
                    })
                })
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())
            },
            |result| Message::WorkflowFinished {
                message: "Approved metadata".into(),
                result,
            },
        )
    }

    fn return_to_schedule(&mut self, uuid: String) -> Task<Message> {
        if self.busy_uuids.contains(&uuid) {
            return Task::none();
        }
        if self.drafts.get(&uuid).is_some_and(Draft::is_dirty) {
            self.pending_return = Some(uuid.clone());
            return self.save_uuid(&uuid);
        }
        let application = self.application.clone();
        self.busy = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    application.mutate(Some(&uuid), |store| {
                        workflow::return_to_scheduling(store, &uuid)
                    })
                })
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())
            },
            |result| Message::WorkflowFinished {
                message: "Returned to scheduling".into(),
                result,
            },
        )
    }

    fn confirm_generation(
        &mut self,
        selection: Option<String>,
        batch_selection: Option<Vec<String>>,
    ) -> Task<Message> {
        if let Some(uuid) = selection.as_deref() {
            if self.drafts.get(uuid).is_some_and(Draft::is_dirty) {
                self.pending_generation = Some(uuid.to_owned());
                self.pending_generation_install = true;
                self.pending_generation_batch = batch_selection;
                return self.save_uuid(uuid);
            }
        } else if self.any_dirty() {
            self.pending_generation = Some(String::new());
            self.pending_generation_install = true;
            self.pending_generation_batch = batch_selection;
            return self.save_next_dirty();
        }
        self.start_generation(selection, true, batch_selection)
    }

    fn request_generation(&mut self, uuid: String) -> Task<Message> {
        if self.pending_local_consent.is_some() || self.busy_uuids.contains(&uuid) {
            return Task::none();
        }
        if self.source == Source::Local {
            return self.local_fit_task(Some(uuid), None, true);
        }
        if self.drafts.get(&uuid).is_some_and(Draft::is_dirty) {
            self.pending_generation_install = false;
            self.pending_generation = Some(uuid.clone());
            return self.save_uuid(&uuid);
        }
        self.start_generation(Some(uuid), false, None)
    }

    fn request_batch_generation(&mut self) -> Task<Message> {
        if self.pending_local_consent.is_some() {
            return Task::none();
        }
        let ids = self.marked_generation_ids();
        if ids
            .iter()
            .any(|id| self.drafts.get(id).is_some_and(Draft::is_dirty))
        {
            self.pending_generation_install = false;
            self.pending_generation = Some(String::new());
            return self.save_next_dirty();
        }
        if ids.is_empty() {
            return Task::none();
        }
        if self.source == Source::Local {
            return self.local_fit_task(None, Some(ids), true);
        }
        self.start_generation(None, false, None)
    }

    fn marked_generation_ids(&self) -> Vec<String> {
        self.publications
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|row| {
                phase(row).ok() == Some(Phase::Scheduled)
                    && ["title", "description"].iter().any(|key| {
                        row.get(*key).and_then(Value::as_str) == Some(crate::model::GENERATE_MARKER)
                    })
            })
            .filter_map(|row| row.get("uuid").and_then(Value::as_str).map(str::to_owned))
            .collect()
    }

    fn local_fit_task(
        &mut self,
        selection: Option<String>,
        batch_selection: Option<Vec<String>>,
        generate: bool,
    ) -> Task<Message> {
        self.local_status_request = self.local_status_request.wrapping_add(1);
        let request_id = self.local_status_request;
        let application = self.application.clone();
        let model = self.model.clone();
        let message_model = model.clone();
        let timeout = self._options.ai_timeout;
        Task::perform(
            async move {
                application
                    .local_status(&model, timeout)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |result| Message::LocalFitFinished {
                request_id,
                model: message_model,
                selection,
                batch_selection,
                generate,
                result,
            },
        )
    }

    fn continue_local_generation(
        &mut self,
        selection: Option<String>,
        batch_selection: Option<Vec<String>>,
    ) -> Task<Message> {
        if let Some(uuid) = selection.as_deref() {
            if self.drafts.get(uuid).is_some_and(Draft::is_dirty) {
                self.pending_generation = Some(uuid.to_owned());
                self.pending_generation_install = false;
                self.pending_generation_batch = batch_selection;
                return self.save_uuid(uuid);
            }
        } else if self.any_dirty() {
            self.pending_generation = Some(String::new());
            self.pending_generation_install = false;
            self.pending_generation_batch = batch_selection;
            return self.save_next_dirty();
        }
        self.start_generation(selection, false, batch_selection)
    }

    fn start_generation(
        &mut self,
        selection: Option<String>,
        install_missing: bool,
        batch_selection: Option<Vec<String>>,
    ) -> Task<Message> {
        let request = GenerationRequest {
            source: self.source,
            model: self.model.clone(),
            timeout: self._options.ai_timeout,
            install_missing,
        };
        let application = self.application.clone();
        if let Some(uuid) = &selection {
            self.busy_uuids.insert(uuid.clone());
        } else {
            self.busy_uuids.extend(
                self.publications
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|row| {
                        phase(row).ok() == Some(Phase::Scheduled)
                            && ["title", "description"].iter().any(|key| {
                                row.get(*key).and_then(Value::as_str)
                                    == Some(crate::model::GENERATE_MARKER)
                            })
                    })
                    .filter_map(|row| row.get("uuid").and_then(Value::as_str).map(str::to_owned)),
            );
        }
        self.busy = true;
        let completion_uuid = selection.clone();
        Task::perform(
            async move {
                if let Some(uuid) = selection {
                    application
                        .generate_one(&uuid, request)
                        .await
                        .map(|_| format!("Generated metadata for {uuid}"))
                } else {
                    let generation = if let Some(expected) = batch_selection.as_deref() {
                        application
                            .generate_marked_expected(&request, expected)
                            .await
                    } else {
                        application.generate_marked(&request).await
                    };
                    generation.map(|report| {
                        format!("Generated {}; {} failed", report.succeeded, report.failed)
                    })
                }
                .map_err(|error| error.to_string())
            },
            move |result| Message::GenerationFinished {
                uuid: completion_uuid,
                result,
            },
        )
    }

    fn any_dirty(&self) -> bool {
        self.drafts.values().any(Draft::is_dirty)
    }

    fn publication_exists(&self, uuid: &str) -> bool {
        self.publication(uuid).is_some()
    }

    fn publication(&self, uuid: &str) -> Option<&Value> {
        self.publications
            .as_ref()?
            .iter()
            .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick),
            iced::window::close_requests().map(|_| Message::CloseRequested),
        ])
    }

    fn view(&self) -> Element<'_, Message> {
        if let Some(error) = &self.load_error {
            return container(
                column![text("Unable to load publications"), text(error)].spacing(12),
            )
            .padding(24)
            .into();
        }
        let Some(rows) = &self.publications else {
            return container(text("Loading publications…")).padding(24).into();
        };
        if !self.search_authorized {
            return container(text("Loading publications…")).padding(24).into();
        }
        let tabs = row(Tab::ALL.into_iter().map(|(tab, label)| {
            let control = button(label);
            if tab == self.active_tab {
                control.into()
            } else {
                control.on_press(Message::Navigate(tab)).into()
            }
        }))
        .spacing(8);
        let mut content = column![
            tabs,
            text(tab_name(self.active_tab)),
            text(format!("{} publications", rows.len()))
        ]
        .spacing(16);
        if self.active_tab == Tab::Pick {
            content = content.push(pick::view(self));
        }
        if self.active_tab == Tab::Schedule {
            content = content.push(schedule::view(self));
        }
        if self.active_tab == Tab::Describe {
            content = content.push(describe::view(self));
        }
        if self.active_tab == Tab::Publish {
            content = content.push(publish::view(self));
        }
        if self.active_tab == Tab::Calendar {
            content = content.push(calendar::view(self));
        }
        if let Some(result) = &self.result {
            content = content.push(text(result));
        }
        if self.pending_close && self.publish_active {
            content = content.push(text("Publishing remains active until its durable outcome and process cleanup are handled."));
            content = content.push(
                row![
                    button("Wait for publishing").on_press(Message::CloseWait),
                    button("Cancel after current delivery").on_press(Message::CloseCancelPublish),
                ]
                .spacing(8),
            );
        }
        content = content.push(
            row![
                button("Refresh").on_press(Message::Refresh),
                button("Retry save").on_press(Message::RetrySave),
                button("Keep editing").on_press(Message::KeepEditing),
                button("Discard drafts and close").on_press(Message::ConfirmDiscard),
            ]
            .spacing(8),
        );
        container(content).padding(24).into()
    }
}

pub(super) fn prompt_tasks(state: &App) -> Task<Message> {
    let paths = state
        .candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .filter(|path| !state.prompt_matches.contains_key(path))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Task::none();
    }
    let failed_paths = paths.clone();
    let archive = state.prompt_archive.clone();
    let archive_error = state.prompt_archive_error.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                resolve_prompt_matches(archive.as_deref(), archive_error.as_deref(), paths)
            })
            .await
            .unwrap_or_else(|error| {
                failed_paths
                    .into_iter()
                    .map(|path| {
                        (
                            path,
                            PromptMatch::Unknown(format!("prompt resolution failed: {error}")),
                        )
                    })
                    .collect()
            })
        },
        Message::PromptMatchesFinished,
    )
}

pub(super) fn resolve_prompt_matches(
    archive: Option<&PromptArchive>,
    archive_error: Option<&str>,
    paths: Vec<PathBuf>,
) -> Vec<(PathBuf, PromptMatch)> {
    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .map(|path| {
            let result = match (archive, archive_error) {
                (Some(archive), _) => archive.resolve_path(&path),
                (None, Some(error)) => return (path, PromptMatch::Unknown(error.to_owned())),
                (None, None) => {
                    return (
                        path,
                        PromptMatch::Unknown("prompt archive is not configured".into()),
                    );
                }
            };
            let prompt = result.unwrap_or_else(|error| {
                PromptMatch::Unknown(format!("prompt resolution failed: {error}"))
            });
            (path, prompt)
        })
        .collect()
}

fn app_theme(_: &App) -> Theme {
    Theme::Dark
}

fn row_metadata(row: &Value) -> (String, String) {
    (
        row.get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        row.get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
    )
}

fn set_option(options: &mut Vec<String>, value: String, selected: bool) {
    if selected && !options.contains(&value) {
        options.push(value);
    } else if !selected {
        options.retain(|option| option != &value);
    }
}

fn tab_name(tab: Tab) -> &'static str {
    Tab::ALL
        .iter()
        .find(|(item, _)| *item == tab)
        .map(|(_, name)| *name)
        .unwrap_or("Pick")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, Paths},
        metadata::{Endpoints, MetadataClient},
        store::Store,
    };

    fn app() -> (App, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(root.path()), None).unwrap();
        let store = Arc::new(Store::new(paths, Config::default()).unwrap());
        let metadata = MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        );
        let application = Arc::new(CoreApplication::new(store, metadata, || 0));
        (
            App {
                application,
                _options: UiOptions {
                    openrouter_model: None,
                    ai_timeout: Duration::from_secs(300),
                },
                active_tab: Tab::Pick,
                publications: Some(Vec::new()),
                load_error: None,
                search_authorized: true,
                drafts: HashMap::new(),
                busy: false,
                result: None,
                pending_navigation: None,
                pending_close: false,
                pending_jump: None,
                revealed_publication: None,
                started: Instant::now(),
                search_limit: 25,
                search_paths: Vec::new(),
                candidates: Vec::new(),
                previews: HashMap::new(),
                describe_previews: HashMap::new(),
                description_editors: HashMap::new(),
                pending_delete: None,
                custom_dates: HashMap::new(),
                source: Source::OpenRouter,
                model: VISION_MODELS[0].clone(),
                busy_uuids: HashSet::new(),
                pending_generation: None,
                pending_generation_install: false,
                pending_generation_batch: None,
                pending_approval: None,
                pending_return: None,
                pending_local_consent: None,
                local_fit: None,
                local_status_request: 0,
                publish_result: None,
                publish_diagnostic: None,
                publish_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                publish_active: false,
                pending_publish: false,
                publish_options: HashMap::new(),
                publish_locks: Some(HashSet::new()),
                publish_lock_error: None,
                publish_attempts: Vec::new(),
                publish_outcomes: HashMap::new(),
                last_publish_cells: HashSet::new(),
                reconcile_urls: HashMap::new(),
                prompt_archive: None,
                prompt_archive_error: None,
                prompt_matches: HashMap::new(),
                calendar_preparing: false,
                calendar_prepared: false,
                calendar_error: None,
                calendar_artwork: HashMap::new(),
                calendar_previews: HashMap::new(),
                url_opener: calendar::system_open_url,
            },
            root,
        )
    }

    #[test]
    fn select_all_is_unlocked_without_unresolved_attempts_and_locked_with_them() {
        let (mut app, _root) = app();
        assert!(!app.selection_locked(None));
        app.publish_locks = Some(HashSet::from(["unresolved".into()]));
        assert!(app.selection_locked(None));
        assert!(app.selection_locked(Some("unresolved")));
        assert!(!app.selection_locked(Some("other")));
    }

    fn dirty_draft(app: &mut App) {
        let mut draft = Draft::new("Old title".into(), "Old description".into());
        draft.edit(Field::Title, "Edited title".into(), 1);
        app.drafts.insert("id".into(), draft);
    }

    fn sent() -> SentDraft {
        SentDraft {
            title: Some(drafts::SentField {
                revision: 1,
                text: "Edited title".into(),
            }),
            description: None,
        }
    }

    #[test]
    fn phase_switch_waits_for_successful_flush() {
        let (mut app, _root) = app();
        dirty_draft(&mut app);
        let _task = app.update(Message::Navigate(Tab::Schedule));
        assert_eq!(app.active_tab, Tab::Pick);
        assert!(app.busy);
        let _task = app.update(Message::SaveFinished {
            uuid: "id".into(),
            sent: sent(),
            result: Ok(()),
        });
        assert_eq!(app.active_tab, Tab::Schedule);
        assert!(!app.any_dirty());
        assert_eq!(
            app.drafts.get("id").unwrap().last_sent(Field::Title),
            Some("Edited title")
        );
    }

    #[test]
    fn failed_save_keeps_current_tab_and_draft_dirty() {
        let (mut app, _root) = app();
        dirty_draft(&mut app);
        let _task = app.update(Message::Navigate(Tab::Schedule));
        let _task = app.update(Message::SaveFinished {
            uuid: "id".into(),
            sent: sent(),
            result: Err("disk full".into()),
        });
        assert_eq!(app.active_tab, Tab::Pick);
        assert!(app.any_dirty());
        assert_eq!(app.pending_navigation, None);
    }

    #[test]
    fn failed_close_save_does_not_exit_or_discard_draft() {
        let (mut app, _root) = app();
        dirty_draft(&mut app);
        let _task = app.update(Message::CloseRequested);
        let _task = app.update(Message::SaveFinished {
            uuid: "id".into(),
            sent: sent(),
            result: Err("disk full".into()),
        });
        assert!(!app.pending_close);
        assert!(app.any_dirty());
        assert!(app.result.as_deref().unwrap().contains("Save failed"));
    }

    #[test]
    fn refresh_is_blocked_while_drafts_are_dirty() {
        let (mut app, _root) = app();
        dirty_draft(&mut app);
        let _task = app.update(Message::Refresh);
        assert!(app.publications.is_some());
        assert!(app.result.as_deref().unwrap().contains("discard"));
    }

    #[test]
    fn malformed_publications_show_error_instead_of_empty_queue() {
        let (mut app, _root) = app();
        let _task = app.update(Message::LoadFinished(Err("malformed JSON".into())));
        assert!(app.publications.is_none());
        assert_eq!(app.load_error.as_deref(), Some("malformed JSON"));
        assert!(!app.search_authorized);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn typed_uncertainty_with_worker_stderr_refreshes_receipt_and_store_through_ui() {
        use iced_runtime::{Action, futures::futures::StreamExt};
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;

        async fn run_task(task: Task<Message>) -> Vec<Message> {
            let Some(mut actions) = iced_runtime::task::into_stream(task) else {
                return Vec::new();
            };
            let mut messages = Vec::new();
            while let Some(action) = actions.next().await {
                if let Action::Output(message) = action {
                    messages.push(message);
                }
            }
            messages
        }

        async fn dispatch(app: &mut App, task: Task<Message>) -> Vec<Message> {
            let mut pending = run_task(task).await;
            let mut messages = Vec::new();
            while !pending.is_empty() {
                let mut next = Vec::new();
                for message in pending {
                    next.extend(run_task(app.update(message.clone())).await);
                    messages.push(message);
                }
                pending = next;
            }
            messages
        }

        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path();
        let tags = data_dir.join("tags.txt");
        std::fs::write(&tags, "fake tag\n").unwrap();
        let mut rows = Vec::new();
        for n in 1..=2 {
            let image = data_dir.join(format!("{n}.png"));
            assert!(
                std::process::Command::new("magick")
                    .args(["-size", "4x3", "xc:white"])
                    .arg(&image)
                    .status()
                    .unwrap()
                    .success()
            );
            rows.push(json!({
                "uuid": format!("00000000-0000-4000-8000-{n:012}"),
                "state":"DESCRIBED", "title":"Title", "description":"Description",
                "submissionTimestamp":1_700_002_800,
                "apparitionTimestampIfDifferentThanSubmission":1_700_002_800,
                "files":[{"basename":format!("{n}.png"),"sha512sum":crate::images::file_sha512(&image).unwrap()}],
                "apparitions":[{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]}]
            }));
        }
        let paths = Paths::resolve(Some(data_dir), None).unwrap();
        let store = Arc::new(
            Store::new(
                paths,
                Config {
                    tags: Some("tags.txt".into()),
                    ..Config::default()
                },
            )
            .unwrap(),
        );
        std::fs::write(&store.paths().json, serde_json::to_vec(&rows).unwrap()).unwrap();
        let calls = data_dir.join("calls.log");
        let worker = data_dir.join("worker.sh");
        std::fs::write(&worker, format!(r#"#!/bin/sh
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
      printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid"
      ;;
    close) exit 0 ;;
  esac
done
"#, calls.display())).unwrap();
        let mut permissions = std::fs::metadata(&worker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&worker, permissions).unwrap();
        let application = Arc::new(
            CoreApplication::new(
                store.clone(),
                MetadataClient::new(
                    reqwest::Client::new(),
                    Endpoints::default(),
                    "llmfit".into(),
                ),
                || 0,
            )
            .with_publisher(crate::publishing::Publisher::new(
                worker,
                data_dir.join("skill.md"),
            )),
        );
        let (mut app, init) = App::from_application(
            application,
            UiOptions {
                openrouter_model: None,
                ai_timeout: Duration::from_secs(1),
            },
        );
        dispatch(&mut app, init).await;
        crate::store::fail_next_commit_for_test(crate::store::CommitFailureForTest::AfterRename);
        let task = app.update(Message::PublishSelected);
        let messages = dispatch(&mut app, task).await;
        let completion = messages
            .iter()
            .find_map(|message| match message {
                Message::PublishFinished(completion) => Some(completion),
                _ => None,
            })
            .expect("Publisher result reaches native App consumer");
        assert!(
            completion
                .result
                .as_ref()
                .unwrap_err()
                .contains("may be visible but is not confirmed durable")
        );
        assert_eq!(
            completion.diagnostics.as_deref(),
            Some("simulated worker stderr")
        );
        assert_eq!(std::fs::read_to_string(calls).unwrap().lines().count(), 1);
        assert_eq!(
            store.read().unwrap()[0]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
        assert_eq!(
            store.read().unwrap()[1]["apparitions"][0]["state"],
            "unpublished"
        );
        assert!(matches!(
            app.publish_attempts()[0].state,
            AttemptState::ScheduledUnapplied { .. }
        ));
        assert_eq!(
            app.publish_cell_outcome("00000000-0000-4000-8000-000000000001")
                .unwrap()
                .state,
            PublishCellState::Unknown
        );
        let receipt: Value = serde_json::from_slice(
            &std::fs::read(data_dir.join(format!(
                ".publicator/publish/{}/receipt.json",
                app.publish_attempts()[0].attempt_id
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["terminal"]["type"], "scheduled");
        assert_eq!(receipt["applied"], false);
        assert!(
            app.publish_result
                .as_deref()
                .unwrap()
                .contains("may be visible but is not confirmed durable")
        );
        assert_eq!(
            app.publish_diagnostic.as_deref(),
            Some("simulated worker stderr")
        );
    }
}
