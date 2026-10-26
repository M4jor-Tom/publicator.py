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
    prompts::{PromptArchive, PromptMatch, SearchMatches},
    publish_journal::{AttemptState, AttemptStatus, ReconcileOutcome},
    publishing::{PublishCell, PublishCellOutcome, PublishCellState, PublishCompletion},
    workflow::{self, MetadataPatch},
};
use iced::{
    Element, Subscription, Task, Theme,
    keyboard::{Key, key::Named},
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

fn publish_keyboard_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: Key::Named(key),
        modifiers,
        repeat: false,
        ..
    }) = event
    else {
        return None;
    };
    if status == iced::event::Status::Captured {
        return Some(Message::PublishKeyboardExit);
    }
    if key == Named::Tab && modifiers == iced::keyboard::Modifiers::SHIFT {
        return Some(Message::PublishKeyboardNext { reverse: true });
    }
    if !modifiers.is_empty() {
        return None;
    }
    match key {
        Named::F6 => Some(Message::PublishKeyboardToggle),
        Named::Tab => Some(Message::PublishKeyboardNext { reverse: false }),
        Named::Space => Some(Message::PublishKeyboardToggleCell),
        Named::Escape => Some(Message::PublishKeyboardExit),
        _ => None,
    }
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
    PromptQueryChanged(String),
    PromptLineageChanged(String),
    SearchFinished {
        request_id: u64,
        result: Result<Vec<Candidate>, String>,
    },
    PromptSearchFinished {
        request_id: u64,
        result: Result<SearchMatches, String>,
    },
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
    PublishKeyboardToggle,
    PublishKeyboardNext {
        reverse: bool,
    },
    PublishKeyboardToggleCell,
    PublishKeyboardExit,
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
    search_request_id: u64,
    search_limit: usize,
    search_paths: Vec<String>,
    prompt_query: String,
    prompt_lineage: String,
    prompt_search: Option<SearchMatches>,
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
    publish_keyboard_mode: bool,
    publish_keyboard_focus: Option<String>,
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
                    match PromptArchive::from_config(&application.store().paths().data_dir, config)
                    {
                        Ok(Some(archive)) => (Some(Arc::new(archive)), None),
                        Ok(None) => (None, None),
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
            search_request_id: 0,
            search_limit: 25,
            search_paths: application.store().config().publicable.clone(),
            prompt_query: String::new(),
            prompt_lineage: String::new(),
            prompt_search: None,
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
            publish_keyboard_mode: false,
            publish_keyboard_focus: None,
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
            let detail = blocked_attempt_detail(attempt);
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
        } else {
            self.pending_cell_outcome(cell)
        };
        Some(outcome)
    }

    fn pending_cell_outcome(&self, cell: PublishCell) -> PublishCellOutcome {
        let uuid = cell.uuid.as_str();
        if let Some(attempt) = self.publish_attempts.iter().find(|attempt| {
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
        } else {
            PublishCellOutcome {
                cell,
                state: PublishCellState::Unattempted,
                detail: None,
            }
        }
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

    fn next_search_request(&mut self) -> u64 {
        self.search_request_id = self.search_request_id.wrapping_add(1);
        self.search_request_id
    }

    fn reload_task(&mut self) -> Task<Message> {
        self.next_search_request();
        Self::load_task(self.application.clone())
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
            Message::LoadFinished(..)
            | Message::PublishLocksFinished(..)
            | Message::PublishStatusesFinished(..) => self.update_load(message),
            Message::Navigate(..)
            | Message::CloseRequested
            | Message::ConfirmDiscard
            | Message::KeepEditing
            | Message::Refresh
            | Message::RetrySave
            | Message::Tick
            | Message::JumpToPublication(..) => self.update_navigation(message),
            Message::PublishKeyboardToggle
            | Message::PublishKeyboardNext { .. }
            | Message::PublishKeyboardToggleCell
            | Message::PublishKeyboardExit => self.update_keyboard(message),
            Message::SelectSource(..)
            | Message::SelectModel(..)
            | Message::ConfirmLocalInstall
            | Message::CancelLocalInstall
            | Message::LocalFitFinished { .. }
            | Message::GenerationFinished { .. } => self.update_generation(message),
            Message::SaveDraft(..)
            | Message::ApproveMetadata(..)
            | Message::ReturnToSchedule(..)
            | Message::GenerateOne(..)
            | Message::GenerateMarked
            | Message::DescriptionAction { .. }
            | Message::Edited { .. }
            | Message::SaveFinished { .. } => self.update_metadata(message),
            Message::SelectCell { .. }
            | Message::SelectAll(..)
            | Message::PublishSelected
            | Message::CancelAfterCurrent
            | Message::CloseCancelPublish
            | Message::CloseWait
            | Message::PublishFinished(..) => self.update_selection(message),
            message => self.update_workflow(message),
        }
    }

    fn update_workflow(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OptionPriceChanged { .. }
            | Message::OptionTierChanged { .. }
            | Message::OptionGalleryChanged { .. }
            | Message::SaveOptions(..)
            | Message::OptionsSaved { .. } => self.update_options(message),
            Message::ReconcileUrlChanged { .. }
            | Message::ReconcileAttempt { .. }
            | Message::ReconcileFinished { .. }
            | Message::OpenActualUrl(..)
            | Message::UrlOpened { .. } => self.update_reconciliation(message),
            Message::SearchLimit(..)
            | Message::SearchPath(..)
            | Message::PromptQueryChanged(..)
            | Message::PromptLineageChanged(..)
            | Message::Search => self.update_search(message),
            Message::SearchFinished { .. }
            | Message::PromptSearchFinished { .. }
            | Message::PromptMatchesFinished(..) => self.update_search_results(message),
            Message::PreviewFinished { .. }
            | Message::PreviewViewport { .. }
            | Message::DescribeViewport { .. }
            | Message::DescribePreviewFinished { .. }
            | Message::CalendarPrepared(..)
            | Message::CalendarPreviewFinished { .. }
            | Message::CalendarViewport { .. } => self.update_previews(message),
            Message::Pick(..)
            | Message::PickFinished(..)
            | Message::AskDelete(..)
            | Message::ConfirmDelete
            | Message::CancelDelete
            | Message::WorkflowFinished { .. } => self.update_pick(message),
            Message::CustomDate { .. }
            | Message::CustomTime { .. }
            | Message::SaveCustomDate(..)
            | Message::PresetDate { .. }
            | Message::SuggestedDate(..)
            | Message::ApproveDate(..)
            | Message::ReturnToPick(..) => self.update_schedule(message),
            Message::LoadFinished(..)
            | Message::PublishLocksFinished(..)
            | Message::PublishStatusesFinished(..)
            | Message::Navigate(..)
            | Message::CloseRequested
            | Message::ConfirmDiscard
            | Message::KeepEditing
            | Message::Refresh
            | Message::RetrySave
            | Message::Tick
            | Message::JumpToPublication(..)
            | Message::PublishKeyboardToggle
            | Message::PublishKeyboardNext { .. }
            | Message::PublishKeyboardToggleCell
            | Message::PublishKeyboardExit
            | Message::SelectSource(..)
            | Message::SelectModel(..)
            | Message::ConfirmLocalInstall
            | Message::CancelLocalInstall
            | Message::LocalFitFinished { .. }
            | Message::GenerationFinished { .. }
            | Message::SaveDraft(..)
            | Message::ApproveMetadata(..)
            | Message::ReturnToSchedule(..)
            | Message::GenerateOne(..)
            | Message::GenerateMarked
            | Message::DescriptionAction { .. }
            | Message::Edited { .. }
            | Message::SaveFinished { .. }
            | Message::SelectCell { .. }
            | Message::SelectAll(..)
            | Message::PublishSelected
            | Message::CancelAfterCurrent
            | Message::CloseCancelPublish
            | Message::CloseWait
            | Message::PublishFinished(..) => unreachable!("message handled by primary dispatcher"),
        }
    }

    fn update_load(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoadFinished(Ok(rows)) => return self.on_load_finished_loaded(rows),
            Message::LoadFinished(Err(error)) => {
                self.next_search_request();
                self.publications = None;
                self.load_error = Some(error);
                self.search_authorized = false;
            }
            Message::PublishLocksFinished(Ok(uuids)) => {
                self.publish_locks = Some(uuids);
                self.publish_lock_error = None;
                self.refresh_keyboard_publish_focus();
            }
            Message::PublishLocksFinished(Err(error)) => {
                self.publish_locks = None;
                self.publish_lock_error = Some(error);
                self.refresh_keyboard_publish_focus();
            }
            Message::PublishStatusesFinished(Ok(attempts)) => {
                return self.on_publish_statuses_finished_ready(attempts);
            }
            Message::PublishStatusesFinished(Err(error)) => {
                return self.on_publish_statuses_finished_failed(error);
            }
            _ => unreachable!("message routed to load handler"),
        }
        Task::none()
    }

    fn refresh(&mut self) -> Task<Message> {
        if self.any_dirty() || self.publish_options.values().any(|options| options.dirty) {
            self.result = Some("Save or explicitly discard drafts before refresh.".into());
        } else {
            return self.reload_task();
        }
        Task::none()
    }

    fn update_navigation(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(tab) => {
                self.publish_keyboard_mode = false;
                self.publish_keyboard_focus = None;
                self.revealed_publication = None;
                self.pending_jump = None;
                return self.navigate(tab);
            }
            Message::JumpToPublication(uuid) => return self.jump_to_publication(uuid),
            Message::CloseRequested => return self.on_close_requested(),
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
            Message::Refresh => return self.refresh(),
            Message::RetrySave => return self.save_next_dirty(),
            Message::Tick => return self.autosave_due(),
            _ => unreachable!("message routed to navigation handler"),
        }
        Task::none()
    }

    fn update_keyboard(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::PublishKeyboardToggle => return self.on_publish_keyboard_toggle(),
            Message::PublishKeyboardNext { reverse } => {
                return self.on_publish_keyboard_next(reverse);
            }
            Message::PublishKeyboardToggleCell => return self.on_publish_keyboard_toggle_cell(),
            Message::PublishKeyboardExit => {
                self.publish_keyboard_mode = false;
                self.publish_keyboard_focus = None;
            }
            _ => unreachable!("message routed to keyboard handler"),
        }
        Task::none()
    }

    fn select_source(&mut self, source: Source) -> Task<Message> {
        if self.pending_local_consent.is_some() || self.pending_generation.is_some() {
            return Task::none();
        }
        self.local_status_request = self.local_status_request.wrapping_add(1);
        self.source = source;
        self.local_fit = None;
        if source == Source::Local {
            return self.local_fit_task(None, None, false);
        }
        Task::none()
    }

    fn select_model(&mut self, model: ModelOption) -> Task<Message> {
        if self.pending_local_consent.is_some()
            || self.pending_generation.is_some()
            || !VISION_MODELS.contains(&model)
        {
            return Task::none();
        }
        self.local_status_request = self.local_status_request.wrapping_add(1);
        self.model = model;
        self.local_fit = None;
        if self.source == Source::Local {
            return self.local_fit_task(None, None, false);
        }
        Task::none()
    }

    fn update_generation(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectSource(source) => return self.select_source(source),
            Message::SelectModel(model) => return self.select_model(model),
            Message::ConfirmLocalInstall => return self.on_confirm_local_install(),
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
                return self.on_local_fit_finished(
                    request_id,
                    model,
                    selection,
                    batch_selection,
                    generate,
                    result,
                );
            }
            Message::GenerationFinished { uuid, result } => {
                return self.on_generation_finished(uuid, result);
            }
            _ => unreachable!("message routed to generation handler"),
        }
        Task::none()
    }

    fn update_metadata(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SaveDraft(uuid) => self.save_uuid(&uuid),
            Message::ApproveMetadata(uuid) => self.approve_metadata(uuid),
            Message::ReturnToSchedule(uuid) => self.return_to_schedule(uuid),
            Message::GenerateOne(uuid) => self.request_generation(uuid),
            Message::GenerateMarked => self.request_batch_generation(),
            Message::DescriptionAction { uuid, action } => self.on_description_action(uuid, action),
            Message::Edited { uuid, field, text } => self.on_edited(uuid, field, text),
            Message::SaveFinished { uuid, sent, result } => {
                self.on_save_finished(uuid, sent, result)
            }
            _ => unreachable!("message routed to metadata handler"),
        }
    }

    fn update_selection(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectCell { key, selected } => return self.on_select_cell(key, selected),
            Message::SelectAll(selected) => {
                if !selected && self.selection_locked(None) {
                    self.publish_result = Some(
                        "Reconcile unresolved attempts before changing platform selections".into(),
                    );
                    return Task::none();
                }
                return Self::select_publish_task(self.application.clone(), None, None, selected);
            }
            Message::PublishSelected => return self.on_publish_selected(),
            Message::CancelAfterCurrent | Message::CloseCancelPublish => self
                .publish_cancel
                .store(true, std::sync::atomic::Ordering::SeqCst),
            Message::CloseWait => {}
            Message::PublishFinished(completion) => return self.on_publish_finished(completion),
            _ => unreachable!("message routed to selection handler"),
        }
        Task::none()
    }

    fn update_options(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OptionPriceChanged { uuid, value } => {
                self.on_option_price_changed(uuid, value)
            }
            Message::OptionTierChanged {
                uuid,
                value,
                selected,
            } => self.on_option_tier_changed(uuid, value, selected),
            Message::OptionGalleryChanged {
                uuid,
                value,
                selected,
            } => self.on_option_gallery_changed(uuid, value, selected),
            Message::SaveOptions(uuid) => self.save_options_task(uuid),
            Message::OptionsSaved { uuid, result } => self.on_options_saved(uuid, result),
            _ => unreachable!("message routed to options handler"),
        }
    }

    fn on_reconcile_finished(
        &mut self,
        attempt_id: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        self.busy = false;
        self.publish_locks = None;
        self.publish_attempts.clear();
        self.publish_result = Some(match result {
            Ok(()) => format!("Reconciled publish attempt {attempt_id}"),
            Err(error) => format!("Reconciliation refused; receipt retained: {error}"),
        });
        self.reload_task()
    }

    fn update_reconciliation(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ReconcileUrlChanged { attempt_id, value } => {
                self.publish_keyboard_mode = false;
                self.publish_keyboard_focus = None;
                self.reconcile_urls.insert(attempt_id, value);
            }
            Message::ReconcileAttempt {
                attempt_id,
                outcome,
            } => return self.on_reconcile_attempt(attempt_id, outcome),
            Message::ReconcileFinished { attempt_id, result } => {
                return self.on_reconcile_finished(attempt_id, result);
            }
            Message::OpenActualUrl(url) => return self.on_open_actual_url(url),
            Message::UrlOpened { url, result } => {
                self.result = Some(match result {
                    Ok(()) => format!("Opened actual URL: {url}"),
                    Err(error) => format!("Could not open actual URL: {error}"),
                });
            }
            _ => unreachable!("message routed to reconciliation handler"),
        }
        Task::none()
    }

    fn search_path(&mut self, path: String) -> Task<Message> {
        if self.search_paths.contains(&path) {
            self.next_search_request();
            self.search_paths.retain(|p| p != &path);
        } else if self.application.store().config().publicable.contains(&path) {
            self.next_search_request();
            self.search_paths.push(path);
        }
        Task::none()
    }

    fn search_limit_changed(&mut self, limit: usize) {
        if [0, 25, 50, 75, 100].contains(&limit) && self.search_limit != limit {
            self.next_search_request();
            self.search_limit = limit;
        }
    }

    fn update_search(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SearchLimit(limit) => {
                self.search_limit_changed(limit);
            }
            Message::SearchPath(path) => return self.search_path(path),
            Message::PromptQueryChanged(query) => {
                if self.prompt_query != query {
                    self.next_search_request();
                    self.prompt_query = query;
                }
            }
            Message::PromptLineageChanged(lineage) => {
                if self.prompt_lineage != lineage {
                    self.next_search_request();
                    self.prompt_lineage = lineage;
                }
            }
            Message::Search => return self.on_search(),
            _ => unreachable!("message routed to search handler"),
        }
        Task::none()
    }

    fn update_search_results(&mut self, message: Message) -> Task<Message> {
        let request_id = match &message {
            Message::SearchFinished { request_id, .. }
            | Message::PromptSearchFinished { request_id, .. } => Some(*request_id),
            _ => None,
        };
        if request_id.is_some_and(|request_id| !self.search_result_current(request_id)) {
            return Task::none();
        }
        match message {
            Message::SearchFinished { result, .. } => self.search_finished(result),
            Message::PromptSearchFinished { result, .. } => self.prompt_search_finished(result),
            Message::PromptMatchesFinished(matches) => {
                self.prompt_matches.extend(matches);
                Task::none()
            }
            _ => unreachable!("message routed to search results handler"),
        }
    }

    fn search_finished(&mut self, result: Result<Vec<Candidate>, String>) -> Task<Message> {
        match result {
            Ok(candidates) => self.on_search_finished_ready(candidates),
            Err(error) => {
                self.result = Some(format!("Search failed: {error}"));
                Task::none()
            }
        }
    }

    fn prompt_search_finished(&mut self, result: Result<SearchMatches, String>) -> Task<Message> {
        match result {
            Ok(matches) => self.on_prompt_search_finished_ready(matches),
            Err(error) => {
                self.result = Some(format!("Prompt search failed: {error}"));
                Task::none()
            }
        }
    }

    fn search_result_current(&self, request_id: u64) -> bool {
        request_id == self.search_request_id && self.search_authorized
    }

    fn update_previews(&mut self, message: Message) -> Task<Message> {
        match message {
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
            _ => unreachable!("message routed to previews handler"),
        }
        Task::none()
    }

    fn on_workflow_finished(
        &mut self,
        message: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        self.busy = false;
        match result {
            Ok(()) => {
                self.result = Some(message);
                return self.reload_task();
            }
            Err(error) => self.result = Some(format!("{message}: {error}")),
        }
        Task::none()
    }

    fn update_pick(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Pick(candidate) => return pick::pick_task(self, candidate),
            Message::PickFinished(Ok(())) => {
                self.busy = false;
                self.active_tab = Tab::Schedule;
                self.result = Some("Picked candidate".into());
                return self.reload_task();
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
                return self.on_workflow_finished(message, result);
            }
            _ => unreachable!("message routed to pick handler"),
        }
        Task::none()
    }

    fn update_schedule(&mut self, message: Message) -> Task<Message> {
        match message {
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
            _ => unreachable!("message routed to schedule handler"),
        }
        Task::none()
    }

    fn on_load_finished_loaded(&mut self, rows: Vec<Value>) -> Task<Message> {
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
                .or_insert_with(|| iced::widget::text_editor::Content::with_text(description));
            if !self
                .drafts
                .get(uuid)
                .is_some_and(|draft| draft.is_field_dirty(Field::Description))
            {
                *editor = iced::widget::text_editor::Content::with_text(description);
            }
        }
        self.publications = Some(rows);
        self.refresh_keyboard_publish_focus();
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
        Task::none()
    }

    fn close_after_status_refresh(&mut self) -> Task<Message> {
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
        Task::none()
    }

    fn on_publish_statuses_finished_ready(
        &mut self,
        attempts: Vec<AttemptStatus>,
    ) -> Task<Message> {
        self.publish_locks = Some(
            attempts
                .iter()
                .filter(|attempt| attempt.blocks_retry())
                .map(|attempt| attempt.uuid.clone())
                .collect(),
        );
        self.publish_lock_error = None;
        self.publish_attempts = attempts;
        self.refresh_keyboard_publish_focus();
        self.close_after_status_refresh()
    }

    fn on_publish_statuses_finished_failed(&mut self, error: String) -> Task<Message> {
        self.publish_locks = None;
        self.publish_lock_error = Some(error);
        self.publish_attempts.clear();
        self.refresh_keyboard_publish_focus();
        self.close_after_status_refresh()
    }

    fn on_publish_keyboard_toggle(&mut self) -> Task<Message> {
        if self.active_tab != Tab::Publish {
            return Task::none();
        }
        if self.publish_keyboard_mode {
            self.publish_keyboard_mode = false;
            self.publish_keyboard_focus = None;
        } else {
            self.publish_keyboard_mode = true;
            self.publish_keyboard_focus = self.keyboard_publish_cells().first().cloned();
        }
        Task::none()
    }

    fn on_publish_keyboard_next(&mut self, reverse: bool) -> Task<Message> {
        if !self.publish_keyboard_mode {
            return Task::none();
        }
        let cells = self.keyboard_publish_cells();
        if cells.is_empty() {
            self.publish_keyboard_focus = None;
            return Task::none();
        }
        let current = self
            .publish_keyboard_focus
            .as_ref()
            .and_then(|uuid| cells.iter().position(|cell| cell == uuid));
        let next = match (current, reverse) {
            (Some(index), false) => (index + 1) % cells.len(),
            (Some(index), true) => (index + cells.len() - 1) % cells.len(),
            (None, false) => 0,
            (None, true) => cells.len() - 1,
        };
        self.publish_keyboard_focus = Some(cells[next].clone());
        Task::none()
    }

    fn on_publish_keyboard_toggle_cell(&mut self) -> Task<Message> {
        if !self.publish_keyboard_mode || self.active_tab != Tab::Publish {
            return Task::none();
        }
        let Some(uuid) = self.publish_keyboard_focus.clone() else {
            return Task::none();
        };
        if self.options_locked(&uuid) {
            self.publish_result = Some(format!(
                "Cannot change {uuid} — DeviantArt while publishing state is locked"
            ));
            return Task::none();
        }
        let Some(publication) = self.publication(&uuid) else {
            self.publish_keyboard_focus = None;
            return Task::none();
        };
        if !matches!(phase(publication), Ok(Phase::Described | Phase::Appeared)) {
            return Task::none();
        }
        let apparition = publication
            .get("apparitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|apparition| {
                apparition.get("platformName").and_then(Value::as_str) == Some("deviantart")
            });
        let selected = match apparition
            .and_then(|apparition| apparition.get("state"))
            .and_then(Value::as_str)
        {
            None => true,
            Some("unpublished") => false,
            Some("published_or_scheduled") => {
                self.publish_result = Some(format!(
                    "{uuid} — DeviantArt is already successful and locked"
                ));
                return Task::none();
            }
            Some(_) => return Task::none(),
        };
        self.update(Message::SelectCell {
            key: format!("{uuid}:deviantart"),
            selected,
        })
    }

    fn on_confirm_local_install(&mut self) -> Task<Message> {
        if let Some(consent) = self.pending_local_consent.take()
            && consent.request_id == self.local_status_request
            && self.source == Source::Local
            && self.model == consent.model
        {
            if let Some(batch_selection) = consent.batch_selection {
                if self.marked_generation_ids() == batch_selection {
                    return self.flush_then_generate(
                        consent.selection,
                        true,
                        Some(batch_selection),
                    );
                }
                self.pending_local_consent = None;
                self.result =
                    Some("Marked batch changed while local install consent was pending".into());
                return Task::none();
            } else {
                return self.flush_then_generate(consent.selection, true, None);
            }
        }
        Task::none()
    }

    fn local_fit_current(&self, request_id: u64, model: &ModelOption) -> bool {
        request_id == self.local_status_request
            && self.source == Source::Local
            && &self.model == model
    }

    fn on_local_fit_finished(
        &mut self,
        request_id: u64,
        model: ModelOption,
        selection: Option<String>,
        batch_selection: Option<Vec<String>>,
        generate: bool,
        result: Result<LocalStatus, String>,
    ) -> Task<Message> {
        if !self.local_fit_current(request_id, &model) {
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
                    return self.flush_then_generate(selection, false, batch_selection);
                }
            }
            Err(error) => {
                self.local_fit = None;
                self.result = Some(format!("Could not assess local model fit: {error}"));
            }
        }
        Task::none()
    }

    fn on_generation_finished(
        &mut self,
        uuid: Option<String>,
        result: Result<String, String>,
    ) -> Task<Message> {
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
        self.reload_task()
    }

    fn on_description_action(
        &mut self,
        uuid: String,
        action: iced::widget::text_editor::Action,
    ) -> Task<Message> {
        if self.busy_uuids.contains(&uuid) {
            return Task::none();
        }
        let Some(editor) = self.description_editors.get_mut(&uuid) else {
            return Task::none();
        };
        editor.perform(action);
        let text = editor.text();
        self.update(Message::Edited {
            uuid,
            field: Field::Description,
            text,
        })
    }

    fn on_edited(&mut self, uuid: String, field: Field, text: String) -> Task<Message> {
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
        Task::none()
    }

    fn continue_after_options(&mut self) -> Task<Message> {
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
        Task::none()
    }

    fn continue_after_drafts(&mut self) -> Task<Message> {
        if (self.pending_navigation.is_some() || self.pending_publish || self.pending_close)
            && self.publish_options.values().any(|options| options.dirty)
        {
            return self.save_next_options();
        }
        self.continue_after_options()
    }

    fn on_save_finished(
        &mut self,
        uuid: String,
        sent: SentDraft,
        result: Result<(), String>,
    ) -> Task<Message> {
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
        self.continue_after_metadata_save()
    }

    fn continue_after_metadata_save(&mut self) -> Task<Message> {
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
            return self.start_generation((!uuid.is_empty()).then_some(uuid), install, batch);
        }
        if let Some(uuid) = self.pending_approval.take() {
            return self.approve_metadata(uuid);
        }
        if let Some(uuid) = self.pending_return.take() {
            return self.return_to_schedule(uuid);
        }
        self.continue_after_drafts()
    }

    fn on_select_cell(&mut self, key: String, selected: bool) -> Task<Message> {
        let Some((uuid, platform)) = key.rsplit_once(':') else {
            return Task::none();
        };
        if platform != "deviantart" {
            return Task::none();
        }
        if !selected && self.selection_locked(Some(uuid)) {
            self.publish_result =
                Some("Reconcile the unresolved attempt before deselecting this delivery".into());
            return Task::none();
        }
        Self::select_publish_task(
            self.application.clone(),
            Some(uuid.to_owned()),
            Some(platform.to_owned()),
            selected,
        )
    }

    fn on_publish_selected(&mut self) -> Task<Message> {
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
            self.publish_result = Some("Reconcile unresolved attempts before retrying".into());
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
        self.publish_selected_task()
    }

    fn on_publish_finished(&mut self, completion: PublishCompletion) -> Task<Message> {
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
        self.reload_task()
    }

    fn on_option_price_changed(&mut self, uuid: String, value: String) -> Task<Message> {
        self.publish_keyboard_mode = false;
        self.publish_keyboard_focus = None;
        if self.options_locked(&uuid) {
            return Task::none();
        }
        self.ensure_publish_options(&uuid);
        if let Some(options) = self.publish_options.get_mut(&uuid) {
            options.price = value;
            options.dirty = true;
            options.error = None;
        }
        Task::none()
    }

    fn on_option_tier_changed(
        &mut self,
        uuid: String,
        value: String,
        selected: bool,
    ) -> Task<Message> {
        self.publish_keyboard_mode = false;
        self.publish_keyboard_focus = None;
        if self.options_locked(&uuid) || !self.application.store().config().tiers.contains(&value) {
            return Task::none();
        }
        self.ensure_publish_options(&uuid);
        if let Some(options) = self.publish_options.get_mut(&uuid) {
            set_option(&mut options.tiers, value, selected);
            options.dirty = true;
            options.error = None;
        }
        Task::none()
    }

    fn on_option_gallery_changed(
        &mut self,
        uuid: String,
        value: String,
        selected: bool,
    ) -> Task<Message> {
        self.publish_keyboard_mode = false;
        self.publish_keyboard_focus = None;
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
        Task::none()
    }

    fn on_options_saved(&mut self, uuid: String, result: Result<(), String>) -> Task<Message> {
        self.busy = false;
        if let Some(options) = self.publish_options.get_mut(&uuid) {
            match result {
                Ok(()) => {
                    self.publish_options.remove(&uuid);
                    self.result = Some(format!("Saved publishing options for {uuid}"));
                }
                Err(error) => {
                    options.error = Some(error.clone());
                    self.result = Some(format!("Publishing options were not saved: {error}"));
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
        self.reload_task()
    }

    fn on_reconcile_attempt(
        &mut self,
        attempt_id: String,
        outcome: ReconcileOutcome,
    ) -> Task<Message> {
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
        Task::perform(
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
        )
    }

    fn on_open_actual_url(&mut self, url: String) -> Task<Message> {
        let opener = self.url_opener;
        let application = self.application.clone();
        let result_url = url.clone();
        Task::perform(
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
        )
    }

    fn on_close_requested(&mut self) -> Task<Message> {
        if self.publish_active {
            self.pending_close = true;
            self.result = Some(
                "Publishing is active. Wait or cancel after the current delivery before closing."
                    .into(),
            );
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
        iced::exit()
    }

    fn on_search(&mut self) -> Task<Message> {
        if self.search_authorized {
            let request_id = self.next_search_request();
            if self.prompt_archive.is_some()
                && (!self.prompt_query.trim().is_empty() || !self.prompt_lineage.trim().is_empty())
            {
                return pick::prompt_search_task(self, request_id);
            }
            return pick::search_task(self, request_id);
        }
        Task::none()
    }

    fn on_search_finished_ready(&mut self, candidates: Vec<Candidate>) -> Task<Message> {
        self.prompt_search = None;
        self.set_candidates(candidates);
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
        Task::batch([pick::preview_tasks(self, 0, 4), prompt_tasks(self)])
    }

    fn on_prompt_search_finished_ready(&mut self, matches: SearchMatches) -> Task<Message> {
        let candidates = matches
            .exact
            .iter()
            .chain(&matches.maybe)
            .cloned()
            .collect();
        let summary = format!(
            "Prompt search: {} exact, {} lineage hints; {} nearest prompts skipped{}",
            matches.exact.len(),
            matches.maybe.len(),
            matches.skipped,
            if matches.truncated {
                "; results capped, narrow the query or lineage"
            } else {
                ""
            }
        );
        self.prompt_search = Some(matches);
        self.set_candidates(candidates);
        self.result = Some(summary);
        Task::batch([pick::preview_tasks(self, 0, 4), prompt_tasks(self)])
    }

    fn set_candidates(&mut self, candidates: Vec<Candidate>) {
        self.candidates = candidates;
        self.pending_delete = None;
        self.previews.clear();
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
        if tab != Tab::Publish {
            self.publish_keyboard_mode = false;
            self.publish_keyboard_focus = None;
        }
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

    fn refresh_keyboard_publish_focus(&mut self) {
        if !self.publish_keyboard_mode {
            return;
        }
        let cells = self.keyboard_publish_cells();
        if !self
            .publish_keyboard_focus
            .as_ref()
            .is_some_and(|focus| cells.contains(focus))
        {
            self.publish_keyboard_focus = cells.first().cloned();
        }
    }

    fn keyboard_publish_cells(&self) -> Vec<String> {
        if self.busy || self.publish_active {
            return Vec::new();
        }
        publish::ordered_rows(
            self.publications.as_deref().unwrap_or_default(),
            self.revealed_publication(),
        )
        .into_iter()
        .filter_map(|publication| {
            let uuid = publication.get("uuid")?.as_str()?;
            if !matches!(phase(publication), Ok(Phase::Described | Phase::Appeared))
                || self.selection_locked(Some(uuid))
            {
                return None;
            }
            let apparition = publication
                .get("apparitions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|apparition| {
                    apparition.get("platformName").and_then(Value::as_str) == Some("deviantart")
                });
            match apparition
                .and_then(|apparition| apparition.get("state"))
                .and_then(Value::as_str)
            {
                None | Some("unpublished") => Some(uuid.to_owned()),
                _ => None,
            }
        })
        .collect()
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
            .unwrap_or_else(Task::none)
    }

    fn mark_options_save_error(&mut self, uuid: &str, error: &str) {
        if let Some(options) = self.publish_options.get_mut(uuid) {
            options.error = Some(error.into());
        }
        self.pending_publish = false;
        self.pending_navigation = None;
    }

    fn options_save_error(&mut self, uuid: String, error: String) -> Task<Message> {
        self.mark_options_save_error(&uuid, &error);
        Task::done(Message::OptionsSaved {
            uuid,
            result: Err(error),
        })
    }

    fn options_lock_error(&self, uuid: &str) -> Option<String> {
        if self.publish_locks.is_none() || self.publish_lock_error.is_some() {
            return Some(
                self.publish_lock_error
                    .clone()
                    .unwrap_or_else(|| "Checking unresolved publish attempts".into()),
            );
        }
        self.publish_locks
            .as_ref()
            .is_some_and(|locks| locks.contains(uuid))
            .then(|| "Unresolved publish attempt must be reconciled before editing options".into())
    }

    fn save_options_task(&mut self, uuid: String) -> Task<Message> {
        if self.busy {
            self.mark_options_save_error(&uuid, "Another operation is in progress");
            return Task::none();
        }
        if self.publish_active {
            return self.options_save_error(uuid, "Publishing options are busy".into());
        }
        self.ensure_publish_options(&uuid);
        let Some(options) = self.publish_options.get(&uuid) else {
            return Task::none();
        };
        if !options.dirty {
            return Task::none();
        }
        let price = match parse_option_price(&options.price) {
            Ok(price) => Some(price),
            Err(error) => return self.options_save_error(uuid, error),
        };
        let options = crate::workflow::PlatformOptions {
            price,
            tiers: Some(options.tiers.clone()),
            galleries: Some(options.galleries.clone()),
        };
        if let Some(error) = self.options_lock_error(&uuid) {
            return self.options_save_error(uuid, error);
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
        schedule::return_task(self, uuid)
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
        if ids.is_empty() {
            return Task::none();
        }
        if self.source == Source::Local {
            return self.local_fit_task(None, Some(ids), true);
        }
        if ids
            .iter()
            .any(|id| self.drafts.get(id).is_some_and(Draft::is_dirty))
        {
            self.pending_generation_install = false;
            self.pending_generation = Some(String::new());
            return self.save_next_dirty();
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

    fn flush_then_generate(
        &mut self,
        selection: Option<String>,
        install_missing: bool,
        batch_selection: Option<Vec<String>>,
    ) -> Task<Message> {
        if let Some(uuid) = selection.as_deref() {
            if self.drafts.get(uuid).is_some_and(Draft::is_dirty) {
                self.pending_generation = Some(uuid.to_owned());
                self.pending_generation_install = install_missing;
                self.pending_generation_batch = batch_selection;
                return self.save_uuid(uuid);
            }
        } else if self.any_dirty() {
            self.pending_generation = Some(String::new());
            self.pending_generation_install = install_missing;
            self.pending_generation_batch = batch_selection;
            return self.save_next_dirty();
        }
        self.start_generation(selection, install_missing, batch_selection)
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
            self.busy_uuids.extend(self.marked_generation_ids());
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
            iced::event::listen_with(publish_keyboard_event),
        ])
    }

    fn loaded_view(&self, rows: &[Value]) -> Element<'_, Message> {
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
        content = content.push(match self.active_tab {
            Tab::Pick => pick::view(self),
            Tab::Schedule => schedule::view(self),
            Tab::Describe => describe::view(self),
            Tab::Publish => publish::view(self),
            Tab::Calendar => calendar::view(self),
        });
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
        self.loaded_view(rows)
    }
}

pub(super) fn prompt_tasks(state: &App) -> Task<Message> {
    if state.prompt_archive.is_none() && state.prompt_archive_error.is_none() {
        return Task::none();
    }
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

fn parse_option_price(price: &str) -> Result<Option<f64>, String> {
    if price.trim().is_empty() {
        return Ok(None);
    }
    match price.trim().parse::<f64>() {
        Ok(price) if price.is_finite() && price >= 0.0 => Ok(Some(price)),
        _ => Err("Price must be blank or a finite nonnegative number".into()),
    }
}

fn blocked_attempt_detail(attempt: &AttemptStatus) -> String {
    if !attempt.database_matches {
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
    }
}

fn revealed_rows<'a>(
    rows: impl Iterator<Item = &'a Value>,
    target: Option<&str>,
) -> Vec<&'a Value> {
    let mut rows: Vec<_> = rows.collect();
    if let Some(target) = target
        && let Some(index) = rows
            .iter()
            .position(|row| row.get("uuid").and_then(Value::as_str) == Some(target))
    {
        rows.swap(0, index);
    }
    rows
}

fn scroll_to_publication(id: &'static str) -> Task<Message> {
    iced::widget::operation::scroll_to(
        id,
        iced::widget::operation::AbsoluteOffset {
            x: None,
            y: Some(0.0),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, Paths, PromptConfig, PromptGrammar},
        metadata::{Endpoints, MetadataClient},
        store::Store,
    };
    use iced_runtime::{Action, futures::futures::StreamExt};

    fn app() -> (App, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(root.path()), None).unwrap();
        let store = Arc::new(Store::new(paths, Config::default()).unwrap());
        let metadata = MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        );
        let application = Arc::new(CoreApplication::new(store, metadata));
        let (mut app, _task) = App::new(
            application,
            UiOptions {
                openrouter_model: None,
                ai_timeout: Duration::from_secs(300),
            },
        );
        app.publications = Some(Vec::new());
        app.search_authorized = true;
        app.publish_locks = Some(HashSet::new());
        (app, root)
    }

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

    fn key_event(key: Named, modifiers: iced::keyboard::Modifiers) -> iced::Event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: Key::Named(key),
            modified_key: Key::Named(key),
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Tab),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn publish_keyboard_listener_keeps_captured_editor_keys_and_maps_cell_navigation() {
        let window = iced::window::Id::unique();
        assert!(matches!(
            publish_keyboard_event(
                key_event(Named::Tab, iced::keyboard::Modifiers::NONE),
                iced::event::Status::Ignored,
                window,
            ),
            Some(Message::PublishKeyboardNext { reverse: false })
        ));
        assert!(matches!(
            publish_keyboard_event(
                key_event(Named::Tab, iced::keyboard::Modifiers::SHIFT),
                iced::event::Status::Ignored,
                window,
            ),
            Some(Message::PublishKeyboardNext { reverse: true })
        ));
        assert!(matches!(
            publish_keyboard_event(
                key_event(Named::Space, iced::keyboard::Modifiers::NONE),
                iced::event::Status::Captured,
                window,
            ),
            Some(Message::PublishKeyboardExit)
        ));
        assert!(matches!(
            publish_keyboard_event(
                key_event(Named::Space, iced::keyboard::Modifiers::NONE),
                iced::event::Status::Ignored,
                window,
            ),
            Some(Message::PublishKeyboardToggleCell)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn publish_keyboard_navigation_uses_stable_cells_and_respects_locks() {
        use serde_json::{Value, json};

        let (mut app, _root) = app();
        let id = |suffix: u8| format!("00000000-0000-4000-8000-{suffix:012}");
        let publication = |uuid: String, state: &str, apparitions: Value| {
            json!({
                "uuid": uuid,
                "state": state,
                "title": "Title",
                "description": "Description",
                "submissionTimestamp": 1700000000,
                "files": [],
                "apparitions": apparitions,
            })
        };
        let rows = vec![
            publication(
                id(1),
                "DESCRIBED",
                json!([{"platformName":"deviantart","state":"unpublished"}]),
            ),
            publication(id(2), "DESCRIBED", json!([])),
            publication(
                id(3),
                "APPEARED",
                json!([{"platformName":"deviantart","state":"published_or_scheduled","url":"https://example.test/locked"}]),
            ),
            publication(
                id(4),
                "DESCRIBED",
                json!([{"platformName":"deviantart","state":"unpublished"}]),
            ),
        ];
        app.application
            .store()
            .mutate(|saved| {
                *saved = rows.clone();
                Ok(())
            })
            .unwrap();
        app.publications = Some(rows);
        app.active_tab = Tab::Publish;
        app.publish_locks = Some(HashSet::from([id(4)]));

        let _ = app.update(Message::PublishKeyboardToggle);
        assert!(app.publish_keyboard_mode);
        assert_eq!(app.publish_keyboard_focus.as_deref(), Some(id(1).as_str()));
        let _ = app.update(Message::PublishKeyboardNext { reverse: false });
        assert_eq!(app.publish_keyboard_focus.as_deref(), Some(id(2).as_str()));
        let _ = app.update(Message::PublishKeyboardNext { reverse: true });
        assert_eq!(app.publish_keyboard_focus.as_deref(), Some(id(1).as_str()));
        let _ = app.update(Message::PublishKeyboardNext { reverse: false });
        assert_eq!(app.publish_keyboard_focus.as_deref(), Some(id(2).as_str()));

        let task = app.update(Message::PublishKeyboardToggleCell);
        dispatch(&mut app, task).await;
        let saved = app.application.store().read().unwrap();
        assert_eq!(saved[1]["apparitions"][0]["state"], "unpublished");
        assert_eq!(saved[0]["apparitions"][0]["state"], "unpublished");
        assert_eq!(
            saved[2]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
        assert_eq!(saved[3]["apparitions"][0]["state"], "unpublished");

        app.publish_locks = Some(HashSet::from([id(1), id(2), id(4)]));
        let before = std::fs::read(&app.application.store().paths().json).unwrap();
        let task = app.update(Message::PublishKeyboardToggleCell);
        assert!(iced_runtime::task::into_stream(task).is_none());
        assert!(app.publish_result.as_deref().unwrap().contains("locked"));
        assert_eq!(
            std::fs::read(&app.application.store().paths().json).unwrap(),
            before
        );
        let _ = app.update(Message::PublishLocksFinished(Ok(HashSet::from([
            id(1),
            id(2),
            id(4),
        ]))));
        assert!(app.keyboard_publish_cells().is_empty());
        assert!(app.publish_keyboard_focus.is_none());

        let _ = app.update(Message::PublishKeyboardExit);
        assert!(!app.publish_keyboard_mode);
        assert!(app.publish_keyboard_focus.is_none());
    }

    #[test]
    fn disabled_prompt_feature_does_not_resolve_cards_as_unknown() {
        let (mut app, root) = app();
        app.candidates.push(Candidate {
            path: root.path().join("plain.png"),
            sha512: "0".repeat(128),
        });
        assert!(iced_runtime::task::into_stream(prompt_tasks(&app)).is_none());
        assert!(app.prompt_matches.is_empty());
    }

    #[test]
    fn calendar_renders_a_shared_prompt_for_each_apparition_event() {
        use crate::prompts::PromptVersion;

        let (mut app, _root) = app();
        let artwork = app.application.store().paths().data_dir.join("shared.png");
        app.prompt_matches.insert(
            artwork,
            PromptMatch::Exact(PromptVersion {
                digest: "shared-digest".into(),
                text: "Shared historical prompt".into(),
                paths: vec!["lineage/prompt.txt".into()],
                committed: 1,
            }),
        );
        let publication = serde_json::json!({
            "uuid":"shared-prompt", "state":"APPEARED", "title":"Shared title",
            "description":"Description", "submissionTimestamp":1,
            "files":[{"basename":"shared.png"}],
            "apparitions":[
                {"platformName":"deviantart","state":"published_or_scheduled"},
                {"platformName":"pixiv","state":"published_or_scheduled"}
            ]
        });
        let events = crate::calendar::timeline(&[publication]).unwrap();
        assert_eq!(events.len(), 2);
        for event in &events {
            assert!(calendar::event_prompt_view(&app, event).is_some());
        }
    }

    #[test]
    fn reverse_search_completions_cannot_restore_stale_candidates_or_errors() {
        let (mut app, root) = app();
        let prompt_config = PromptConfig {
            repo: "missing-prompts".into(),
            filename: r"(?P<version>.+)".into(),
            version_hash: "sha256".into(),
        };
        app.prompt_archive = Some(Arc::new(
            crate::prompts::PromptArchive::new(root.path(), &prompt_config).unwrap(),
        ));
        let stale = Candidate {
            path: root.path().join("stale.png"),
            sha512: "a".repeat(128),
        };

        let _ = app.update(Message::PromptQueryChanged("old prompt".into()));
        let _old_prompt_search = app.update(Message::Search);
        let old_request = app.search_request_id;
        let _ = app.update(Message::PromptQueryChanged(String::new()));
        let _new_regular_search = app.update(Message::Search);
        let current_request = app.search_request_id;
        assert!(current_request > old_request);

        let _ = app.update(Message::SearchFinished {
            request_id: current_request,
            result: Ok(Vec::new()),
        });
        let current_result = app.result.clone();
        let _ = app.update(Message::PromptSearchFinished {
            request_id: old_request,
            result: Ok(SearchMatches {
                exact: vec![stale.clone()],
                ..SearchMatches::default()
            }),
        });
        let _ = app.update(Message::SearchFinished {
            request_id: old_request,
            result: Err("stale regular-search error".into()),
        });
        assert!(app.candidates.is_empty());
        assert!(app.prompt_search.is_none());
        assert_eq!(app.result, current_result);

        let _ = app.update(Message::LoadFinished(Err(
            "search authorization revoked".into()
        )));
        let _ = app.update(Message::SearchFinished {
            request_id: current_request,
            result: Ok(vec![stale]),
        });
        assert!(app.candidates.is_empty());
        assert_eq!(
            app.load_error.as_deref(),
            Some("search authorization revoked")
        );
    }

    #[test]
    fn prompt_search_summary_explains_when_results_are_capped() {
        let (mut app, _root) = app();
        let _ = app.update(Message::PromptSearchFinished {
            request_id: app.search_request_id,
            result: Ok(SearchMatches {
                skipped: 3,
                truncated: true,
                ..SearchMatches::default()
            }),
        });
        assert_eq!(
            app.result.as_deref(),
            Some(
                "Prompt search: 0 exact, 0 lineage hints; 3 nearest prompts skipped; results capped, narrow the query or lineage"
            )
        );
    }

    #[test]
    fn missing_configured_prompt_repo_disables_archive_without_config_error() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(Some(root.path()), None).unwrap();
        let config = Config {
            prompts: Some(PromptConfig {
                repo: "missing/archive".into(),
                filename: r"(?P<lineage>.+)_(?P<version>[a-f0-9]+)\.png".into(),
                version_hash: "sha1".into(),
            }),
            ..Config::default()
        };
        let store = Arc::new(Store::new(paths, config).unwrap());
        let metadata = MetadataClient::new(
            reqwest::Client::new(),
            Endpoints::default(),
            "llmfit".into(),
        );
        let application = Arc::new(CoreApplication::new(store, metadata));
        let (app, _) = App::from_application(
            application,
            UiOptions {
                openrouter_model: None,
                ai_timeout: Duration::from_secs(300),
            },
        );
        assert!(app.prompt_archive.is_none());
        assert!(app.prompt_archive_error.is_none());
        assert!(app.prompt_query.is_empty());
        assert!(app.prompt_lineage.is_empty());
        assert!(!root.path().join("missing/archive").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pick_prompt_search_filters_exact_text_and_orders_lineage_hints() {
        async fn completion(task: Task<Message>) -> Message {
            let mut actions = iced_runtime::task::into_stream(task).expect("search task");
            while let Some(action) = actions.next().await {
                if let Action::Output(message) = action {
                    return message;
                }
            }
            panic!("search task produced no result");
        }

        let root = tempfile::tempdir().unwrap();
        let archive_dir = root.path().join("archive");
        let images_dir = root.path().join("picked");
        std::fs::create_dir_all(&archive_dir).unwrap();
        std::fs::create_dir_all(&images_dir).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.invalid"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&archive_dir)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::create_dir_all(archive_dir.join("lineage")).unwrap();
        std::fs::write(
            archive_dir.join("lineage/prompt.txt"),
            "An archived moonlit garden",
        )
        .unwrap();
        for args in [vec!["add", "."], vec!["commit", "-qm", "fixture"]] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&archive_dir)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let prompt_config = PromptConfig {
            repo: "archive".into(),
            filename: r"(?P<lineage>[^_]+)_(?P<version>[a-f0-9]{40})\.png".into(),
            version_hash: "sha1".into(),
        };
        let digest = PromptGrammar::compile(&prompt_config)
            .unwrap()
            .digest(b"An archived moonlit garden")
            .unwrap();
        let exact_path = images_dir.join(format!("prompt.txt_{digest}.png"));
        let hinted_path = images_dir.join(format!("prompt.txt_{}.png", "f".repeat(40)));
        std::fs::write(&exact_path, b"exact image").unwrap();
        std::fs::write(&hinted_path, b"unarchived image").unwrap();
        std::fs::write(images_dir.join("plain.png"), b"unknown image").unwrap();

        let config = Config {
            publicable: vec!["picked".into()],
            prompts: Some(prompt_config),
            ..Config::default()
        };
        let store =
            Arc::new(Store::new(Paths::resolve(Some(root.path()), None).unwrap(), config).unwrap());
        let application = Arc::new(CoreApplication::new(
            store,
            MetadataClient::new(
                reqwest::Client::new(),
                Endpoints::default(),
                "llmfit".into(),
            ),
        ));
        let (mut app, _) = App::from_application(
            application,
            UiOptions {
                openrouter_model: None,
                ai_timeout: Duration::from_secs(300),
            },
        );
        let _ = app.update(Message::LoadFinished(Ok(Vec::new())));
        let _ = app.update(Message::PromptQueryChanged("MOONLIT".into()));
        let result = completion(app.update(Message::Search)).await;
        let Message::PromptSearchFinished {
            result: Ok(matches),
            ..
        } = result
        else {
            panic!("exact query should complete as prompt search");
        };
        assert_eq!(matches.exact.len(), 1);
        assert_eq!(matches.exact[0].path, exact_path);
        assert!(matches.maybe.is_empty());
        assert_eq!(matches.skipped, 1);
        let _ = app.update(Message::PromptSearchFinished {
            request_id: app.search_request_id,
            result: Ok(matches),
        });
        assert_eq!(app.prompt_query, "MOONLIT");
        assert_eq!(
            app.candidates,
            vec![app.prompt_search.as_ref().unwrap().exact[0].clone()]
        );

        let _ = app.update(Message::PromptQueryChanged(String::new()));
        let _ = app.update(Message::PromptLineageChanged("lineage/prompt.txt".into()));
        let result = completion(app.update(Message::Search)).await;
        let Message::PromptSearchFinished {
            result: Ok(matches),
            ..
        } = result
        else {
            panic!("lineage query should complete as prompt search");
        };
        assert_eq!(matches.exact.len(), 1);
        assert_eq!(matches.maybe.len(), 1);
        assert_eq!(matches.maybe[0].path, hinted_path);
        assert_eq!(matches.skipped, 0);
        let _ = app.update(Message::PromptSearchFinished {
            request_id: app.search_request_id,
            result: Ok(matches),
        });
        assert_eq!(
            app.candidates
                .iter()
                .map(|candidate| candidate.path.clone())
                .collect::<Vec<_>>(),
            [exact_path, hinted_path]
        );
        assert!(
            app.result
                .as_deref()
                .unwrap()
                .contains("1 exact, 1 lineage hints")
        );
        let _: Element<'_, Message> = pick::view(&app);
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
        use serde_json::json;

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
        crate::executable_fixture::write_executable(
            &worker,
            &format!(
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
      printf '{{"protocol":1,"type":"scheduled","attempt_id":"%s","uuid":"%s","url":"https://example.test/%s"}}\n' "$attempt" "$uuid" "$uuid"
      ;;
    close) exit 0 ;;
  esac
done
"#,
                calls.display()
            ),
        );
        let application = Arc::new(
            CoreApplication::new(
                store.clone(),
                MetadataClient::new(
                    reqwest::Client::new(),
                    Endpoints::default(),
                    "llmfit".into(),
                ),
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
