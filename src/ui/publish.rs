use super::{App, Message, Tab, style};
use crate::{
    model::{Phase, phase},
    publish_journal::{AttemptState, AttemptStatus, ReconcileOutcome},
    publishing::{PublishCellOutcome, PublishCellState},
};
use iced::{
    Element, Length, Task,
    widget::{button, checkbox, column, container, row, scrollable, table, text, text_input},
};
use serde_json::Value;

const SCROLL_ID: &str = "publish-publications";
/// Estimated table row height, for lazy thumbnails.
pub(super) const ROW_HEIGHT: f32 = 200.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionState {
    None,
    Partial,
    All,
}

pub fn selection_state(selected: &[bool]) -> SelectionState {
    match (
        selected.iter().any(|value| *value),
        selected.iter().all(|value| *value),
    ) {
        (false, _) => SelectionState::None,
        (true, true) => SelectionState::All,
        (true, false) => SelectionState::Partial,
    }
}

pub fn view(state: &App) -> Element<'_, Message> {
    let mut content = column![
        text("Cell keyboard: F6 enter/exit · Tab/Shift+Tab move · Space toggle · Esc exit")
            .color(style::MUTED),
    ]
    .spacing(10);
    if state.publish_keyboard_mode {
        let focus = state
            .publish_keyboard_focus
            .as_deref()
            .map(|uuid| format!("Keyboard focus: {uuid} — DeviantArt"))
            .unwrap_or_else(|| "No unlocked pending DeviantArt cell to focus".into());
        content = content.push(text(focus));
    }
    if let Some(error) = &state.publish_lock_error {
        content =
            content.push(text(format!("Publish status unavailable: {error}")).color(style::ERROR));
    }

    let rows: Vec<&Value> = state
        .publications
        .as_deref()
        .map(|rows| ordered_rows(rows, state.revealed_publication()))
        .unwrap_or_default()
        .into_iter()
        .filter(|publication| !matches!(delivery(publication), Delivery::Other))
        .collect();
    let selections: Vec<bool> = rows
        .iter()
        .filter_map(|publication| match delivery(publication) {
            Delivery::Pending(da) => Some(da.is_some()),
            _ => None,
        })
        .collect();
    let mut list = column![
        container(
            table(
                [
                    table::column(text("Publication").font(style::BOLD), |publication| {
                        summary(state, publication)
                    })
                    .width(Length::Fill),
                    table::column(text("DeviantArt").font(style::BOLD), |publication| {
                        platform_cell(state, publication)
                    })
                    .width(Length::Fill),
                ],
                rows,
            )
            .width(Length::Fill)
            .padding(12)
            .separator(1),
        )
        .style(style::cell)
    ]
    .spacing(12);
    for attempt in state.unresolved_publish_attempts() {
        list = list.push(
            container(attempt_view(state, attempt))
                .padding(12)
                .width(Length::Fill)
                .style(style::notice),
        );
    }
    content = publish_actions(state, content, &selections);
    content
        .push(
            scrollable(list)
                .id(SCROLL_ID)
                .height(Length::Fill)
                .on_scroll(|viewport| Message::Viewport(Tab::Publish, viewport.into())),
        )
        .into()
}

fn publish_actions<'a>(
    state: &'a App,
    mut content: iced::widget::Column<'a, Message>,
    selections: &[bool],
) -> iced::widget::Column<'a, Message> {
    let selection = selection_state(selections);
    let all = selection == SelectionState::All && !selections.is_empty();
    let select_all = checkbox(all).label(match selection {
        SelectionState::None => "Select all eligible pending DeviantArt cells",
        SelectionState::Partial => "Some DeviantArt cells selected",
        SelectionState::All => "All eligible pending DeviantArt cells selected",
    });
    content = content.push(if state.selection_locked(None) {
        select_all
    } else {
        select_all.on_toggle(Message::SelectAll)
    });
    let publish = button("Publish selected");
    let publish = if publish_disabled(state, selections) {
        publish
    } else {
        publish.on_press(Message::PublishSelected)
    };
    let mut actions = row![publish].spacing(8);
    if state.publish_active {
        actions =
            actions.push(button("Cancel after current").on_press(Message::CancelAfterCurrent));
    }
    content = content.push(actions);
    if let Some(result) = &state.publish_result {
        content = content.push(text(result));
    }
    if let Some(diagnostic) = &state.publish_diagnostic {
        content = content
            .push(text(format!("Worker diagnostic (stderr): {diagnostic}")).color(style::ERROR));
    }
    content
}

fn publish_disabled(state: &App, selections: &[bool]) -> bool {
    state.publish_active
        || state.selection_locked(None)
        || !selections.iter().any(|selected| *selected)
}

enum Delivery<'a> {
    Locked,
    Pending(Option<&'a Value>),
    Other,
}

/// The DeviantArt apparition's delivery state; other states have no publish cell.
fn delivery(publication: &Value) -> Delivery<'_> {
    let da = publication
        .get("apparitions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|apparition| {
            apparition.get("platformName").and_then(Value::as_str) == Some("deviantart")
        });
    match da.and_then(|apparition| apparition.get("state").and_then(Value::as_str)) {
        Some("published_or_scheduled") => Delivery::Locked,
        Some("unpublished") | None => Delivery::Pending(da),
        Some(_) => Delivery::Other,
    }
}

fn summary<'a>(state: &'a App, publication: &'a Value) -> Element<'a, Message> {
    let uuid = publication
        .get("uuid")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = publication
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    column![
        style::art(state.row_previews.get(uuid), false, 90.0),
        text(title).font(style::BOLD),
        text(super::schedule::date_label(state, publication)).font(style::BOLD),
    ]
    .spacing(4)
    .into()
}

fn platform_cell<'a>(state: &'a App, publication: &'a Value) -> Element<'a, Message> {
    let uuid = publication
        .get("uuid")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let label = format!("{uuid} — DeviantArt");
    let mut cell = column![].spacing(6);
    if state.publish_keyboard_mode && state.publish_keyboard_focus.as_deref() == Some(uuid) {
        cell = cell.push(text(format!("Keyboard focus → {label}")).color(style::ACCENT));
    }
    if state.revealed_publication() == Some(uuid) {
        cell = cell.push(style::warning(format!("Opened from Calendar — {label}")));
    }
    let outcome = state.publish_cell_outcome(uuid);
    let Delivery::Pending(da) = delivery(publication) else {
        let locked = outcome_text(outcome.as_ref()).unwrap_or_else(|| "Published (locked)".into());
        return cell.push(checkbox(true).label(locked)).into();
    };
    let checked = da.is_some();
    let key = format!("{uuid}:deviantart");
    let selection = checkbox(checked).label(if checked { "Selected" } else { "Select" });
    cell = cell.push(if state.selection_locked(Some(uuid)) {
        selection
    } else {
        selection.on_toggle(move |selected| Message::SelectCell {
            key: key.clone(),
            selected,
        })
    });
    if let Some(outcome) = &outcome {
        cell = cell.push(text(outcome_text(Some(outcome)).unwrap()));
    }
    if let Some(apparition) = da {
        cell = cell.push(options_view(state, uuid, apparition));
    }
    cell.into()
}

pub(super) fn ordered_rows<'a>(rows: &'a [Value], target: Option<&str>) -> Vec<&'a Value> {
    super::revealed_rows(
        rows.iter()
            .filter(|row| matches!(phase(row), Ok(Phase::Described | Phase::Appeared))),
        target,
    )
}

pub(super) fn scroll_to_target() -> Task<Message> {
    super::scroll_to_publication(SCROLL_ID)
}

fn attempt_view<'a>(state: &'a App, attempt: &'a AttemptStatus) -> Element<'a, Message> {
    let mut panel = column![
        text(format!(
            "Unresolved attempt {} — {}",
            attempt.attempt_id, attempt.uuid
        )),
        text(attempt.reason()),
    ]
    .spacing(4);
    if !attempt.database_matches {
        panel = panel.push(text("This receipt belongs to a different publication database and cannot be reconciled here."));
        return panel.into();
    }
    if attempt.current_matches == Some(false) {
        panel = panel
            .push(text(approved_summary(attempt)))
            .push(text(current_summary(state, &attempt.uuid)));
    }
    if let Some(error) = &attempt.current_error {
        panel = panel.push(text(format!(
            "Current snapshot could not be verified: {error}"
        )));
    }
    let attempt_id = attempt.attempt_id.clone();
    let value = state
        .reconcile_urls
        .get(&attempt.attempt_id)
        .map(String::as_str)
        .unwrap_or("");
    panel = panel.push(
        text_input("Actual HTTP(S) URL (optional)", value)
            .on_input(move |value| Message::ReconcileUrlChanged {
                attempt_id: attempt_id.clone(),
                value,
            })
            .style(style::input),
    );
    let actual_url = if value.trim().is_empty() {
        None
    } else {
        Some(value.trim().to_owned())
    };
    let scheduled_id = attempt.attempt_id.clone();
    let mut actions =
        row![
            button("I verified this was scheduled").on_press(Message::ReconcileAttempt {
                attempt_id: scheduled_id,
                outcome: ReconcileOutcome::Scheduled { actual_url },
            })
        ]
        .spacing(8);
    if matches!(
        &attempt.state,
        AttemptState::Pending | AttemptState::Unknown { .. }
    ) {
        actions = actions.push(button("Confirm not submitted").on_press(
            Message::ReconcileAttempt {
                attempt_id: attempt.attempt_id.clone(),
                outcome: ReconcileOutcome::NotSubmitted,
            },
        ));
    }
    panel = panel.push(actions);
    panel.into()
}

fn approved_summary(attempt: &AttemptStatus) -> String {
    let approved = &attempt.approved;
    format!(
        "Approved snapshot: title={:?}; description={:?}; schedule={}; price={:?}; tiers={:?}; galleries={:?}",
        approved.title,
        approved.description,
        approved.schedule_epoch,
        approved.price,
        approved.tiers,
        approved.galleries
    )
}

fn current_summary(state: &App, uuid: &str) -> String {
    let current = state
        .publications
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid));
    let Some(row) = current else {
        return "Current publication is missing".into();
    };
    let apparition = row
        .get("apparitions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|app| app.get("platformName").and_then(Value::as_str) == Some("deviantart"));
    format!(
        "Current publication: title={:?}; description={:?}; schedule={:?}; price={:?}; tiers={:?}; galleries={:?}",
        row.get("title").and_then(Value::as_str),
        row.get("description").and_then(Value::as_str),
        row.get("apparitionTimestampIfDifferentThanSubmission")
            .and_then(Value::as_i64)
            .or_else(|| row.get("submissionTimestamp").and_then(Value::as_i64)),
        apparition
            .and_then(|app| app.get("priceIfNotFree"))
            .and_then(Value::as_f64),
        apparition.and_then(|app| app.get("tiers")),
        apparition.and_then(|app| app.get("galleries"))
    )
}

fn outcome_text(outcome: Option<&PublishCellOutcome>) -> Option<String> {
    outcome.map(|outcome| {
        let retry = if outcome.retryable() {
            "retry available"
        } else {
            "retry blocked"
        };
        let status = match &outcome.state {
            PublishCellState::Success => "Published",
            PublishCellState::SafeFailed => "Safe failure",
            PublishCellState::Unknown => "Unknown; reconcile first",
            PublishCellState::Unattempted => "Unattempted",
            PublishCellState::ConfirmedNotSubmitted => "Confirmed not submitted",
        };
        format!(
            "{status} ({retry}){}",
            outcome
                .detail
                .as_deref()
                .map(|detail| format!(": {detail}"))
                .unwrap_or_default()
        )
    })
}

fn options_view<'a>(state: &'a App, uuid: &'a str, apparition: &'a Value) -> Element<'a, Message> {
    let draft = state.publish_options.get(uuid);
    let price = draft
        .map(|options| options.price.clone())
        .or_else(|| {
            apparition
                .get("priceIfNotFree")
                .and_then(Value::as_f64)
                .map(|price| price.to_string())
        })
        .unwrap_or_default();
    let locked = state.options_locked(uuid);
    let mut panel = column![text("DeviantArt options")].spacing(4);
    let price_input = text_input("Price (blank clears/free)", &price).style(style::input);
    panel = panel.push(if locked {
        price_input
    } else {
        let uuid = uuid.to_owned();
        price_input.on_input(move |value| Message::OptionPriceChanged {
            uuid: uuid.clone(),
            value,
        })
    });
    for tier in &state.application.store().config().tiers {
        let selected = draft
            .map(|options| options.tiers.contains(tier))
            .unwrap_or_else(|| contains_option(apparition, "tiers", tier));
        let control = checkbox(selected).label(tier.as_str());
        let uuid = uuid.to_owned();
        let tier = tier.clone();
        panel = panel.push(
            control.on_toggle_maybe((!locked).then_some(move |selected| {
                Message::OptionTierChanged {
                    uuid: uuid.clone(),
                    value: tier.clone(),
                    selected,
                }
            })),
        );
    }
    for gallery in &state.application.store().config().galleries {
        let selected = draft
            .map(|options| options.galleries.contains(gallery))
            .unwrap_or_else(|| contains_option(apparition, "galleries", gallery));
        let control = checkbox(selected).label(gallery.as_str());
        let uuid = uuid.to_owned();
        let gallery = gallery.clone();
        panel = panel.push(
            control.on_toggle_maybe((!locked).then_some(move |selected| {
                Message::OptionGalleryChanged {
                    uuid: uuid.clone(),
                    value: gallery.clone(),
                    selected,
                }
            })),
        );
    }
    if let Some(error) = draft
        .and_then(|options| options.error.as_deref())
        .or(state.publish_lock_error.as_deref())
    {
        panel = panel.push(text(error));
    }
    if draft.is_some_and(|options| options.dirty) {
        let save = button("Save options");
        panel = panel.push(if locked {
            save
        } else {
            save.on_press(Message::SaveOptions(uuid.to_owned()))
        });
        if state.publish_locks.is_none() {
            panel = panel.push(text("Checking unresolved attempts; options are locked."));
        }
    }
    panel.into()
}

fn contains_option(apparition: &Value, key: &str, selected: &str) -> bool {
    apparition
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(selected)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revealed_publish_publication_is_first_in_the_scroll_list() {
        let rows = vec![
            serde_json::json!({"uuid":"first", "state":"DESCRIBED"}),
            serde_json::json!({"uuid":"target", "state":"APPEARED"}),
        ];
        let ordered = ordered_rows(&rows, Some("target"));
        assert_eq!(ordered[0]["uuid"], "target");
    }
}
