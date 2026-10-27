use super::{App, Message, Tab, style};
use crate::{
    application::Application,
    error::Result,
    model::{Phase, effective_schedule, phase},
    scheduling::{profiles, timezone},
    workflow::{self, DateEdit},
};
use chrono::{DateTime, Utc};
use iced::{
    Alignment, Element, Length, Task,
    widget::{column, pick_list, row, scrollable, text, text_input},
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

const SCROLL_ID: &str = "schedule-publications";
/// Estimated card row height, for lazy thumbnails.
pub(super) const ROW_HEIGHT: f32 = 440.0;

pub fn edit_date(application: &Application, uuid: &str, edit: DateEdit, now: i64) -> Result<()> {
    application.mutate(Some(uuid), |store| {
        workflow::edit_date(store, uuid, edit, now)
    })
}

pub(super) fn date_task(app: &mut App, uuid: String, edit: DateEdit) -> Task<Message> {
    let application = app.application.clone();
    app.busy = true;
    Task::perform(
        async move {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            tokio::task::spawn_blocking(move || edit_date(&application, &uuid, edit, now))
                .await
                .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
                .map_err(|error| error.to_string())
        },
        |result| Message::WorkflowFinished {
            message: "Saved schedule date".into(),
            result,
        },
    )
}

pub(super) fn custom_task(app: &mut App, uuid: String) -> Task<Message> {
    let Some((date, time)) = app.custom_dates.get(&uuid).cloned() else {
        return Task::none();
    };
    date_task(app, uuid, DateEdit::Custom(format!("{date}T{time}")))
}

pub(super) fn approve_task(app: &mut App, uuid: String) -> Task<Message> {
    let application = app.application.clone();
    app.busy = true;
    Task::perform(
        async move {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            tokio::task::spawn_blocking(move || {
                application.mutate(Some(&uuid), |store| {
                    workflow::approve_date(store, &uuid, now)
                })
            })
            .await
            .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
            .map_err(|error| error.to_string())
        },
        |result| Message::WorkflowFinished {
            message: "Approved schedule".into(),
            result,
        },
    )
}

pub(super) fn return_task(app: &mut App, uuid: String) -> Task<Message> {
    let application = app.application.clone();
    app.busy = true;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                application.mutate(Some(&uuid), |store| {
                    workflow::return_to_scheduling(store, &uuid)
                })
            })
            .await
            .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
            .map_err(|error| error.to_string())
        },
        |result| Message::WorkflowFinished {
            message: "Returned to scheduling".into(),
            result,
        },
    )
}

pub(super) fn date_parts(state: &App, row: &Value) -> (String, String) {
    let uuid = row.get("uuid").and_then(Value::as_str).unwrap_or_default();
    if let Some(parts) = state.custom_dates.get(uuid) {
        return parts.clone();
    }
    timezone(state.application.store().config())
        .ok()
        .and_then(|zone| local_schedule(row, zone))
        .map(|local| {
            (
                local.format("%Y-%m-%d").to_string(),
                local.format("%H:%M").to_string(),
            )
        })
        .unwrap_or_default()
}

/// The effective schedule in the configured timezone, as the cards and table show it.
pub(super) fn date_label(state: &App, row: &Value) -> String {
    let zone = timezone(state.application.store().config()).unwrap_or(chrono_tz::UTC);
    local_schedule(row, zone)
        .map(|local| local.format("%Y-%m-%d %H:%M %Z").to_string())
        .unwrap_or_else(|| "Date unavailable".into())
}

fn local_schedule(row: &Value, zone: chrono_tz::Tz) -> Option<DateTime<chrono_tz::Tz>> {
    effective_schedule(row)
        .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
        .map(|date| date.with_timezone(&zone))
}

pub fn view(state: &App) -> Element<'_, Message> {
    let config = state.application.store().config();
    let configured_profiles = profiles(config).unwrap_or_default();
    let rows = ordered_rows(
        state.publications.as_deref().unwrap_or_default(),
        state.revealed_publication(),
    );
    let mut cards = Vec::new();
    for row in rows {
        let current = phase(row).ok();
        let Some(uuid) = row.get("uuid").and_then(Value::as_str) else {
            continue;
        };
        let uuid = uuid.to_owned();
        let title = row
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("Untitled");
        cards.push(if current == Some(Phase::Scheduled) {
            scheduled_card(state, row, uuid, title)
        } else {
            picked_card(state, row, uuid, title, &configured_profiles)
        });
    }
    column![
        text("Schedule dates are shown in the configured timezone.").color(style::MUTED),
        scrollable(style::cards(cards))
            .id(SCROLL_ID)
            .height(Length::Fill)
            .on_scroll(|viewport| Message::Viewport(Tab::Schedule, viewport.into())),
    ]
    .spacing(8)
    .into()
}

fn scheduled_card<'a>(
    state: &'a App,
    row: &Value,
    uuid: String,
    title: &str,
) -> Element<'a, Message> {
    let mut card = column![
        style::art(state.row_previews.get(&uuid), false, 240.0),
        text(format!("{title} — {}", date_label(state, row))),
    ]
    .spacing(8);
    if state.revealed_publication() == Some(uuid.as_str()) {
        card = card.push(style::warning("Opened from Calendar"));
    }
    style::card(card.push(style::wide("Return to Pick").on_press(Message::ReturnToPick(uuid))))
        .into()
}

fn picked_card<'a>(
    state: &'a App,
    row: &Value,
    uuid: String,
    title: &str,
    configured_profiles: &[crate::scheduling::Profile],
) -> Element<'a, Message> {
    let (date, time) = date_parts(state, row);
    let mut card = column![
        style::art(state.row_previews.get(&uuid), false, 240.0),
        text(format!("{title} — {}", date_label(state, row))),
    ]
    .spacing(8);
    if state.revealed_publication() == Some(uuid.as_str()) {
        card = card.push(style::warning("Opened from Calendar"));
    }
    let names: Vec<String> = configured_profiles.iter().map(|p| p.name.clone()).collect();
    let current = row
        .get("scheduleProfile")
        .and_then(Value::as_str)
        .filter(|name| names.iter().any(|known| known == name))
        .map(str::to_owned);
    let preset_uuid = uuid.clone();
    card = card.push(
        row![
            text("Preset"),
            pick_list(names, current, move |profile| Message::PresetDate {
                uuid: preset_uuid.clone(),
                profile,
            })
            .placeholder("Custom / unassigned")
            .style(style::select),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    );
    let date_uuid = uuid.clone();
    let time_uuid = uuid.clone();
    card = card
        .push(text("Date"))
        .push(
            row![
                text_input("YYYY-MM-DD", &date)
                    .on_input(move |value| Message::CustomDate {
                        uuid: date_uuid.clone(),
                        value
                    })
                    .style(style::input),
                text_input("HH:MM", &time)
                    .on_input(move |value| Message::CustomTime {
                        uuid: time_uuid.clone(),
                        value
                    })
                    .style(style::input)
                    .width(Length::Fixed(80.0)),
            ]
            .spacing(6),
        )
        .push(style::wide("Save Custom").on_press(Message::SaveCustomDate(uuid.clone())));
    if effective_schedule(row).is_some() {
        card = card
            .push(style::wide("Use suggestion").on_press(Message::SuggestedDate(uuid.clone())))
            .push(style::wide("Approve date").on_press(Message::ApproveDate(uuid)))
            .push(text("Suggested date available"));
    } else {
        card = card
            .push(style::wide("Use suggestion (unavailable)"))
            .push(style::wide("Approve date (unavailable)"))
            .push(style::warning(
                "No suggestion available; approval is disabled",
            ));
    }
    style::card(card).into()
}

pub(super) fn ordered_rows<'a>(rows: &'a [Value], target: Option<&str>) -> Vec<&'a Value> {
    super::revealed_rows(
        rows.iter()
            .filter(|row| matches!(phase(row), Ok(Phase::Picked | Phase::Scheduled))),
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
    fn revealed_schedule_publication_is_first_in_the_scroll_list() {
        let rows = vec![
            serde_json::json!({"uuid":"first", "state":"PICKED"}),
            serde_json::json!({"uuid":"target", "state":"SCHEDULED"}),
        ];
        let ordered = ordered_rows(&rows, Some("target"));
        assert_eq!(ordered[0]["uuid"], "target");
    }
}
