pub mod calendar;
pub mod describe;
pub mod drafts;
pub mod pick;
mod preview;
pub mod prompt;
pub mod publish;
pub mod schedule;
pub mod style;

use crate::{
    application::{Application as CoreApplication, BatchEvent},
    images::{BasenameIndex, Candidate},
    metadata::{GenerationRequest, LocalStatus, ModelOption, Source, VISION_MODELS},
    model::{Phase, phase},
    prompts::{PromptArchive, PromptMatch, SearchMatches},
    publish_journal::{AttemptState, AttemptStatus, ReconcileOutcome},
    publishing::{PublishCell, PublishCellOutcome, PublishCellState, PublishCompletion},
    workflow::{self, MetadataPatch},
};
use iced::{
    Element, Length, Subscription, Task, Theme,
    keyboard::{Key, key::Named},
    widget::{button, column, container, row, text},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use drafts::{Draft, Field, SentDraft};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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

    /// The tab's list widget id: its unique label.
    fn scroll_id(self) -> &'static str {
        Self::ALL[self as usize].1
    }
}

/// The tab's list: it reports its offset so returning to the tab resumes there. Nothing
/// optional may precede it in its parent: `push` skips a `None`, which shifts the list's tree
/// slot and resets its scroll. Optional lines go inside a fixed header child, or [`floating`].
fn tab_scrollable<'a>(
    tab: Tab,
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Scrollable<'a, Message> {
    iced::widget::scrollable(content)
        .id(tab.scroll_id())
        .height(Length::Fill)
        .on_scroll(move |viewport| Message::Scrolled(tab, viewport.absolute_offset()))
}

/// Floats `banner` over the top of `list`, in view however far it is scrolled.
fn floating<'a>(
    list: impl Into<Element<'a, Message>>,
    banner: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    iced::widget::stack![list.into()]
        .push(banner.map(iced::widget::opaque))
        .into()
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
    /// Initial model selection; `None` selects the first supported vision model.
    pub openrouter_model: Option<ModelOption>,
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

/// What a reload read off the UI thread: the store rows, and which of the candidates listed when
/// it started are gone from disk.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub rows: Vec<Value>,
    pub missing: HashSet<PathBuf>,
}

/// Rows that no candidate check accompanies.
impl From<Vec<Value>> for Loaded {
    fn from(rows: Vec<Value>) -> Self {
        Self {
            rows,
            missing: HashSet::new(),
        }
    }
}

/// A Describe card's place in the latest generation run that covered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationState {
    Queued,
    Generating,
    Generated,
    Failed(String),
}

impl GenerationState {
    pub fn busy(&self) -> bool {
        matches!(self, Self::Queued | Self::Generating)
    }
}

/// The marked batch behind the summary line under "Generate marked".
#[derive(Clone, Debug, PartialEq, Eq)]
enum BatchProgress {
    Running {
        ids: Vec<String>,
        /// The active record's index in `ids`, from its start event.
        active: Option<usize>,
        done: usize,
        failed: usize,
    },
    Finished {
        succeeded: usize,
        failed: usize,
    },
    Failed(String),
}

const WAIT_FOR_GENERATION: &str = "Wait for this image's generation to finish";

#[derive(Debug, Clone)]
pub enum Message {
    /// A store snapshot, stamped with the reload generation that requested it.
    LoadFinished(u64, Result<Loaded, String>),
    Navigate(Tab),
    Scrolled(Tab, iced::widget::scrollable::AbsoluteOffset),
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
    /// A thumbnail placeholder entered (`shown`) or left the visible part of its list.
    PreviewShown {
        key: String,
        shown: bool,
    },
    /// On success, the file the row previews from and its cached thumbnail.
    RowPreviewFinished {
        uuid: String,
        result: Result<(PathBuf, PathBuf), String>,
    },
    /// Shows a file over the whole window until `CloseViewer`.
    OpenViewer(PathBuf),
    CloseViewer,
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
    AskDeletePublication(String),
    ConfirmDeletePublication,
    CancelDeletePublication,
    PublicationDeleted {
        uuid: String,
        result: Result<(), String>,
    },
    SaveDraft(String),
    ApproveMetadata(String),
    ReturnToSchedule(String),
    ReturnToDescribe(String),
    GenerateOne(String),
    GenerateMarked,
    ConfirmLocalInstall,
    CancelLocalInstall,
    SelectSource(Source),
    SelectModel(ModelOption),
    GenerationProgress(BatchEvent),
    GenerationFinished {
        uuid: Option<String>,
        result: Result<(), String>,
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
    /// A publisher run's completion, stamped with the run that started it.
    PublishFinished(u64, PublishCompletion),
    PromptMatchesFinished(Vec<(PathBuf, PromptMatch)>),
    /// Calendar artwork and prompts, stamped with the reload generation of their rows.
    CalendarPrepared(u64, Result<calendar::Preparation, String>),
    /// A Calendar thumbnail, stamped with the reload generation of the rows it previews.
    CalendarPreviewFinished {
        generation: u64,
        uuid: String,
        result: Result<PathBuf, String>,
    },
    ToggleCalendarEvent(String),
    /// Show or hide a Pick card's prompt text (collapsed by default).
    TogglePickPrompt(PathBuf),
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
    load_generation: u64,
    /// The generation of the accepted `LoadFinished` that last set `publications`.
    rows_generation: u64,
    search_request_id: u64,
    search_limit: usize,
    search_paths: Vec<String>,
    prompt_query: String,
    prompt_lineage: String,
    prompt_search: Option<SearchMatches>,
    candidates: Vec<Candidate>,
    previews: HashMap<std::path::PathBuf, Option<Result<std::path::PathBuf, String>>>,
    row_previews: HashMap<String, Option<Result<std::path::PathBuf, String>>>,
    /// Per row UUID with a thumbnail: the file it previews from (`artwork::preview_source`).
    row_sources: HashMap<String, PathBuf>,
    /// The file shown over the whole window (`OpenViewer`).
    viewer: Option<PathBuf>,
    /// Preview keys (candidate paths, row UUIDs) whose placeholder is on screen.
    shown_previews: HashSet<String>,
    description_editors: HashMap<String, iced::widget::text_editor::Content>,

    pending_delete: Option<Candidate>,
    pending_publication_delete: Option<String>,
    scroll_offsets: HashMap<Tab, iced::widget::scrollable::AbsoluteOffset>,
    custom_dates: HashMap<String, (String, String)>,
    source: Source,
    model: ModelOption,
    generation_states: HashMap<String, GenerationState>,
    batch: Option<BatchProgress>,
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
    publish_run: u64,
    pending_publish: bool,
    publish_options: HashMap<String, PublishOptionsDraft>,
    publish_keyboard_mode: bool,
    publish_keyboard_focus: Option<String>,
    publish_locks: Option<HashSet<String>>,
    publish_lock_error: Option<String>,
    publish_attempts: Vec<AttemptStatus>,
    publish_outcomes: HashMap<PublishCell, PublishCellOutcome>,
    reconcile_urls: HashMap<String, String>,
    prompt_archive: Option<Arc<PromptArchive>>,
    prompt_archive_error: Option<String>,
    prompt_matches: HashMap<PathBuf, PromptMatch>,
    /// Pick cards whose prompt text is shown.
    pick_prompts_open: HashSet<PathBuf>,
    calendar_preparing: bool,
    calendar_prepared: bool,
    calendar_error: Option<String>,
    /// Per row UUID: the file its artwork previews from and its recorded SHA-512
    /// (`artwork::preview_source`).
    calendar_artwork: HashMap<String, Result<(PathBuf, String), String>>,
    calendar_previews: HashMap<String, Option<Result<PathBuf, String>>>,
    calendar_expanded: HashSet<String>,
    /// The data directory's basename index, walked at most once per load (`artwork_index`).
    artwork_index: Arc<OnceLock<Result<BasenameIndex, String>>>,
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
        let model = options
            .openrouter_model
            .clone()
            .unwrap_or_else(|| VISION_MODELS[0].clone());
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
            load_generation: 0,
            rows_generation: 0,
            search_request_id: 0,
            search_limit: 25,
            search_paths: application.store().config().publicable.clone(),
            prompt_query: String::new(),
            prompt_lineage: String::new(),
            prompt_search: None,
            candidates: Vec::new(),
            previews: HashMap::new(),
            row_previews: HashMap::new(),
            row_sources: HashMap::new(),
            viewer: None,
            shown_previews: HashSet::new(),
            description_editors: HashMap::new(),
            pending_delete: None,
            pending_publication_delete: None,
            scroll_offsets: HashMap::new(),
            custom_dates: HashMap::new(),
            source: Source::OpenRouter,
            model,
            generation_states: HashMap::new(),
            batch: None,
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
            publish_run: 0,
            pending_publish: false,
            publish_options: HashMap::new(),
            publish_keyboard_mode: false,
            publish_keyboard_focus: None,
            publish_locks: None,
            publish_lock_error: None,
            publish_attempts: Vec::new(),
            publish_outcomes: HashMap::new(),
            reconcile_urls: HashMap::new(),
            prompt_archive,
            prompt_archive_error,
            prompt_matches: HashMap::new(),
            pick_prompts_open: HashSet::new(),
            calendar_preparing: false,
            calendar_prepared: false,
            calendar_error: None,
            calendar_artwork: HashMap::new(),
            calendar_previews: HashMap::new(),
            calendar_expanded: HashSet::new(),
            artwork_index: Default::default(),
            url_opener,
        };
        (app, Self::load_task(application, 0, None, Vec::new()))
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
    /// The header status line.
    pub fn status_message(&self) -> Option<&str> {
        self.result.as_deref()
    }
    pub fn generation_state(&self, uuid: &str) -> Option<&GenerationState> {
        self.generation_states.get(uuid)
    }
    fn generation_busy(&self, uuid: &str) -> bool {
        self.generation_state(uuid)
            .is_some_and(GenerationState::busy)
    }
    /// Refuses a user action on a card whose generation is queued or running, saying why.
    fn generation_blocked(&mut self, uuid: &str) -> bool {
        let busy = self.generation_busy(uuid);
        if busy {
            self.result = Some(WAIT_FOR_GENERATION.into());
        }
        busy
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
    /// The file a row's Calendar artwork previews from and its recorded SHA-512.
    pub fn calendar_art(&self, uuid: &str) -> Option<(&Path, &str)> {
        let (path, sha512) = self.calendar_artwork.get(uuid)?.as_ref().ok()?;
        Some((path, sha512))
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

    /// Loads the store and checks which `candidates` still exist; on Refresh, `invalidate` first
    /// makes the prompt archive rebuild its index on the next resolve (this waits for any
    /// in-flight rebuild, so it stays off the UI thread).
    fn load_task(
        application: Arc<CoreApplication>,
        generation: u64,
        invalidate: Option<Arc<PromptArchive>>,
        candidates: Vec<PathBuf>,
    ) -> Task<Message> {
        let lock_application = application.clone();
        Task::batch([
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        if let Some(archive) = invalidate {
                            // A poisoned archive lock fails every later resolve too, and the
                            // cards report it there.
                            let _ = archive.invalidate();
                        }
                        let rows = application
                            .mutate(None, |store| store.read())
                            .map_err(|error| error.to_string())?;
                        let missing = candidates.into_iter().filter(|path| !path.exists());
                        Ok(Loaded {
                            rows,
                            missing: missing.collect(),
                        })
                    })
                    .await
                    .unwrap_or_else(|error| Err(format!("Could not load publications: {error}")))
                },
                move |result| Message::LoadFinished(generation, result),
            ),
            Self::publish_locks_task(lock_application),
        ])
    }

    fn next_search_request(&mut self) -> u64 {
        self.search_request_id = self.search_request_id.wrapping_add(1);
        self.search_request_id
    }

    fn reload_task(&mut self) -> Task<Message> {
        self.reload_invalidating(None)
    }

    fn reload_invalidating(&mut self, archive: Option<Arc<PromptArchive>>) -> Task<Message> {
        self.next_search_request();
        self.load_generation = self.load_generation.wrapping_add(1);
        let candidates = self.candidates.iter().map(|c| c.path.clone()).collect();
        Self::load_task(
            self.application.clone(),
            self.load_generation,
            archive,
            candidates,
        )
    }

    /// The one guard for reload and calendar replies: a completion stamped before the latest
    /// `reload_task` describes an older snapshot and must not replace newer rows or artwork.
    /// A calendar preparation or thumbnail carries the generation of the rows it read, so one
    /// started while a reload is pending is dropped and that reload's own request stands.
    fn stale_load(&mut self, message: &Message) -> bool {
        let (Message::LoadFinished(generation, _)
        | Message::CalendarPrepared(generation, _)
        | Message::CalendarPreviewFinished { generation, .. }) = message
        else {
            return false;
        };
        if *generation == self.load_generation {
            return false;
        }
        if matches!(message, Message::CalendarPrepared(..)) {
            self.calendar_preparing = false;
        }
        true
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
        if self.stale_load(&message) {
            return Task::none();
        }
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
            | Message::GenerationProgress(..)
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
            Message::Scrolled(tab, offset) => {
                self.scroll_offsets.insert(tab, offset);
                Task::none()
            }
            message => self.update_workflow(message),
        }
    }

    fn update_workflow(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OptionPriceChanged { .. }
            | Message::OptionTierChanged { .. }
            | Message::OptionGalleryChanged { .. }
            | Message::SaveOptions(..)
            | Message::OptionsSaved { .. }
            | Message::ReturnToDescribe(..) => self.update_options(message),
            Message::ReconcileUrlChanged { .. }
            | Message::ReconcileAttempt { .. }
            | Message::ReconcileFinished { .. }
            | Message::OpenActualUrl(..)
            | Message::UrlOpened { .. } => self.update_reconciliation(message),
            Message::SearchLimit(..)
            | Message::SearchPath(..)
            | Message::PromptQueryChanged(..)
            | Message::PromptLineageChanged(..)
            | Message::TogglePickPrompt(..)
            | Message::Search => self.update_search(message),
            Message::SearchFinished { .. }
            | Message::PromptSearchFinished { .. }
            | Message::PromptMatchesFinished(..) => self.update_search_results(message),
            Message::PreviewFinished { .. }
            | Message::PreviewShown { .. }
            | Message::RowPreviewFinished { .. }
            | Message::CalendarPrepared(..)
            | Message::CalendarPreviewFinished { .. }
            | Message::ToggleCalendarEvent(..)
            | Message::OpenViewer(..)
            | Message::CloseViewer => self.update_previews(message),
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
            Message::AskDeletePublication(..)
            | Message::ConfirmDeletePublication
            | Message::CancelDeletePublication
            | Message::PublicationDeleted { .. } => self.update_publication_delete(message),
            Message::LoadFinished(..)
            | Message::PublishLocksFinished(..)
            | Message::PublishStatusesFinished(..)
            | Message::Navigate(..)
            | Message::Scrolled(..)
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
            | Message::GenerationProgress(..)
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
        if let Message::LoadFinished(generation, _) = message {
            self.rows_generation = generation;
        }
        match message {
            Message::LoadFinished(_, Ok(loaded)) => return self.on_load_finished_loaded(loaded),
            Message::LoadFinished(_, Err(error)) => {
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
        if self.publish_active {
            // Receipt loading needs the publisher lock that the active delivery holds.
            self.result = Some("Publishing is active; refresh after it finishes.".into());
        } else if self.any_dirty() || self.publish_options.values().any(|options| options.dirty) {
            self.result = Some("Save or explicitly discard drafts before refresh.".into());
        } else {
            // Refresh re-derives what ordinary reloads keep: prompt labels (resolved again from
            // the invalidated archive once the reload lands) and unsaved custom date inputs.
            self.prompt_matches.clear();
            self.custom_dates.clear();
            self.generation_states.retain(|_, state| state.busy());
            if !matches!(self.batch, Some(BatchProgress::Running { .. })) {
                self.batch = None;
            }
            return self.reload_invalidating(self.prompt_archive.clone());
        }
        Task::none()
    }

    /// Every close continuation exits here: an active publisher keeps the window open, with its
    /// wait/cancel row, until the durable outcome and process cleanup are handled.
    fn close_now(&mut self) -> Task<Message> {
        if self.publish_active {
            self.pending_close = true;
            self.result = Some(
                "Publishing is active. Wait or cancel after the current delivery before closing."
                    .into(),
            );
            return Task::none();
        }
        self.pending_close = false;
        iced::exit()
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
                return self.close_now();
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
        // Escape closes the viewer first; other keys would act on cells hidden under it.
        if self.viewer.is_some() {
            if matches!(message, Message::PublishKeyboardExit) {
                self.viewer = None;
            }
            return Task::none();
        }
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
            Message::GenerationProgress(event) => return self.on_generation_progress(event),
            Message::GenerationFinished { uuid, result } => {
                return self.on_generation_finished(uuid, result);
            }
            _ => unreachable!("message routed to generation handler"),
        }
        Task::none()
    }

    fn update_metadata(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SaveDraft(uuid) => self.save_draft(&uuid),
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
            Message::PublishFinished(run, completion) => {
                return self.on_publish_finished(run, completion);
            }
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
            Message::ReturnToDescribe(uuid) => self.return_to_describe(uuid),
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
            Message::TogglePickPrompt(path) => toggle(&mut self.pick_prompts_open, path),
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

    fn on_preview_shown(&mut self, key: String, shown: bool) -> Task<Message> {
        if !shown {
            self.shown_previews.remove(&key);
            return Task::none();
        }
        self.shown_previews.insert(key);
        // Each loader takes only its own keys: candidate paths or row UUIDs.
        Task::batch([pick::preview_tasks(self), describe::preview_tasks(self)])
    }

    fn update_viewer(&mut self, message: Message) -> Task<Message> {
        self.viewer = match message {
            Message::OpenViewer(path) => Some(path),
            _ => None,
        };
        Task::none()
    }

    fn update_previews(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CalendarPrepared(_, Ok(prepared)) => {
                self.calendar_preparing = false;
                self.calendar_prepared = true;
                self.calendar_error = None;
                self.calendar_artwork = prepared.artwork;
                self.prompt_matches.extend(prepared.prompts);
                return calendar::preview_tasks(self);
            }
            Message::CalendarPrepared(_, Err(error)) => {
                self.calendar_preparing = false;
                self.calendar_prepared = true;
                self.calendar_error = Some(error);
            }
            Message::CalendarPreviewFinished { uuid, result, .. } => {
                self.calendar_previews.insert(uuid, Some(result));
                return calendar::preview_tasks(self);
            }
            Message::ToggleCalendarEvent(key) => {
                toggle(&mut self.calendar_expanded, key);
                return calendar::preview_tasks(self);
            }
            // Each finished preview frees a slot: keep filling the on-screen placeholders.
            Message::PreviewFinished { source, result } => {
                self.previews.insert(source, Some(result));
                return pick::preview_tasks(self);
            }
            Message::PreviewShown { key, shown } => return self.on_preview_shown(key, shown),
            Message::RowPreviewFinished { uuid, result } => {
                let result = result.map(|(source, thumb)| {
                    self.row_sources.insert(uuid.clone(), source);
                    thumb
                });
                self.row_previews.insert(uuid, Some(result));
                return describe::preview_tasks(self);
            }
            Message::OpenViewer(..) | Message::CloseViewer => return self.update_viewer(message),
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

    fn update_publication_delete(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AskDeletePublication(uuid) if self.generation_blocked(&uuid) => {}
            Message::AskDeletePublication(uuid) => self.pending_publication_delete = Some(uuid),
            Message::ConfirmDeletePublication => return schedule::delete_task(self),
            Message::CancelDeletePublication => self.pending_publication_delete = None,
            Message::PublicationDeleted { uuid, result } => {
                if result.is_ok() {
                    // The confirmed deletion discards the row's unsaved input with it.
                    self.drafts.remove(&uuid);
                    self.description_editors.remove(&uuid);
                    self.custom_dates.remove(&uuid);
                }
                return self.on_workflow_finished("Deleted publication".into(), result);
            }
            _ => unreachable!("message routed to publication delete handler"),
        }
        Task::none()
    }

    /// Clean draft fields take the authoritative stored text, and drafts with nothing unsaved or
    /// in flight are dropped, so a reload never keeps showing an older presentation copy.
    fn rebase_drafts(&mut self, rows: &[Value]) {
        self.drafts.retain(|uuid, draft| {
            if let Some(row) = rows
                .iter()
                .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid.as_str()))
            {
                let workflow::Metadata { title, description } = workflow::Metadata::of(row);
                draft.rebase(title, description);
            }
            draft.is_dirty() || draft.is_saving()
        });
    }

    fn on_load_finished_loaded(&mut self, Loaded { rows, missing }: Loaded) -> Task<Message> {
        self.rebase_drafts(&rows);
        pick::retain_offerable(&mut self.candidates, &rows, &missing);
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
        self.artwork_index = Default::default();
        let shown = match self.active_tab {
            Tab::Calendar => calendar::prepare_task(self),
            _ => describe::preview_tasks(self),
        };
        // Candidates without a label (cleared by Refresh) resolve again, after its invalidation.
        Task::batch([shown, prompt_tasks(self)])
    }

    fn close_after_status_refresh(&mut self) -> Task<Message> {
        if self.pending_close {
            if self.any_dirty() {
                return self.save_next_dirty();
            }
            if self.publish_options.values().any(|options| options.dirty) {
                return self.save_next_options();
            }
            return self.close_now();
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

    fn on_generation_progress(&mut self, event: BatchEvent) -> Task<Message> {
        let Some(BatchProgress::Running {
            ids,
            active,
            done,
            failed,
        }) = &mut self.batch
        else {
            return Task::none();
        };
        match event {
            BatchEvent::Admitted(admitted) => {
                for id in ids.iter().filter(|id| !admitted.contains(id)) {
                    self.generation_states.remove(id);
                }
                for id in &admitted {
                    self.generation_states
                        .insert(id.clone(), GenerationState::Queued);
                }
                *ids = admitted;
            }
            BatchEvent::Started(uuid) => {
                *active = ids.iter().position(|id| *id == uuid);
                self.generation_states
                    .insert(uuid, GenerationState::Generating);
            }
            BatchEvent::Finished(record) => {
                let state = match record.error {
                    Some(error) => {
                        *failed += 1;
                        GenerationState::Failed(error)
                    }
                    None => {
                        *done += 1;
                        GenerationState::Generated
                    }
                };
                self.generation_states.insert(record.uuid, state);
                return self.reload_task();
            }
        }
        Task::none()
    }

    /// Ends a run, touching only the cards it covered: a single generation's card, or the
    /// batch's admitted cards, whose unfinished ones lose their state when the batch aborts.
    fn on_generation_finished(
        &mut self,
        uuid: Option<String>,
        result: Result<(), String>,
    ) -> Task<Message> {
        if let Some(uuid) = uuid {
            let state = match result {
                Ok(()) => GenerationState::Generated,
                Err(error) => GenerationState::Failed(error),
            };
            self.generation_states.insert(uuid, state);
        } else if let Some(BatchProgress::Running {
            ids, done, failed, ..
        }) = self.batch.take()
        {
            for id in &ids {
                if self.generation_busy(id) {
                    self.generation_states.remove(id);
                }
            }
            self.batch = Some(match result {
                Ok(()) => BatchProgress::Finished {
                    succeeded: done,
                    failed,
                },
                Err(error) => BatchProgress::Failed(error),
            });
        }
        self.busy = false;
        self.reload_task()
    }

    fn on_description_action(
        &mut self,
        uuid: String,
        action: iced::widget::text_editor::Action,
    ) -> Task<Message> {
        if self.generation_busy(&uuid) {
            if action.is_edit() {
                self.result = Some(WAIT_FOR_GENERATION.into());
            }
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
        if self.generation_blocked(&uuid) {
            return Task::none();
        }
        let current = self
            .drafts
            .get(&uuid)
            .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
            .or_else(|| {
                let saved = workflow::Metadata::of(self.publication(&uuid)?);
                Some((saved.title, saved.description))
            });
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
            return self.close_now();
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
        if self.publish_active {
            self.publish_result = Some("Publishing is already active".into());
            return Task::none();
        }
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

    fn on_publish_finished(&mut self, run: u64, completion: PublishCompletion) -> Task<Message> {
        // Only the active run's first completion ends its lease; a duplicate changes nothing.
        if !self.publish_active || run != self.publish_run {
            return Task::none();
        }
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
            return self.close_now();
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
            return self.close_now();
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
        self.close_now()
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
        Task::batch([pick::preview_tasks(self), prompt_tasks(self)])
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
        Task::batch([pick::preview_tasks(self), prompt_tasks(self)])
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
        // The new tab's placeholders report themselves; the old tab's never report hiding.
        self.shown_previews.clear();
        // A revealed publication is moved first in its list, so it shows at the top.
        let offset = match reveal {
            Some(_) => Default::default(),
            None => self.scroll_offsets.get(&tab).copied().unwrap_or_default(),
        };
        self.revealed_publication = reveal;
        let scroll = iced::widget::operation::scroll_to(tab.scroll_id(), offset);
        match tab {
            Tab::Calendar => Task::batch([calendar::prepare_task(self), scroll]),
            _ => scroll,
        }
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
            self.publish_locks.as_ref(),
        )
        .into_iter()
        .filter_map(|publication| {
            let uuid = publication.get("uuid")?.as_str()?;
            (matches!(
                publish::delivery(publication),
                publish::Delivery::Pending(_)
            ) && !self.selection_locked(Some(uuid)))
            .then(|| uuid.to_owned())
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

    /// Sends a DESCRIBED Publish row back to Describe through the CLI's guarded rollback.
    fn return_to_describe(&mut self, uuid: String) -> Task<Message> {
        if self.options_locked(&uuid) {
            return Task::none();
        }
        if self
            .publish_options
            .get(&uuid)
            .is_some_and(|options| options.dirty)
        {
            self.result = Some("Save options before returning to Describe".into());
            return Task::none();
        }
        let application = self.application.clone();
        self.busy = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || application.roll_back(&uuid).map(|_| ()))
                    .await
                    .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                    .map_err(|error| error.to_string())
            },
            |result| Message::WorkflowFinished {
                message: "Returned to Describe".into(),
                result,
            },
        )
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
        self.publish_run = self.publish_run.wrapping_add(1);
        let run = self.publish_run;
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
            move |completion| Message::PublishFinished(run, completion),
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
            let reveal = self.pending_jump.take();
            return self.activate_tab(tab, reveal);
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

    /// The user's Save draft, refused on a card whose generation is queued or running.
    fn save_draft(&mut self, uuid: &str) -> Task<Message> {
        if self.generation_blocked(uuid) {
            return Task::none();
        }
        self.save_uuid(uuid)
    }

    fn save_uuid(&mut self, uuid: &str) -> Task<Message> {
        if self.generation_busy(uuid) {
            return Task::none();
        }
        let Some(draft) = self.drafts.get_mut(uuid) else {
            return Task::none();
        };
        // One flush per draft: its completion saves any newer revision, so the newest lands last.
        if !draft.is_dirty() || draft.is_saving() {
            return Task::none();
        }
        let sent = draft.begin_save();
        self.save_task(uuid.to_owned(), sent)
    }

    fn approve_metadata(&mut self, uuid: String) -> Task<Message> {
        if self.generation_blocked(&uuid) {
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
        if self.generation_blocked(&uuid) {
            return Task::none();
        }
        if self.drafts.get(&uuid).is_some_and(Draft::is_dirty) {
            self.pending_return = Some(uuid.clone());
            return self.save_uuid(&uuid);
        }
        schedule::return_task(self, uuid)
    }

    fn request_generation(&mut self, uuid: String) -> Task<Message> {
        if self.pending_local_consent.is_some() || self.generation_blocked(&uuid) {
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
        if matches!(self.batch, Some(BatchProgress::Running { .. })) {
            self.result = Some("A batch generation is already running".into());
            return Task::none();
        }
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
        self.busy = true;
        if let Some(uuid) = selection {
            self.generation_states
                .insert(uuid.clone(), GenerationState::Generating);
            let completion_uuid = uuid.clone();
            return Task::perform(
                async move {
                    application
                        .generate_one(&uuid, request)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                },
                move |result| Message::GenerationFinished {
                    uuid: Some(completion_uuid),
                    result,
                },
            );
        }
        // Cards another run holds keep their state; admission narrows the rest to the batch.
        let ids: Vec<String> = self
            .marked_generation_ids()
            .into_iter()
            .filter(|id| !self.generation_busy(id))
            .collect();
        for id in &ids {
            self.generation_states
                .insert(id.clone(), GenerationState::Queued);
        }
        self.batch = Some(BatchProgress::Running {
            ids,
            active: None,
            done: 0,
            failed: 0,
        });
        Task::run(
            iced::stream::channel(1, async move |mut output| {
                use iced::futures::{SinkExt, StreamExt, channel::mpsc};
                let (events, mut received) = mpsc::unbounded();
                let generation = application.generate_marked_with_events(
                    &request,
                    batch_selection.as_deref(),
                    move |event| {
                        let _ = events.unbounded_send(event);
                    },
                );
                // The callback's sender drops with the finished generation, ending `forward`.
                let forward = async {
                    while let Some(event) = received.next().await {
                        let _ = output.send(Message::GenerationProgress(event)).await;
                    }
                };
                let (result, ()) = iced::futures::join!(generation, forward);
                let _ = output
                    .send(Message::GenerationFinished {
                        uuid: None,
                        result: result.map(|_| ()).map_err(|error| error.to_string()),
                    })
                    .await;
            }),
            std::convert::identity,
        )
    }

    fn any_dirty(&self) -> bool {
        self.drafts.values().any(Draft::is_dirty)
    }

    /// A row's thumbnail (`style::art`), opening the file it previews from.
    fn row_art(&self, uuid: &str, height: f32) -> Element<'_, Message> {
        style::art(
            self.row_previews.get(uuid),
            self.row_sources.get(uuid).map(PathBuf::as_path),
            false,
            height,
            uuid,
        )
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
        let count = |tab| match tab {
            Tab::Pick => Some(self.candidates.len()),
            Tab::Schedule => Some(schedule::ordered_rows(rows, None).len()),
            Tab::Describe => Some(describe::ordered_rows(rows, None, |_| false).len()),
            Tab::Publish => {
                Some(publish::ordered_rows(rows, None, self.publish_locks.as_ref()).len())
            }
            Tab::Calendar => None,
        };
        let mut header = row![text("Publish next").font(style::BOLD)]
            .spacing(8)
            .align_y(iced::Alignment::Center);
        for (tab, label) in Tab::ALL {
            let label = match count(tab) {
                Some(n) => format!("{label} ({n})"),
                None => label.to_owned(),
            };
            let control = button(text(label)).style(style::tab(tab == self.active_tab));
            header = header.push(if tab == self.active_tab {
                control
            } else {
                control.on_press(Message::Navigate(tab))
            });
        }
        if let Some(result) = &self.result {
            header = header.push(text(result));
        }
        let mut content = column![match self.active_tab {
            Tab::Pick => pick::view(self),
            Tab::Schedule => schedule::view(self),
            Tab::Describe => describe::view(self),
            Tab::Publish => publish::view(self),
            Tab::Calendar => calendar::view(self),
        }]
        .spacing(16)
        .height(Length::Fill);
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
        let footer = row![
            button("Refresh").on_press(Message::Refresh),
            button("Retry save").on_press(Message::RetrySave),
            button("Keep editing").on_press(Message::KeepEditing),
            button("Discard drafts and close").on_press(Message::ConfirmDiscard),
        ]
        .spacing(8);
        let page = column![
            container(header.wrap())
                .padding(12)
                .width(Length::Fill)
                .style(style::header),
            container(content).padding(20).height(Length::Fill),
            container(footer)
                .padding(8)
                .width(Length::Fill)
                .style(style::header),
        ];
        // The page keeps its tree slot whether or not the viewer covers it, so no scroll resets.
        let viewer = self.viewer.as_deref().map(|path| {
            iced::widget::mouse_area(
                container(
                    iced::widget::image(iced::widget::image::Handle::from_path(path))
                        .content_fit(iced::ContentFit::ScaleDown),
                )
                .center(Length::Fill)
                .style(style::viewer),
            )
            .on_press(Message::CloseViewer)
            .into()
        });
        floating(page, viewer)
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

/// The load's basename index, walked on first use; blocks, so call it from a blocking task.
pub(super) fn artwork_index<'a>(
    index: &'a OnceLock<Result<BasenameIndex, String>>,
    data_dir: &Path,
) -> Result<&'a BasenameIndex, String> {
    index
        .get_or_init(|| {
            crate::images::index_by_basename(data_dir).map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
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
    static THEME: std::sync::LazyLock<Theme> = std::sync::LazyLock::new(style::theme);
    THEME.clone()
}

fn set_option(options: &mut Vec<String>, value: String, selected: bool) {
    if selected && !options.contains(&value) {
        options.push(value);
    } else if !selected {
        options.retain(|option| option != &value);
    }
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

/// Remove `key` from `set` if present, else insert it.
fn toggle<T: Eq + std::hash::Hash>(set: &mut HashSet<T>, key: T) {
    if !set.remove(&key) {
        set.insert(key);
    }
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
        let (mut app, _task) = offline_app(root.path(), &[], Config::default());
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
            match action {
                Action::Output(message) => messages.push(message),
                Action::Exit => panic!("unexpected exit"),
                _ => {}
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
    fn viewer_opens_and_closes_on_the_same_tab() {
        let (mut app, _root) = app();
        app.active_tab = Tab::Schedule;
        let _ = app.update(Message::OpenViewer(PathBuf::from("/art/a.png")));
        assert_eq!(app.viewer.as_deref(), Some(Path::new("/art/a.png")));
        let _ = app.update(Message::CloseViewer);
        assert!(app.viewer.is_none());
        assert_eq!(app.active_tab, Tab::Schedule);
    }

    #[test]
    fn viewer_escape_closes_it_before_publish_keyboard_mode() {
        let (mut app, _root) = app();
        app.active_tab = Tab::Publish;
        app.publish_keyboard_mode = true;
        let _ = app.update(Message::OpenViewer(PathBuf::from("/art/a.png")));
        let _ = app.update(Message::PublishKeyboardToggle);
        assert!(app.publish_keyboard_mode);
        let _ = app.update(Message::PublishKeyboardExit);
        assert!(app.viewer.is_none());
        assert!(app.publish_keyboard_mode);
        let _ = app.update(Message::PublishKeyboardExit);
        assert!(!app.publish_keyboard_mode);
    }

    #[test]
    fn viewer_shows_the_original_or_a_video_thumbnail() {
        let thumb = Path::new("/cache/t.png");
        for (original, shown) in [
            ("/art/a.png", "/art/a.png"),
            ("/art/v.mp4", "/cache/t.png"),
            ("/art/v.MP4", "/cache/t.png"),
        ] {
            assert_eq!(
                style::viewer_file(Path::new(original), thumb),
                Path::new(shown)
            );
        }
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

    /// A reload drops candidates it shows were picked (now referenced) or deleted, so the
    /// Pick list never offers them again.
    #[tokio::test(flavor = "current_thread")]
    async fn reload_removes_picked_and_deleted_candidates_from_the_pick_list() {
        let (mut app, root) = app();
        let candidate = |name: &str, bytes: &[u8]| {
            let path = root.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            Candidate {
                sha512: crate::images::file_sha512(&path).unwrap(),
                path,
            }
        };
        let (picked, deleted, kept) = (
            candidate("picked.png", b"picked"),
            candidate("deleted.png", b"deleted"),
            candidate("kept.png", b"kept"),
        );
        app.candidates = vec![picked.clone(), deleted.clone(), kept.clone()];
        std::fs::remove_file(&deleted.path).unwrap();
        let row = serde_json::json!({
            "uuid": uuid(1), "state": "PICKED", "title": "Title", "description": "Description",
            "submissionTimestamp": 1_700_002_800,
            "files": [{"basename": "elsewhere.png", "sha512sum": picked.sha512}],
            "apparitions": []
        });
        replace_rows(&app, vec![row]);
        let reload = app.reload_task();
        dispatch(&mut app, reload).await;
        assert_eq!(app.candidates, [kept]);
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

        let _ = app.update(Message::LoadFinished(
            0,
            Err("search authorization revoked".into()),
        ));
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
        let config = Config {
            prompts: Some(PromptConfig {
                repo: "missing/archive".into(),
                filename: r"(?P<lineage>.+)_(?P<version>[a-f0-9]+)\.png".into(),
                version_hash: "sha1".into(),
            }),
            ..Config::default()
        };
        let (app, _) = offline_app(root.path(), &[], config);
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
        prompt_archive(root.path(), "An archived moonlit garden");
        let images_dir = root.path().join("picked");
        std::fs::create_dir_all(&images_dir).unwrap();
        let prompt_config = archive_config();
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
        let (mut app, _) = offline_app(root.path(), &[], config);
        let _ = app.update(Message::LoadFinished(0, Ok(Vec::new().into())));
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
        let _task = app.update(Message::LoadFinished(0, Err("malformed JSON".into())));
        assert!(app.publications.is_none());
        assert_eq!(app.load_error.as_deref(), Some("malformed JSON"));
        assert!(!app.search_authorized);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn typed_uncertainty_with_worker_stderr_refreshes_receipt_and_store_through_ui() {
        let (root, application) =
            crate::publishing::timeout_tests::publisher_fixture(2, "Description");
        let data_dir = root.path();
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
            application.with_publisher(crate::publishing::Publisher::new(
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
                Message::PublishFinished(_, completion) => Some(completion),
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
            app.application.store().read().unwrap()[0]["apparitions"][0]["state"],
            "published_or_scheduled"
        );
        assert_eq!(
            app.application.store().read().unwrap()[1]["apparitions"][0]["state"],
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

    fn uuid(n: u64) -> String {
        format!("00000000-0000-4000-8000-{n:012}")
    }

    use crate::git_fixture::git;

    /// `root/archive`, a Git prompt archive whose HEAD holds `lineage/prompt.txt` = `prompt`.
    fn prompt_archive(root: &Path, prompt: &str) -> PathBuf {
        let dir = root.join("archive");
        std::fs::create_dir_all(dir.join("lineage")).unwrap();
        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("lineage/prompt.txt"), prompt).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "fixture"]);
        dir
    }

    fn archive_config() -> PromptConfig {
        PromptConfig {
            repo: "archive".into(),
            filename: r"(?P<lineage>[^_]+)_(?P<version>[a-f0-9]{40})\.png".into(),
            version_hash: "sha1".into(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refresh_invalidates_the_prompt_archive_and_resolves_shown_candidates_again() {
        let root = tempfile::tempdir().unwrap();
        let archive_dir = prompt_archive(root.path(), "archived at HEAD");
        let digest = PromptGrammar::compile(&archive_config())
            .unwrap()
            .digest(b"loose prompt")
            .unwrap();
        let candidate = Candidate {
            path: root.path().join(format!("prompt.txt_{digest}.png")),
            sha512: "0".repeat(128),
        };
        // A reload keeps only candidates still on disk.
        std::fs::write(&candidate.path, b"candidate").unwrap();
        let config = Config {
            prompts: Some(archive_config()),
            ..Config::default()
        };
        let (mut app, init) = offline_app(root.path(), &[], config);
        dispatch(&mut app, init).await;
        let request_id = app.search_request_id;
        let search = Message::SearchFinished {
            request_id,
            result: Ok(vec![candidate.clone()]),
        };
        send(&mut app, search).await;
        let exact = |app: &App| {
            matches!(
                app.prompt_matches.get(&candidate.path),
                Some(PromptMatch::Exact(_))
            )
        };
        assert!(app.prompt_matches.contains_key(&candidate.path) && !exact(&app));

        // HEAD is unchanged, so only an explicit Refresh sees a blob that no ref reaches.
        std::fs::write(archive_dir.join("loose.txt"), "loose prompt").unwrap();
        git(&archive_dir, &["hash-object", "-w", "loose.txt"]);
        send(&mut app, Message::Refresh).await;
        assert!(exact(&app), "{:?}", app.prompt_matches.get(&candidate.path));
    }

    /// A SCHEDULED row whose `{n}.png` exists in `root` with its real hash, as generation needs.
    fn art_row(root: &Path, n: u64) -> Value {
        let image = root.join(format!("{n}.png"));
        crate::image_fixture::solid_image(&image, "white", 4, 3);
        serde_json::json!({
            "uuid": uuid(n), "state": "SCHEDULED", "title": "Title", "description": "Description",
            "submissionTimestamp": 1_700_002_800,
            "files": [{"basename": format!("{n}.png"), "sha512sum": crate::images::file_sha512(&image).unwrap()}],
            "apparitions": []
        })
    }

    /// An App over `rows` and `config` in `root` with its unpolled startup task; providers sit on
    /// a closed port.
    fn offline_app(root: &Path, rows: &[Value], config: Config) -> (App, Task<Message>) {
        let paths = Paths::resolve(Some(root), None).unwrap();
        let store = Arc::new(Store::new(paths, config).unwrap());
        std::fs::write(&store.paths().json, serde_json::to_vec(rows).unwrap()).unwrap();
        let closed = || "http://127.0.0.1:1".to_owned();
        let metadata = MetadataClient::new(
            reqwest::Client::new(),
            Endpoints {
                openrouter: closed(),
                ollama: closed(),
            },
            "llmfit".into(),
        )
        .with_openrouter_key("fixture");
        App::new(
            Arc::new(CoreApplication::new(store, metadata)),
            UiOptions {
                openrouter_model: None,
                ai_timeout: Duration::from_secs(5),
            },
        )
    }

    async fn loaded_offline_app(root: &Path, rows: &[Value]) -> App {
        let (mut app, init) = offline_app(root, rows, Config::default());
        dispatch(&mut app, init).await;
        app
    }

    fn edit(app: &mut App, uuid: &str, field: Field, text: &str) {
        let _ = app.update(Message::Edited {
            uuid: uuid.into(),
            field,
            text: text.into(),
        });
    }

    async fn send(app: &mut App, message: Message) -> Vec<Message> {
        let task = app.update(message);
        dispatch(app, task).await
    }

    /// What a Describe card shows: the draft when one exists, else the loaded row.
    fn shown(app: &App, uuid: &str) -> (String, String) {
        app.drafts
            .get(uuid)
            .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
            .unwrap_or_else(|| pair(app.publication(uuid).unwrap()))
    }

    fn stored(app: &App, uuid: &str) -> (String, String) {
        let rows = app.application.store().read().unwrap();
        pair(rows.iter().find(|row| row["uuid"] == uuid).unwrap())
    }

    fn pair(row: &Value) -> (String, String) {
        let saved = workflow::Metadata::of(row);
        (saved.title, saved.description)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reload_shows_stored_metadata_for_clean_drafts_and_keeps_unsaved_input() {
        use serde_json::json;
        let root = tempfile::tempdir().unwrap();
        let rows = [1, 2, 3].map(|n| art_row(root.path(), n));
        let mut app = loaded_offline_app(root.path(), &rows).await;
        let [one, two, three] = [1, 2, 3].map(uuid);
        for id in [&one, &two, &three] {
            edit(&mut app, id, Field::Title, "Saved manually");
            send(&mut app, Message::SaveDraft(id.clone())).await;
            assert_eq!(stored(&app, id).0, "Saved manually");
        }
        app.application
            .store()
            .mutate(|rows| {
                rows[0]["title"] = json!("Another writer");
                Ok(())
            })
            .unwrap();
        send(&mut app, Message::Refresh).await;
        assert!(
            app.drafts.is_empty(),
            "clean drafts are not presentation copies"
        );
        assert_eq!(shown(&app, &one), stored(&app, &one));

        // A failed generation writes markers and reloads while other drafts hold input.
        edit(&mut app, &one, Field::Title, "In flight");
        let held = app.update(Message::SaveDraft(one.clone()));
        edit(&mut app, &two, Field::Description, "Unsaved");
        app.application
            .store()
            .mutate(|rows| {
                rows[1]["title"] = json!("Newer stored title");
                Ok(())
            })
            .unwrap();
        let generated = send(&mut app, Message::GenerateOne(three.clone())).await;
        assert!(
            generated.iter().any(|message| matches!(
                message,
                Message::GenerationFinished { result: Err(_), .. }
            ))
        );
        let marker = crate::model::GENERATE_MARKER.to_owned();
        assert_eq!(stored(&app, &three), (marker.clone(), marker));
        assert_eq!(shown(&app, &three), stored(&app, &three));
        assert_eq!(
            shown(&app, &two),
            ("Newer stored title".into(), "Unsaved".into())
        );
        assert!(app.drafts[&two].is_field_dirty(Field::Description));
        assert_eq!(shown(&app, &one).0, "In flight");
        assert!(app.drafts[&one].is_saving());

        dispatch(&mut app, held).await;
        assert_eq!(stored(&app, &one).0, "In flight");
        assert_eq!(
            stored(&app, &two),
            ("Newer stored title".into(), "Unsaved".into())
        );
        send(&mut app, Message::Refresh).await;
        assert!(app.drafts.is_empty());
        for id in [&one, &two, &three] {
            assert_eq!(shown(&app, id), stored(&app, id));
        }
    }

    /// Whether a task ends the application, as `close_now` does.
    async fn exits(task: Task<Message>) -> bool {
        let Some(mut actions) = iced_runtime::task::into_stream(task) else {
            return false;
        };
        while let Some(action) = actions.next().await {
            if matches!(action, Action::Exit) {
                return true;
            }
        }
        false
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_after_a_failed_then_retried_save_saves_the_newer_edit_before_exiting() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let mut app = loaded_offline_app(root.path(), &[art_row(root.path(), 1)]).await;
        let id = uuid(1);
        let writable = |mode| {
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(mode)).unwrap()
        };
        edit(&mut app, &id, Field::Title, "Kept through failure");
        writable(0o555);
        send(&mut app, Message::SaveDraft(id.clone())).await;
        writable(0o755);
        assert!(app.result.as_deref().unwrap().starts_with("Save failed"));
        assert_eq!(stored(&app, &id).0, "Title");
        send(&mut app, Message::RetrySave).await;
        assert_eq!(stored(&app, &id).0, "Kept through failure");

        edit(&mut app, &id, Field::Title, "Saved by close");
        let save = app.update(Message::CloseRequested);
        assert!(app.pending_close);
        let finished = run_task(save).await;
        assert_eq!(stored(&app, &id).0, "Saved by close");
        assert_eq!(finished.len(), 1);
        let close = app.update(finished.into_iter().next().unwrap());
        assert!(exits(close).await, "close waits for the save, then exits");
    }

    fn calendar_row(n: u64, basename: &str) -> Value {
        serde_json::json!({
            "uuid": uuid(n), "state": "DESCRIBED", "title": "Title", "description": "Description",
            "submissionTimestamp": 1_700_002_800,
            "files": [{"basename": basename, "sha512sum": "0".repeat(128)}],
            "apparitions": [{"platformName": "deviantart", "state": "unpublished"}]
        })
    }

    fn replace_rows(app: &App, rows: Vec<Value>) {
        app.application
            .store()
            .mutate(|saved| {
                *saved = rows;
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replies_from_before_the_latest_reload_cannot_replace_newer_rows_or_calendar() {
        let root = tempfile::tempdir().unwrap();
        let (mut app, init) = offline_app(
            root.path(),
            &[calendar_row(1, "old.png")],
            Config::default(),
        );
        let startup = run_task(init).await;
        for message in startup.clone() {
            let _ = app.update(message);
        }
        assert!(app.publications.is_some(), "the startup load is accepted");
        let old_prepared = run_task(app.update(Message::Navigate(Tab::Calendar))).await;
        assert!(app.calendar_preparing);

        replace_rows(&app, vec![calendar_row(2, "new.png")]);
        send(&mut app, Message::Refresh).await;
        assert!(app.calendar_artwork.contains_key(&uuid(2)));
        for message in startup.into_iter().chain(old_prepared).filter(|message| {
            matches!(
                message,
                Message::LoadFinished(..) | Message::CalendarPrepared(..)
            )
        }) {
            assert!(iced_runtime::task::into_stream(app.update(message)).is_none());
        }
        assert_eq!(
            app.publications.as_ref().unwrap()[0]["files"][0]["basename"],
            "new.png"
        );
        assert_eq!(app.calendar_artwork.keys().collect::<Vec<_>>(), [&uuid(2)]);
        let data_dir = &app.application.store().paths().data_dir;
        assert!(!app.prompt_matches.contains_key(&data_dir.join("old.png")));
        assert!(app.prompt_matches.contains_key(&data_dir.join("new.png")));
        assert!(app.calendar_prepared && !app.calendar_preparing);
    }

    /// Pick prompt text starts hidden; each card's toggle opens and closes only that card.
    #[tokio::test(flavor = "current_thread")]
    async fn pick_prompts_start_closed_and_toggle_per_card() {
        let root = tempfile::tempdir().unwrap();
        let mut app = loaded_offline_app(root.path(), &[]).await;
        let (one, two) = (root.path().join("one.png"), root.path().join("two.png"));
        assert!(app.pick_prompts_open.is_empty());
        let _ = app.update(Message::TogglePickPrompt(one.clone()));
        assert!(app.pick_prompts_open.contains(&one) && !app.pick_prompts_open.contains(&two));
        let _ = app.update(Message::TogglePickPrompt(one.clone()));
        assert!(app.pick_prompts_open.is_empty());
    }

    /// A Calendar thumbnail requested before a Refresh cannot fill the reloaded Calendar's slot,
    /// which the reload's own preparation then requests again.
    #[tokio::test(flavor = "current_thread")]
    async fn calendar_preview_from_before_a_reload_cannot_fill_the_reloaded_slot() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("art.png"), "not the recorded content").unwrap();
        let mut app = loaded_offline_app(root.path(), &[calendar_row(1, "art.png")]).await;
        let _ = app.update(Message::ToggleCalendarEvent(format!("{}:0", uuid(1))));
        let mut old_preview = Vec::new();
        for message in run_task(app.update(Message::Navigate(Tab::Calendar))).await {
            old_preview.extend(run_task(app.update(message)).await);
        }
        assert!(matches!(
            old_preview[..],
            [Message::CalendarPreviewFinished { .. }]
        ));

        let mut reloaded = Vec::new();
        for message in run_task(app.update(Message::Refresh)).await {
            reloaded.extend(run_task(app.update(message)).await);
        }
        assert!(app.calendar_previews.is_empty(), "the reload cleared it");
        for message in old_preview {
            let _ = app.update(message);
        }
        assert!(
            app.calendar_previews.is_empty(),
            "the old thumbnail is dropped"
        );
        for message in reloaded {
            let _ = app.update(message);
        }
        assert_eq!(app.calendar_previews.get(&uuid(1)), Some(&None));
    }

    /// A preparation that read the rows a pending reload replaces cannot overwrite that reload's
    /// own preparation, even when it finishes last.
    #[tokio::test(flavor = "current_thread")]
    async fn calendar_preparation_of_replaced_rows_is_dropped_after_the_reload_prepares() {
        let root = tempfile::tempdir().unwrap();
        let mut app = loaded_offline_app(root.path(), &[calendar_row(1, "old.png")]).await;
        replace_rows(&app, vec![calendar_row(2, "new.png")]);
        let reload = run_task(app.update(Message::Refresh)).await;
        let old_prepared = run_task(app.update(Message::Navigate(Tab::Calendar))).await;
        for message in reload {
            let task = app.update(message);
            dispatch(&mut app, task).await;
        }
        for message in old_prepared {
            assert!(iced_runtime::task::into_stream(app.update(message)).is_none());
        }
        assert_eq!(app.calendar_artwork.keys().collect::<Vec<_>>(), [&uuid(2)]);
        assert!(app.calendar_prepared && !app.calendar_preparing);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stale_calendar_preparation_ends_preparing_without_extending_prompts() {
        let root = tempfile::tempdir().unwrap();
        let mut app = loaded_offline_app(root.path(), &[calendar_row(1, "old.png")]).await;
        let old_prepared = run_task(app.update(Message::Navigate(Tab::Calendar))).await;
        std::fs::write(&app.application.store().paths().json, "{").unwrap();
        let reload = send(&mut app, Message::Refresh).await;
        assert!(
            reload
                .iter()
                .any(|message| matches!(message, Message::LoadFinished(1, Err(_))))
        );
        assert!(
            app.calendar_preparing,
            "the generation-0 preparation is still out"
        );
        for message in old_prepared {
            let _ = app.update(message);
        }
        assert!(!app.calendar_preparing);
        assert!(app.prompt_matches.is_empty() && app.calendar_artwork.is_empty());
    }

    /// Every cell of a week, blank ones included, is painted down to the week's tallest day;
    /// weeks without a tall day keep the 78 px minimum.
    #[tokio::test(flavor = "current_thread")]
    async fn calendar_week_cells_share_the_tallest_day_height() {
        use iced::{Color, Size};
        use iced_runtime::core::{Layout, layout::Limits, renderer, widget::Tree};
        let (mut app, _root) = app();
        // Six events on Friday 3 November 2023, in the week after two blank cells.
        let rows = (1..=6)
            .map(|n| {
                let mut row = calendar_row(n, "art.png");
                row["submissionTimestamp"] = 1_699_012_800.into();
                row
            })
            .collect::<Vec<_>>();
        let events = crate::calendar::timeline(&rows).unwrap();
        app.publications = Some(rows);
        let month = chrono::NaiveDate::from_ymd_opt(2023, 11, 1).unwrap();
        let mut view = calendar::month_view(&app, &events, month, None, 0, chrono_tz::UTC);
        let mut renderer = <iced::Renderer as renderer::Headless>::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        )
        .await
        .unwrap();
        let mut tree = Tree::new(&view);
        let limits = Limits::new(Size::ZERO, Size::new(700.0, f32::INFINITY));
        let node = view.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let size = node.size();
        view.as_widget().draw(
            &tree,
            &mut renderer,
            &Theme::Dark,
            &renderer::Style {
                text_color: Color::WHITE,
            },
            Layout::new(&node),
            iced::mouse::Cursor::Unavailable,
            &iced::Rectangle::with_size(size),
        );
        let unpainted = Color::from_rgb8(255, 0, 255);
        let (width, height) = (size.width as u32, size.height as u32);
        let pixels =
            renderer::Headless::screenshot(&mut renderer, Size::new(width, height), 1.0, unpainted);
        // The month is its title, the weekday names, then its weeks.
        let weeks = Layout::new(&node)
            .children()
            .nth(2)
            .unwrap()
            .children()
            .collect::<Vec<_>>();
        let heights = weeks
            .iter()
            .map(|week| week.bounds().height)
            .collect::<Vec<_>>();
        assert!(heights[0] > 100.0, "the six-event week grows: {heights:?}");
        assert!(heights[1..].iter().all(|&h| h == 78.0), "{heights:?}");
        for (index, week) in weeks.iter().enumerate() {
            let bounds = week.bounds();
            let y = (bounds.y + bounds.height - 3.0) as u32;
            for column in 0..7 {
                let x = (bounds.x + (column as f32 + 0.5) * bounds.width / 7.0) as u32;
                let at = ((y * width + x) * 4) as usize;
                assert_ne!(
                    pixels[at..at + 3],
                    [255, 0, 255],
                    "week {index} column {column} is blank above its bottom"
                );
            }
        }
    }

    #[test]
    fn custom_date_input_ends_at_a_date_save_or_refresh_and_blocks_its_approval() {
        let (mut app, root) = app();
        let [first, second] = [1, 2].map(uuid);
        for id in [&first, &second] {
            let _ = app.update(Message::CustomDate {
                uuid: id.clone(),
                value: "2027-01-01".into(),
            });
        }
        let refused = app.update(Message::ApproveDate(second.clone()));
        assert!(iced_runtime::task::into_stream(refused).is_none());
        assert!(app.result.as_deref().unwrap().contains("Save Custom"));

        let _preset = app.update(Message::PresetDate {
            uuid: first.clone(),
            profile: "default".into(),
        });
        assert!(!app.custom_dates.contains_key(&first));
        assert!(app.custom_dates.contains_key(&second));

        app.prompt_matches.insert(
            root.path().join("art.png"),
            PromptMatch::Unknown("resolved before refresh".into()),
        );
        let _refresh = app.update(Message::Refresh);
        assert!(app.custom_dates.is_empty());
        assert!(
            app.prompt_matches.is_empty(),
            "Refresh resolves prompts again"
        );
    }
}
