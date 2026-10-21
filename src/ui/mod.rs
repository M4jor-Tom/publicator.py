pub mod drafts;
pub mod pick;
pub mod schedule;

use crate::{
    application::Application as CoreApplication,
    images::Candidate,
    model::{Phase, phase},
    workflow::{self, MetadataPatch},
};
use iced::{
    Element, Length, Subscription, Task, Theme,
    widget::{button, column, container, row, scrollable, text, text_input},
};
use serde_json::Value;
use std::{
    collections::HashMap,
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
    started: Instant,
    search_limit: usize,
    search_paths: Vec<String>,
    candidates: Vec<Candidate>,
    previews: HashMap<std::path::PathBuf, Option<Result<std::path::PathBuf, String>>>,
    pending_delete: Option<Candidate>,
    custom_dates: HashMap<String, (String, String)>,
}

impl App {
    fn new(application: Arc<CoreApplication>, options: UiOptions) -> (Self, Task<Message>) {
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
            started: Instant::now(),
            search_limit: 25,
            search_paths: application.store().config().publicable.clone(),
            candidates: Vec::new(),
            previews: HashMap::new(),
            pending_delete: None,
            custom_dates: HashMap::new(),
        };
        (app, Self::load_task(application))
    }

    pub fn from_application(
        application: Arc<CoreApplication>,
        options: UiOptions,
    ) -> (Self, Task<Message>) {
        Self::new(application, options)
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

    fn load_task(application: Arc<CoreApplication>) -> Task<Message> {
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
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoadFinished(Ok(rows)) => {
                self.publications = Some(rows);
                self.load_error = None;
                self.search_authorized = true;
            }
            Message::LoadFinished(Err(error)) => {
                self.publications = None;
                self.load_error = Some(error);
                self.search_authorized = false;
            }
            Message::Navigate(tab) => return self.navigate(tab),
            Message::Edited { uuid, field, text } => {
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
                if let Some(tab) = self.pending_navigation.take() {
                    if self.any_dirty() {
                        self.pending_navigation = Some(tab);
                        return self.save_next_dirty();
                    }
                    self.active_tab = tab;
                }
                if self.pending_close && !self.any_dirty() {
                    return iced::exit();
                }
                if self.any_dirty() {
                    return self.save_next_dirty();
                }
            }
            Message::CloseRequested => {
                if self.any_dirty() {
                    self.pending_close = true;
                    self.result = Some("Saving drafts before close…".into());
                    return self.save_next_dirty();
                }
                return iced::exit();
            }
            Message::ConfirmDiscard => {
                self.drafts.clear();
                self.pending_navigation = None;
                self.pending_close = false;
                return iced::exit();
            }
            Message::KeepEditing => self.pending_close = false,
            Message::Refresh => {
                if self.any_dirty() {
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
                return pick::preview_tasks(self);
            }
            Message::PreviewFinished { source, result } => {
                self.previews.insert(source, Some(result));
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
        self.active_tab = tab;
        Task::none()
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

    fn any_dirty(&self) -> bool {
        self.drafts.values().any(Draft::is_dirty)
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
            let cards = rows.iter().filter_map(|row| {
                if phase(row).ok() != Some(Phase::Scheduled) {
                    return None;
                }
                let uuid = row.get("uuid")?.as_str()?.to_owned();
                let (title, description) = self
                    .drafts
                    .get(&uuid)
                    .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
                    .unwrap_or_else(|| row_metadata(row));
                let title_uuid = uuid.clone();
                let description_uuid = uuid.clone();
                Some(
                    column![
                        text_input("Title", &title).on_input(move |value| Message::Edited {
                            uuid: title_uuid.clone(),
                            field: Field::Title,
                            text: value,
                        }),
                        text_input("Description", &description).on_input(move |value| {
                            Message::Edited {
                                uuid: description_uuid.clone(),
                                field: Field::Description,
                                text: value,
                            }
                        }),
                        text(format!("{uuid}")),
                    ]
                    .spacing(4)
                    .into(),
                )
            });
            content = content.push(scrollable(column(cards).spacing(16)).height(Length::Fill));
        }
        if let Some(result) = &self.result {
            content = content.push(text(result));
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
                started: Instant::now(),
                search_limit: 25,
                search_paths: Vec::new(),
                candidates: Vec::new(),
                previews: HashMap::new(),
                pending_delete: None,
                custom_dates: HashMap::new(),
            },
            root,
        )
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
}
