use super::{App, Message};
use crate::{
    application::Application,
    images::{self, Candidate, Search},
    prompts,
};
use iced::{
    Element, Length, Task,
    widget::{button, checkbox, column, row, scrollable, text, text_input},
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

pub(super) fn search_task(app: &App) -> Task<Message> {
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
        Message::SearchFinished,
    )
}

pub(super) fn prompt_search_task(app: &App) -> Task<Message> {
    let Some(archive) = app.prompt_archive.clone() else {
        return Task::done(Message::PromptSearchFinished(Err(
            "prompt archive is not configured".into(),
        )));
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
        Message::PromptSearchFinished,
    )
}

pub fn pick_candidate(
    application: &Application,
    candidate: &Candidate,
    now: i64,
) -> crate::error::Result<String> {
    application.mutate(None, |store| crate::workflow::pick(store, candidate, now))
}

pub(super) fn preview_tasks(app: &mut App, start: usize, end: usize) -> Task<Message> {
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
        .skip(start)
        .take(end.saturating_sub(start))
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
    let mut controls = row![
        text("Limit"),
        button("0").on_press(Message::SearchLimit(0)),
        button("25").on_press(Message::SearchLimit(25)),
        button("50").on_press(Message::SearchLimit(50)),
        button("75").on_press(Message::SearchLimit(75)),
        button("100").on_press(Message::SearchLimit(100))
    ]
    .spacing(6);
    for configured in &state.application.store().config().publicable {
        let selected = state.search_paths.contains(configured);
        let path = configured.clone();
        controls = controls.push(
            checkbox(selected)
                .label(configured)
                .on_toggle(move |_| Message::SearchPath(path.clone())),
        );
    }
    let mut cards = column![].spacing(10);
    if let Some(matches) = &state.prompt_search {
        if exact_prompt_heading_visible(&state.prompt_lineage, &matches.exact) {
            cards = cards.push(text("Exact archived prompt matches"));
        }
        for candidate in &matches.exact {
            cards = cards.push(candidate_card(state, candidate));
        }
        if !matches.maybe.is_empty() {
            cards = cards.push(text(
                "Possible lineage matches — prompt may be from this archive",
            ));
            for candidate in &matches.maybe {
                cards = cards.push(candidate_card(state, candidate));
            }
        }
        if matches.truncated {
            cards = cards.push(text(
                "More matches exist; narrow the prompt text or lineage",
            ));
        }
    } else {
        for candidate in &state.candidates {
            cards = cards.push(candidate_card(state, candidate));
        }
    }
    if let Some(candidate) = &state.pending_delete {
        cards = cards.push(
            row![
                text(format!("Delete {}?", candidate.path.display())),
                button("Confirm delete").on_press(Message::ConfirmDelete),
                button("Cancel").on_press(Message::CancelDelete)
            ]
            .spacing(8),
        );
    }
    let mut content = column![controls].spacing(12);
    if state.prompt_archive.is_some() {
        content = content
            .push(text("Exact prompt text (optional)"))
            .push(
                text_input("Search archived prompt text", &state.prompt_query)
                    .on_input(Message::PromptQueryChanged),
            )
            .push(text(
                "Prompt lineage path (optional; includes possible matches)",
            ))
            .push(
                text_input("e.g. prompts/lineage.txt", &state.prompt_lineage)
                    .on_input(Message::PromptLineageChanged),
            );
    }
    content
        .push(button("Search").on_press(Message::Search))
        .push(
            scrollable(cards)
                .height(Length::Fill)
                .on_scroll(|viewport| Message::PreviewViewport {
                    offset: viewport.absolute_offset().y,
                    height: viewport.bounds().height,
                }),
        )
        .into()
}

fn candidate_card<'a>(state: &'a App, candidate: &'a Candidate) -> Element<'a, Message> {
    let candidate_pick = candidate.clone();
    let candidate_delete = candidate.clone();
    let preview: Element<'_, Message> = match state.previews.get(&candidate.path) {
        None | Some(None) => text("Loading preview…").into(),
        Some(Some(Ok(path))) => iced::widget::image(iced::widget::image::Handle::from_path(path))
            .width(Length::Fixed(180.0))
            .height(Length::Fixed(180.0))
            .into(),
        Some(Some(Err(_error))) if !candidate.path.exists() => text("Artwork missing").into(),
        Some(Some(Err(error))) => text(format!("Preview unavailable: {error}")).into(),
    };
    let mut content = column![
        row![
            preview,
            column![
                text(
                    candidate
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Artwork")
                ),
                button("Pick").on_press(Message::Pick(candidate_pick)),
                button("Delete…").on_press(Message::AskDelete(candidate_delete)),
            ]
            .spacing(6),
        ]
        .spacing(10)
    ]
    .spacing(6);
    if state.prompt_archive.is_some() || state.prompt_archive_error.is_some() {
        content = content.push(match state.prompt_match(&candidate.path) {
            Some(match_) => super::prompt::view(match_),
            None => text("Resolving prompt…").into(),
        });
    }
    content.into()
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
