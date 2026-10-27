use super::{App, Message, Tab, style};
use crate::{
    metadata::{Source, VISION_MODELS},
    model::{GENERATE_MARKER, Phase, phase},
};
use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, scrollable, text, text_input},
};
use serde_json::Value;

const SCROLL_ID: &str = "describe-publications";

const ROW_HEIGHT: f32 = 560.0;

/// Publication thumbnails for the visible rows of the Schedule, Describe or Publish list;
/// other tabs have no publication rows.
pub(super) fn preview_tasks(state: &mut App, tab: Tab) -> Task<Message> {
    let Some(rows) = state.publications.as_ref() else {
        return Task::none();
    };
    let target = state.revealed_publication();
    let viewport = state.viewport(tab);
    let (rows, columns, row_height) = match tab {
        Tab::Schedule => (
            super::schedule::ordered_rows(rows, target),
            style::columns(viewport.width),
            super::schedule::ROW_HEIGHT,
        ),
        Tab::Publish => (
            super::publish::ordered_rows(rows, target),
            1,
            super::publish::ROW_HEIGHT,
        ),
        Tab::Describe => (
            ordered_rows(rows, target),
            style::columns(viewport.width),
            ROW_HEIGHT,
        ),
        Tab::Pick | Tab::Calendar => return Task::none(),
    };
    let visible = style::visible_range(viewport, columns, row_height, rows.len());
    let capacity = 4usize.saturating_sub(
        state
            .row_previews
            .values()
            .filter(|result| result.is_none())
            .count(),
    );
    let records: Vec<_> = rows
        .into_iter()
        .skip(visible.start)
        .take(visible.len())
        .filter_map(|row| {
            let uuid = row.get("uuid")?.as_str()?.to_owned();
            if state.row_previews.contains_key(&uuid) {
                return None;
            }
            let (basename, sha) = preview_identity(row)?;
            Some((uuid, basename, sha))
        })
        .take(capacity)
        .collect();
    let root = state.application.store().paths().data_dir.clone();
    let cache = root.join(".thumbs");
    let mut tasks = Vec::new();
    for (uuid, basename, sha) in records {
        state.row_previews.insert(uuid.clone(), None);
        let task_uuid = uuid.clone();
        let root = root.clone();
        let cache = cache.clone();
        tasks.push(Task::perform(
            async move {
                let source = tokio::task::spawn_blocking(move || {
                    crate::images::index_by_basename(&root)?
                        .remove(&basename)
                        .ok_or_else(|| crate::error::Error::NotFound(format!("artwork {basename}")))
                })
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())?;
                crate::ui::preview::prepare(source, sha, cache).await
            },
            move |result| Message::RowPreviewFinished {
                uuid: task_uuid,
                result,
            },
        ));
    }
    Task::batch(tasks)
}

fn preview_identity(row: &Value) -> Option<(String, String)> {
    let file = row.get("files")?.as_array()?.first()?;
    let basename = file.get("basename")?.as_str()?.to_owned();
    let sha = file
        .get("sha512sum")
        .or_else(|| file.get("sha512sumHex"))?
        .as_str()?
        .to_owned();
    Some((basename, sha))
}

pub fn view(state: &App) -> Element<'_, Message> {
    let mut controls = column![text("AI provider")].spacing(8);
    for source in crate::metadata::available_sources() {
        let label = match source {
            Source::OpenRouter => "OpenRouter",
            Source::Local => "Local Ollama",
        };
        controls = controls.push(
            style::wide(label)
                .style(style::tab(state.source == *source))
                .on_press(Message::SelectSource(*source)),
        );
    }
    controls = controls.push(text("Model"));
    for model in VISION_MODELS.iter() {
        controls = controls.push(
            style::wide(model.ollama_name.clone())
                .style(style::tab(state.model == *model))
                .on_press(Message::SelectModel(model.clone())),
        );
    }
    controls = controls
        .push(text(format!(
            "Selected: {:?} / {}",
            state.source, state.model.ollama_name
        )))
        .push(style::wide("Generate marked").on_press(Message::GenerateMarked));
    if state.source == Source::Local {
        controls = controls.push(local_fit_view(state));
    }
    let mut cards = vec![controls.into()];
    for row in ordered_rows(
        state.publications.as_deref().unwrap_or_default(),
        state.revealed_publication(),
    ) {
        let Some(uuid) = row.get("uuid").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        cards.push(metadata_card(state, row, uuid));
    }
    let mut content = column![
        scrollable(style::cards(cards))
            .id(SCROLL_ID)
            .height(Length::Fill)
            .on_scroll(|viewport| Message::Viewport(Tab::Describe, viewport.into()))
    ]
    .spacing(12);
    if state.local_install_consent_pending() {
        content = content.push(
            container(
                row![
                    text(format!("Install {} and generate?", state.model.ollama_name)),
                    button("Confirm install and generate").on_press(Message::ConfirmLocalInstall),
                    button("Cancel").on_press(Message::CancelLocalInstall),
                ]
                .spacing(8),
            )
            .padding(8)
            .width(Length::Fill)
            .style(style::notice),
        );
    }
    content.into()
}

fn local_fit_view(state: &App) -> Element<'_, Message> {
    let Some(status) = state.local_fit_status() else {
        return text("Checking local model fit…").into();
    };
    let reason: String = status.fit.reason.chars().take(512).collect();
    let availability = if !status.fit.fit {
        "This model does not fit this host."
    } else if !status.available {
        "Ollama is unavailable; generation cannot continue."
    } else if status.installed {
        "Model is installed."
    } else {
        "Model is not installed; installation requires confirmation."
    };
    text(format!(
        "Local fit: {} — {}. {}",
        status.fit.level,
        if reason.is_empty() {
            "No additional fit notes"
        } else {
            &reason
        },
        availability
    ))
    .into()
}

fn metadata_card<'a>(state: &'a App, row: &Value, uuid: String) -> Element<'a, Message> {
    let (saved_title, saved_description) = super::row_metadata(row);
    let (title, description) = state
        .drafts
        .get(&uuid)
        .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
        .unwrap_or((saved_title, saved_description));
    let busy = state.busy_uuids.contains(&uuid);
    let title_uuid = uuid.clone();
    let mut card = column![
        style::art(state.row_previews.get(&uuid), false, 240.0),
        text(super::schedule::date_label(state, row)),
    ]
    .spacing(8);
    if busy {
        card = card.push(style::warning("Generating and saving…"));
    } else if title.contains(GENERATE_MARKER) || description.contains(GENERATE_MARKER) {
        card = card.push(style::warning("Marked for generation"));
    }
    if state.revealed_publication() == Some(uuid.as_str()) {
        card = card.push(style::warning("Opened from Calendar"));
    }
    card = card.push(text("Title")).push(
        text_input("Title", &title)
            .on_input(move |value| Message::Edited {
                uuid: title_uuid.clone(),
                field: super::drafts::Field::Title,
                text: value,
            })
            .on_submit(Message::SaveDraft(uuid.clone()))
            .style(style::input),
    );
    card = card.push(text("Description"));
    if let Some(editor) = state.description_editors.get(&uuid) {
        let description_uuid = uuid.clone();
        card = card.push(
            iced::widget::text_editor(editor)
                .placeholder("Description")
                .height(Length::Fixed(120.0))
                .on_action(move |action| Message::DescriptionAction {
                    uuid: description_uuid.clone(),
                    action,
                })
                .style(style::editor),
        );
    } else {
        card = card.push(text(description.clone()));
    }
    let dirty = |field| {
        state
            .drafts
            .get(&uuid)
            .is_some_and(|d| d.is_field_dirty(field))
    };
    let saved = |field| if dirty(field) { "unsaved" } else { "saved" };
    let status = format!(
        "Title: {}; description: {}",
        saved(super::drafts::Field::Title),
        saved(super::drafts::Field::Description)
    );
    style::card(
        card.push(style::wide("Generate one").on_press(Message::GenerateOne(uuid.clone())))
            .push(style::wide("Approve metadata").on_press(Message::ApproveMetadata(uuid.clone())))
            .push(
                style::wide("Return to Schedule").on_press(Message::ReturnToSchedule(uuid.clone())),
            )
            .push(style::wide("Save draft").on_press(Message::SaveDraft(uuid)))
            .push(style::warning(status)),
    )
    .into()
}

pub(super) fn ordered_rows<'a>(rows: &'a [Value], target: Option<&str>) -> Vec<&'a Value> {
    super::revealed_rows(
        rows.iter()
            .filter(|row| phase(row).ok() == Some(Phase::Scheduled)),
        target,
    )
}

pub(super) fn scroll_to_target() -> Task<Message> {
    super::scroll_to_publication(SCROLL_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revealed_scheduled_publication_is_first_without_reordering_saved_rows() {
        let rows = vec![
            serde_json::json!({"uuid":"first", "state":"SCHEDULED"}),
            serde_json::json!({"uuid":"target", "state":"SCHEDULED"}),
        ];
        let ordered = ordered_rows(&rows, Some("target"));
        assert_eq!(ordered[0]["uuid"], "target");
        assert_eq!(rows[0]["uuid"], "first");
    }
}
