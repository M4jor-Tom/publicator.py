use super::{App, BatchProgress, GenerationState, Message, style};
use crate::{
    metadata::{Source, VISION_MODELS},
    model::{GENERATE_MARKER, Phase, phase},
};
use iced::{
    Element, Length, Task,
    widget::{Column, button, column, container, row, text, text_input},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The Rows layout's thumbnail and action column widths.
const ROW_ART_WIDTH: f32 = 240.0;
const ROW_ACTIONS_WIDTH: f32 = 200.0;

/// Publication thumbnails for the on-screen placeholders of the Schedule, Describe and Publish
/// lists, which report row UUIDs (Pick's report candidate paths and match no row).
pub(super) fn preview_tasks(state: &mut App) -> Task<Message> {
    let Some(rows) = state.publications.as_ref() else {
        return Task::none();
    };
    let capacity = 4usize.saturating_sub(
        state
            .row_previews
            .values()
            .filter(|result| result.is_none())
            .count(),
    );
    let records: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let uuid = row.get("uuid")?.as_str()?;
            if !state.shown_previews.contains(uuid) || state.row_previews.contains_key(uuid) {
                return None;
            }
            Some((uuid.to_owned(), row.clone()))
        })
        .take(capacity)
        .collect();
    let root = state.application.store().paths().data_dir.clone();
    let cache = root.join(".thumbs");
    let mut tasks = Vec::new();
    for (uuid, row) in records {
        state.row_previews.insert(uuid.clone(), None);
        let task_uuid = uuid.clone();
        let root = root.clone();
        let cache = cache.clone();
        let index = state.artwork_index.clone();
        tasks.push(Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let index = super::artwork_index(&index, &root)?;
                    crate::artwork::preview_source(index, &row)
                        .and_then(|(source, sha512)| {
                            let thumb = crate::images::ensure_thumb(&source, &cache, &sha512)?;
                            Ok((source, thumb))
                        })
                        .map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(error.to_string()))
            },
            move |result| Message::RowPreviewFinished {
                uuid: task_uuid,
                result,
            },
        ));
    }
    Task::batch(tasks)
}

/// The provider, model and layout choices, Generate marked and its summary.
fn controls(state: &App) -> Column<'_, Message> {
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
    controls = controls.push(text("Layout"));
    for (layout, label) in [
        (DescribeLayout::Grid, "Grid"),
        (DescribeLayout::Rows, "Rows"),
    ] {
        controls = controls.push(
            style::wide(label)
                .style(style::tab(state.describe_layout == layout))
                .on_press(Message::SelectDescribeLayout(layout)),
        );
    }
    controls = controls
        .push(text(format!(
            "Selected: {:?} / {}",
            state.source, state.model.ollama_name
        )))
        .push(style::wide("Generate marked").on_press(Message::GenerateMarked));
    if let Some(summary) = batch_summary(state) {
        controls = controls.push(text(summary));
    }
    if state.source == Source::Local {
        controls = controls.push(local_fit_view(state));
    }
    controls
}

pub fn view(state: &App) -> Element<'_, Message> {
    let mut cards = vec![controls(state).into()];
    cards.extend(publication_cards(state));
    let list = match state.describe_layout {
        DescribeLayout::Grid => style::cards(cards),
        DescribeLayout::Rows => style::rows(cards),
    };
    let mut content = column![]
        .push(super::floating(
            super::tab_scrollable(super::Tab::Describe, list),
            super::schedule::delete_banner(state),
        ))
        .spacing(12);
    if state.local_install_consent_pending() {
        content = content.push(
            container(
                row![
                    text(format!("Install {} and generate?", state.model.ollama_name))
                        .width(Length::Fill),
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

/// The listed publications' cards, the current batch's first.
fn publication_cards(state: &App) -> Vec<Element<'_, Message>> {
    let generated = |row: &Value| {
        row.get("uuid")
            .and_then(Value::as_str)
            .and_then(|uuid| state.generation_state(uuid))
            == Some(&GenerationState::Generated)
    };
    let mut cards = Vec::new();
    for row in ordered_rows(
        state.publications.as_deref().unwrap_or_default(),
        state.batch_ids(),
        state.revealed_publication(),
        generated,
    ) {
        let Some(uuid) = row.get("uuid").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        cards.push(if phase(row).ok() == Some(Phase::Scheduled) {
            metadata_card(state, row, uuid)
        } else {
            generated_card(state, row, uuid)
        });
    }
    cards
}

/// The summary line under "Generate marked".
pub fn batch_summary(state: &App) -> Option<String> {
    Some(match state.batch.as_ref()? {
        BatchProgress::Running {
            ids, active: None, ..
        } => format!("Preparing {}…", ids.len()),
        BatchProgress::Running {
            ids,
            active: Some(active),
            done,
            failed,
        } => format!(
            "Generating {} of {} · {done} done · {failed} failed",
            active + 1,
            ids.len()
        ),
        BatchProgress::Finished {
            succeeded, failed, ..
        } => {
            let mut summary = format!("Generated {succeeded} of {}", succeeded + failed);
            if *failed > 0 {
                summary.push_str(&format!(" · {failed} failed"));
            }
            summary
        }
        BatchProgress::Failed(error) => format!("Generation failed: {error}"),
    })
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

/// A card's one generation state line; `Generating…` alone is boxed, so the active card
/// stands out.
fn generation_line<'a>(state: &App, uuid: &str, marked: bool) -> Option<Element<'a, Message>> {
    Some(match state.generation_state(uuid) {
        Some(GenerationState::Queued) => text("Queued").color(style::MUTED).into(),
        Some(GenerationState::Generating) => container(text("Generating…"))
            .padding([2, 8])
            .style(style::active)
            .into(),
        Some(GenerationState::Generated) => text("Generated").color(style::TODAY).into(),
        Some(GenerationState::Failed(error)) => text(format!("Failed: {error}"))
            .color(style::ERROR)
            .wrapping(text::Wrapping::WordOrGlyph)
            .into(),
        None if marked => style::warning("Marked for generation"),
        None => return None,
    })
}

/// A card in the current layout. Grid stacks the thumbnail, `details`, `actions` and `status`;
/// Rows puts a smaller thumbnail, `details` over `status`, and `actions` side by side.
fn layout_card<'a>(
    state: &'a App,
    uuid: &str,
    details: Column<'a, Message>,
    actions: Option<Column<'a, Message>>,
    status: Element<'a, Message>,
) -> Element<'a, Message> {
    let content: Element<'a, Message> = match state.describe_layout {
        DescribeLayout::Grid => column![state.row_art(uuid, 240.0), details]
            .push(actions.map(Element::from))
            .push(status)
            .spacing(8)
            .into(),
        DescribeLayout::Rows => {
            let actions = actions.map(|actions| actions.width(Length::Fixed(ROW_ACTIONS_WIDTH)));
            row![
                container(state.row_art(uuid, 160.0)).width(Length::Fixed(ROW_ART_WIDTH)),
                column![details, status].spacing(8).width(Length::Fill),
            ]
            .push(actions.map(Element::from))
            .spacing(16)
            .into()
        }
    };
    style::card(content).into()
}

/// A card whose generation moved it on to DESCRIBED: read-only, it stays listed until Refresh
/// so its `Generated` state remains in view; its actions belong to Publish now.
fn generated_card<'a>(state: &'a App, row: &Value, uuid: String) -> Element<'a, Message> {
    let saved = crate::workflow::Metadata::of(row);
    let mut details = column![text(super::schedule::date_label(state, row))].spacing(8);
    if let Some(line) = generation_line(state, &uuid, false) {
        details = details.push(line);
    }
    let details = details
        .push(text("Title"))
        .push(text(saved.title))
        .push(text("Description"))
        .push(text(saved.description));
    let status = text("Described; listed here until Refresh").color(style::MUTED);
    layout_card(state, &uuid, details, None, status.into())
}

fn metadata_card<'a>(state: &'a App, row: &Value, uuid: String) -> Element<'a, Message> {
    let saved = crate::workflow::Metadata::of(row);
    let (title, description) = state
        .drafts
        .get(&uuid)
        .map(|draft| (draft.title().to_owned(), draft.description().to_owned()))
        .unwrap_or((saved.title, saved.description));
    let title_uuid = uuid.clone();
    let mut card = column![text(super::schedule::date_label(state, row))].spacing(8);
    let marked = title.contains(GENERATE_MARKER) || description.contains(GENERATE_MARKER);
    if let Some(line) = generation_line(state, &uuid, marked) {
        card = card.push(line);
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
    if let Some(editor) = state.description_editors.get(&uuid)
        && state.viewer.is_some()
    {
        // tiny-skia paints an editor's overflowing last line unclipped, over the viewer; a
        // clipped text of the same height keeps the layout and scroll.
        card = card.push(
            container(text(editor.text()))
                .height(Length::Fixed(120.0))
                .clip(true),
        );
    } else if let Some(editor) = state.description_editors.get(&uuid) {
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
    let actions = column![
        style::wide("Generate one").on_press(Message::GenerateOne(uuid.clone())),
        style::wide("Approve metadata").on_press(Message::ApproveMetadata(uuid.clone())),
        style::wide("Return to Schedule").on_press(Message::ReturnToSchedule(uuid.clone())),
        style::wide("Save draft").on_press(Message::SaveDraft(uuid.clone())),
        super::schedule::delete_button(uuid.clone()),
    ]
    .spacing(8);
    layout_card(state, &uuid, card, Some(actions), style::warning(status))
}

/// SCHEDULED rows, plus those `kept` (cards whose generation just described them); the `batch`
/// ones first, in its order, the others in store order.
pub(super) fn ordered_rows<'a>(
    rows: &'a [Value],
    batch: &[String],
    target: Option<&str>,
    kept: impl Fn(&Value) -> bool,
) -> Vec<&'a Value> {
    let mut listed: Vec<_> = rows
        .iter()
        .filter(|row| phase(row).ok() == Some(Phase::Scheduled) || kept(row))
        .collect();
    // A stable sort: rows outside the batch keep their store order.
    listed.sort_by_key(|row| {
        row.get("uuid")
            .and_then(Value::as_str)
            .and_then(|uuid| batch.iter().position(|id| id == uuid))
            .unwrap_or(usize::MAX)
    });
    super::revealed_rows(listed.into_iter(), target)
}

/// The Describe card arrangement, kept in `<data dir>/.publicator/ui.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DescribeLayout {
    /// Cards in a responsive grid.
    #[default]
    Grid,
    /// One full-width card per row.
    Rows,
}

#[derive(Serialize, Deserialize)]
struct UiPreferences {
    describe_layout: DescribeLayout,
}

fn preferences_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(".publicator")
}

/// The saved layout; Grid when the file is missing, unreadable or invalid.
pub(super) fn load_layout(data_dir: &Path) -> DescribeLayout {
    std::fs::read(preferences_dir(data_dir).join("ui.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<UiPreferences>(&bytes).ok())
        .map(|preferences| preferences.describe_layout)
        .unwrap_or_default()
}

pub(super) fn save_layout(data_dir: &Path, layout: DescribeLayout) -> crate::error::Result<()> {
    let dir = preferences_dir(data_dir);
    crate::publish_journal::create_real_directory(
        &dir,
        "UI preference directory must be a real directory",
    )?;
    let bytes = serde_json::to_vec(&UiPreferences {
        describe_layout: layout,
    })?;
    crate::store::write_atomic(&dir.join("ui.json"), &bytes)
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
        let ordered = ordered_rows(&rows, &[], Some("target"), |_| false);
        assert_eq!(ordered[0]["uuid"], "target");
        assert_eq!(rows[0]["uuid"], "first");
    }

    #[test]
    fn only_kept_described_rows_join_the_scheduled_ones() {
        let rows = vec![
            serde_json::json!({"uuid":"scheduled", "state":"SCHEDULED"}),
            serde_json::json!({"uuid":"generated", "state":"DESCRIBED"}),
            serde_json::json!({"uuid":"other", "state":"DESCRIBED"}),
        ];
        let ordered = ordered_rows(&rows, &[], None, |row| row["uuid"] == "generated");
        let uuids: Vec<_> = ordered.iter().map(|row| row["uuid"].clone()).collect();
        assert_eq!(uuids, ["scheduled", "generated"]);
    }

    fn uuids(ordered: &[&Value]) -> Vec<String> {
        ordered
            .iter()
            .map(|row| row["uuid"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn batch_rows_come_first_in_batch_order_and_the_rest_keep_store_order() {
        let rows: Vec<_> = ["a", "b", "c", "d", "e"]
            .map(|uuid| serde_json::json!({"uuid": uuid, "state": "SCHEDULED"}))
            .into();
        let batch = ["d".to_owned(), "b".to_owned()];
        assert_eq!(
            uuids(&ordered_rows(&rows, &batch, None, |_| false)),
            ["d", "b", "a", "c", "e"]
        );
        // The Calendar-revealed row still swaps to the top.
        assert_eq!(
            uuids(&ordered_rows(&rows, &batch, Some("e"), |_| false)),
            ["e", "b", "a", "c", "d"]
        );
    }

    #[test]
    fn layout_preference_round_trips_and_falls_back_to_grid() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_layout(dir.path()),
            DescribeLayout::Grid,
            "missing file"
        );
        save_layout(dir.path(), DescribeLayout::Rows).unwrap();
        let file = dir.path().join(".publicator/ui.json");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            r#"{"describe_layout":"rows"}"#
        );
        assert_eq!(load_layout(dir.path()), DescribeLayout::Rows);
        for invalid in ["not json", r#"{"describe_layout":"columns"}"#, "{}"] {
            std::fs::write(&file, invalid).unwrap();
            assert_eq!(load_layout(dir.path()), DescribeLayout::Grid, "{invalid}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn layout_preference_refuses_a_symlinked_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(".publicator")).unwrap();
        assert!(save_layout(dir.path(), DescribeLayout::Rows).is_err());
        assert!(!elsewhere.path().join("ui.json").exists());
    }
}
