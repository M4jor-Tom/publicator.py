use super::{App, Message, Tab, style};
use crate::{
    application::Application,
    images::{self, Candidate, Search},
    prompts,
};
use iced::{
    Alignment, Element, Length, Task,
    widget::{button, checkbox, column, container, pick_list, row, scrollable, text, text_input},
};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn search_candidates(
    store: &crate::store::Store,
    limit: usize,
    paths: &[String],
) -> crate::error::Result<Vec<Candidate>> {
    images::find_candidates(
        store,
        &Search {
            limit,
            paths: paths.to_vec(),
        },
    )
}

pub(super) fn search_task(app: &App, request_id: u64) -> Task<Message> {
    let application = app.application.clone();
    let search = Search {
        limit: app.search_limit,
        paths: app.search_paths.clone(),
    };
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                application.mutate(None, |store| images::find_candidates(store, &search))
            })
            .await
            .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
            .map_err(|error| error.to_string())
        },
        move |result| Message::SearchFinished { request_id, result },
    )
}

pub(super) fn prompt_search_task(app: &App, request_id: u64) -> Task<Message> {
    let Some(archive) = app.prompt_archive.clone() else {
        return Task::done(Message::PromptSearchFinished {
            request_id,
            result: Err("prompt archive is not configured".into()),
        });
    };
    let store = app.application.store().clone();
    let search = Search {
        limit: app.search_limit,
        paths: app.search_paths.clone(),
    };
    let needle = (!app.prompt_query.trim().is_empty()).then(|| app.prompt_query.clone());
    let lineage = (!app.prompt_lineage.trim().is_empty()).then(|| app.prompt_lineage.clone());
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                prompts::search(
                    &store,
                    &search,
                    Some(&archive),
                    needle.as_deref(),
                    lineage.as_deref(),
                )
            })
            .await
            .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
            .map_err(|error| error.to_string())
        },
        move |result| Message::PromptSearchFinished { request_id, result },
    )
}

pub fn pick_candidate(
    application: &Application,
    candidate: &Candidate,
    now: i64,
) -> crate::error::Result<String> {
    application.mutate(None, |store| crate::workflow::pick(store, candidate, now))
}

const LIMITS: [usize; 5] = [0, 25, 50, 75, 100];
const ROW_HEIGHT: f32 = 380.0;

pub(super) fn preview_tasks(app: &mut App) -> Task<Message> {
    let viewport = app.viewport(Tab::Pick);
    let visible = style::visible_range(
        viewport,
        style::columns(viewport.width),
        ROW_HEIGHT,
        app.candidates.len(),
    );
    let cache = app.application.store().paths().data_dir.join(".thumbs");
    let capacity = 4usize.saturating_sub(
        app.previews
            .values()
            .filter(|result| result.is_none())
            .count(),
    );
    let candidates: Vec<_> = app
        .candidates
        .iter()
        .skip(visible.start)
        .take(visible.len())
        .filter(|candidate| !app.previews.contains_key(&candidate.path))
        .take(capacity)
        .cloned()
        .collect();
    let mut tasks = Vec::new();
    for candidate in candidates {
        app.previews.insert(candidate.path.clone(), None);
        let source = candidate.path.clone();
        let preview_source = source.clone();
        let sha = candidate.sha512.clone();
        let cache = cache.clone();
        tasks.push(Task::perform(
            crate::ui::preview::prepare(source, sha, cache),
            move |result| Message::PreviewFinished {
                source: preview_source,
                result,
            },
        ));
    }
    Task::batch(tasks)
}

pub(super) fn pick_task(app: &mut App, candidate: Candidate) -> Task<Message> {
    if !app
        .candidates
        .iter()
        .any(|authorized| authorized == &candidate)
    {
        return Task::none();
    }
    app.busy = true;
    let application = app.application.clone();
    Task::perform(
        async move {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            tokio::task::spawn_blocking(move || pick_candidate(&application, &candidate, now))
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map(|_| ())
                .map_err(|error| error.to_string())
        },
        Message::PickFinished,
    )
}

pub fn delete_candidate(
    application: &Application,
    authorized: &[Candidate],
    candidate: &Candidate,
) -> crate::error::Result<()> {
    application.mutate(None, |store| {
        images::delete_candidate(store, authorized, &candidate.path)
    })
}

pub(super) fn delete_task(app: &mut App) -> Task<Message> {
    let Some(candidate) = app.pending_delete.take() else {
        return Task::none();
    };
    if !app
        .candidates
        .iter()
        .any(|authorized| authorized == &candidate)
    {
        return Task::none();
    }
    app.busy = true;
    let authorized = app.candidates.clone();
    let application = app.application.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                delete_candidate(&application, &authorized, &candidate)
            })
            .await
            .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
            .map_err(|error| error.to_string())
        },
        |result| Message::WorkflowFinished {
            message: "Deleted candidate".into(),
            result,
        },
    )
}

fn exact_prompt_heading_visible(lineage: &str, exact: &[Candidate]) -> bool {
    !lineage.trim().is_empty() && !exact.is_empty()
}

pub fn view(state: &App) -> Element<'_, Message> {
    let mut controls = column![
        row![
            text("Candidate limit"),
            pick_list(LIMITS, Some(state.search_limit), Message::SearchLimit).style(style::select),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text("Search paths"),
    ]
    .spacing(8);
    for configured in &state.application.store().config().publicable {
        let selected = state.search_paths.contains(configured);
        let path = configured.clone();
        controls = controls.push(
            checkbox(selected)
                .label(configured)
                .on_toggle(move |_| Message::SearchPath(path.clone())),
        );
    }
    if state.prompt_archive.is_some() {
        controls = controls
            .push(text("Exact prompt text (optional)"))
            .push(
                text_input("Search archived prompt text", &state.prompt_query)
                    .on_input(Message::PromptQueryChanged)
                    .style(style::input),
            )
            .push(text(
                "Prompt lineage path (optional; includes possible matches)",
            ))
            .push(
                text_input("e.g. prompts/lineage.txt", &state.prompt_lineage)
                    .on_input(Message::PromptLineageChanged)
                    .style(style::input),
            );
    }
    controls = controls.push(button("Search").on_press(Message::Search));
    let mut content = column![].spacing(16);
    if let Some(candidate) = &state.pending_delete {
        content = content.push(
            container(
                row![
                    text(format!("Delete {}?", candidate.path.display())),
                    button("Confirm delete").on_press(Message::ConfirmDelete),
                    button("Cancel").on_press(Message::CancelDelete)
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding(8)
            .width(Length::Fill)
            .style(style::notice),
        );
    }
    content = candidate_groups(state, content, controls.into());
    scrollable(content)
        .height(Length::Fill)
        .on_scroll(|viewport| Message::Viewport(Tab::Pick, viewport.into()))
        .into()
}

/// The search controls lead the first grid, as the web page's first grid cell.
// ponytail: lineage groups are separate grids; visible_range treats them as one list.
fn candidate_groups<'a>(
    state: &'a App,
    mut content: iced::widget::Column<'a, Message>,
    controls: Element<'a, Message>,
) -> iced::widget::Column<'a, Message> {
    let banner = |label: &'static str| {
        container(text(label))
            .padding(6)
            .width(Length::Fill)
            .style(style::notice)
    };
    let Some(matches) = &state.prompt_search else {
        let mut cards = vec![controls];
        cards.extend(state.candidates.iter().map(|c| candidate_card(state, c)));
        return content.push(style::cards(cards));
    };
    if exact_prompt_heading_visible(&state.prompt_lineage, &matches.exact) {
        content = content.push(banner("Exact archived prompt matches"));
    }
    let mut cards = vec![controls];
    cards.extend(matches.exact.iter().map(|c| candidate_card(state, c)));
    content = content.push(style::cards(cards));
    if !matches.maybe.is_empty() {
        content = content
            .push(banner(
                "Possible lineage matches — prompt may be from this archive",
            ))
            .push(style::cards(
                matches
                    .maybe
                    .iter()
                    .map(|c| candidate_card(state, c))
                    .collect(),
            ));
    }
    if matches.truncated {
        content = content.push(style::warning(
            "More matches exist; narrow the prompt text or lineage",
        ));
    }
    content
}

fn candidate_card<'a>(state: &'a App, candidate: &'a Candidate) -> Element<'a, Message> {
    let preview = state.previews.get(&candidate.path);
    let missing = matches!(preview, Some(Some(Err(_)))) && !candidate.path.exists();
    let mut content = column![
        style::art(preview, missing, 240.0),
        text(
            candidate
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Artwork")
        ),
    ]
    .spacing(8);
    if state.prompt_archive.is_some() || state.prompt_archive_error.is_some() {
        content = content.push(match state.prompt_match(&candidate.path) {
            Some(match_) => super::prompt::view(match_),
            None => text("Resolving prompt…").color(style::MUTED).into(),
        });
    }
    style::card(
        content
            .push(style::wide("Pick").on_press(Message::Pick(candidate.clone())))
            .push(style::wide("Delete…").on_press(Message::AskDelete(candidate.clone()))),
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn exact_prompt_heading_requires_lineage_filter_and_exact_results() {
        let exact = [Candidate {
            path: PathBuf::from("picked/art.png"),
            sha512: "digest".into(),
        }];
        assert!(exact_prompt_heading_visible("lineage/prompt.txt", &exact));
        assert!(!exact_prompt_heading_visible("", &exact));
        assert!(!exact_prompt_heading_visible("lineage/prompt.txt", &[]));
    }
}
