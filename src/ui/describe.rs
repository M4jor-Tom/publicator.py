use super::{App, Message};
use crate::{
    metadata::{Source, VISION_MODELS},
    model::{GENERATE_MARKER, Phase, phase},
};
use iced::{
    Element, Length, Task,
    widget::{button, column, row, scrollable, text, text_input},
};
use serde_json::Value;

pub(super) fn preview_tasks(state: &mut App, start: usize, end: usize) -> Task<Message> {
    let Some(rows) = state.publications.as_ref() else {
        return Task::none();
    };
    let capacity = 4usize.saturating_sub(
        state
            .describe_previews
            .values()
            .filter(|result| result.is_none())
            .count(),
    );
    let records: Vec<_> = rows
        .iter()
        .filter(|row| phase(row).ok() == Some(Phase::Scheduled))
        .skip(start)
        .take(end.saturating_sub(start))
        .filter_map(|row| {
            let uuid = row.get("uuid")?.as_str()?.to_owned();
            if state.describe_previews.contains_key(&uuid) {
                return None;
            }
            let basename = row
                .get("files")?
                .as_array()?
                .first()?
                .get("basename")?
                .as_str()?
                .to_owned();
            let sha = row
                .get("files")?
                .as_array()?
                .first()?
                .get("sha512sum")
                .or_else(|| row.get("files")?.as_array()?.first()?.get("sha512sumHex"))?
                .as_str()?
                .to_owned();
            Some((uuid, basename, sha))
        })
        .take(capacity)
        .collect();
    let root = state.application.store().paths().data_dir.clone();
    let cache = root.join(".thumbs");
    let mut tasks = Vec::new();
    for (uuid, basename, sha) in records {
        state.describe_previews.insert(uuid.clone(), None);
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
            move |result| Message::DescribePreviewFinished {
                uuid: task_uuid,
                result,
            },
        ));
    }
    Task::batch(tasks)
}

pub fn view(state: &App) -> Element<'_, Message> {
    let mut source_controls = row![text("Provider")].spacing(8);
    for source in crate::metadata::available_sources() {
        let label = match source {
            Source::OpenRouter => "OpenRouter",
            Source::Local => "Local Ollama",
        };
        source_controls =
            source_controls.push(button(label).on_press(Message::SelectSource(*source)));
    }
    let mut model_controls = row![text("Model")].spacing(8);
    for model in VISION_MODELS.iter() {
        model_controls = model_controls.push(
            button(text(model.ollama_name.clone())).on_press(Message::SelectModel(model.clone())),
        );
    }
    let mut content = column![
        source_controls,
        model_controls,
        text(format!(
            "Selected: {:?} / {}",
            state.source, state.model.ollama_name
        )),
        button("Generate marked").on_press(Message::GenerateMarked),
    ]
    .spacing(12);
    if state.source == Source::Local {
        let fit = state.local_fit_status();
        if let Some(status) = fit {
            let reason: String = status.fit.reason.chars().take(512).collect();
            content = content.push(text(format!(
                "Local fit: {} — {}. {}",
                status.fit.level,
                if reason.is_empty() {
                    "No additional fit notes"
                } else {
                    &reason
                },
                if !status.fit.fit {
                    "This model does not fit this host."
                } else if !status.available {
                    "Ollama is unavailable; generation cannot continue."
                } else if status.installed {
                    "Model is installed."
                } else {
                    "Model is not installed; installation requires confirmation."
                }
            )));
        } else {
            content = content.push(text("Checking local model fit…"));
        }
    }
    let mut cards = column![].spacing(12);
    for row in state.publications.as_deref().unwrap_or_default() {
        if phase(row).ok() != Some(Phase::Scheduled) {
            continue;
        }
        let Some(uuid) = row.get("uuid").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let (saved_title, saved_description) = super::row_metadata(row);
        let (title, description) = state
            .drafts
            .get(&uuid)
            .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
            .unwrap_or((saved_title, saved_description));
        let busy = state.busy_uuids.contains(&uuid);
        let title_uuid = uuid.clone();
        let description_uuid = uuid.clone();
        let mut card = column![
            text(uuid.clone()),
            text_input("Title", &title)
                .on_input(move |value| Message::Edited {
                    uuid: title_uuid.clone(),
                    field: super::drafts::Field::Title,
                    text: value
                })
                .on_submit(Message::SaveDraft(uuid.clone())),
            text_input("Description", &description)
                .on_input(move |value| Message::Edited {
                    uuid: description_uuid.clone(),
                    field: super::drafts::Field::Description,
                    text: value
                })
                .on_submit(Message::SaveDraft(uuid.clone())),
        ]
        .spacing(5);
        let dirty_title = state
            .drafts
            .get(&uuid)
            .is_some_and(|d| d.is_field_dirty(super::drafts::Field::Title));
        let dirty_description = state
            .drafts
            .get(&uuid)
            .is_some_and(|d| d.is_field_dirty(super::drafts::Field::Description));
        let preview: Element<'_, Message> = match state.describe_previews.get(&uuid) {
            Some(Some(Ok(path))) => {
                iced::widget::image(iced::widget::image::Handle::from_path(path))
                    .width(Length::Fixed(160.0))
                    .height(Length::Fixed(160.0))
                    .into()
            }
            Some(Some(Err(error))) => text(format!("Preview unavailable: {error}")).into(),
            _ => text("Preview loads when this card is visible").into(),
        };
        card = card.push(preview);
        card = card.push(text(format!(
            "Title: {}; description: {}",
            if dirty_title { "unsaved" } else { "saved" },
            if dirty_description {
                "unsaved"
            } else {
                "saved"
            }
        )));
        card = card.push(
            row![
                button("Save draft").on_press(Message::SaveDraft(uuid.clone())),
                button("Approve metadata").on_press(Message::ApproveMetadata(uuid.clone())),
                button("Return to Schedule").on_press(Message::ReturnToSchedule(uuid.clone())),
                button("Generate one").on_press(Message::GenerateOne(uuid.clone())),
            ]
            .spacing(6),
        );
        if busy {
            card = card.push(text("Generating and saving…"));
        } else if title.contains(GENERATE_MARKER) || description.contains(GENERATE_MARKER) {
            card = card.push(text("Marked for generation"));
        }
        cards = cards.push(card);
    }
    content = content.push(
        scrollable(cards)
            .height(Length::Fill)
            .on_scroll(|viewport| Message::DescribeViewport {
                offset: viewport.absolute_offset().y,
                height: viewport.bounds().height,
            }),
    );
    if state.local_install_consent_pending() {
        content = content.push(
            column![
                text(format!("Install {} and generate?", state.model.ollama_name)),
                button("Confirm install and generate").on_press(Message::ConfirmLocalInstall),
                button("Cancel").on_press(Message::CancelLocalInstall),
            ]
            .spacing(8),
        );
    }
    content.into()
}
