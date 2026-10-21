use super::{App, Message};
use crate::{
    metadata::{Source, VISION_MODELS},
    model::{GENERATE_MARKER, Phase, phase},
};
use iced::{
    Element, Length,
    widget::{button, column, row, scrollable, text, text_input},
};
use serde_json::Value;

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
        content = content.push(text(
            "Local model fit is checked before generation; installation requires confirmation.",
        ));
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
    content = content.push(scrollable(cards).height(Length::Fill));
    if state.pending_local_consent.is_some() {
        content = content.push(
            column![
                text("Install the missing local model?"),
                button("Confirm install and generate").on_press(Message::ConfirmLocalInstall),
                button("Cancel").on_press(Message::CancelLocalInstall),
            ]
            .spacing(8),
        );
    }
    content.into()
}
