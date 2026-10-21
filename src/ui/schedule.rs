use super::{App, Message};
use crate::{
    application::Application,
    error::Result,
    model::{Phase, effective_schedule, phase},
    scheduling::{profiles, timezone},
    workflow::{self, DateEdit},
};
use chrono::{DateTime, Utc};
use iced::{
    Element, Length, Task,
    widget::{button, column, row, scrollable, text, text_input},
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

const SCROLL_ID: &str = "schedule-publications";

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
    effective_schedule(row)
        .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
        .and_then(|date| {
            let local = date.with_timezone(&timezone(state.application.store().config()).ok()?);
            Some((
                local.format("%Y-%m-%d").to_string(),
                local.format("%H:%M").to_string(),
            ))
        })
        .unwrap_or_default()
}

pub fn view(state: &App) -> Element<'_, Message> {
    let config = state.application.store().config();
    let configured_profiles = profiles(config).unwrap_or_default();
    let rows = ordered_rows(
        state.publications.as_deref().unwrap_or_default(),
        state.revealed_publication(),
    );
    let mut cards =
        column![text("Schedule dates are shown in the configured timezone.")].spacing(12);
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
        if current == Some(Phase::Scheduled) {
            let date = effective_schedule(row)
                .and_then(|ts| DateTime::<Utc>::from_timestamp(ts, 0))
                .map(|date| {
                    date.with_timezone(&timezone(config).unwrap_or(chrono_tz::UTC))
                        .format("%Y-%m-%d %H:%M %Z")
                        .to_string()
                })
                .unwrap_or_else(|| "Date unavailable".into());
            let mut card = column![text(format!("{title} — {date}"))].spacing(4);
            if state.revealed_publication() == Some(uuid.as_str()) {
                card = card.push(text("Opened from Calendar"));
            }
            cards = cards
                .push(card.push(button("Return to Pick").on_press(Message::ReturnToPick(uuid))));
            continue;
        }
        let (date, time) = date_parts(state, row);
        let mut card =
            column![text(format!("{title} — {uuid}")), text("Read-only presets")].spacing(5);
        if state.revealed_publication() == Some(uuid.as_str()) {
            card = card.push(text("Opened from Calendar"));
        }
        let date_uuid = uuid.clone();
        let time_uuid = uuid.clone();
        for profile in &configured_profiles {
            let name = profile.name.clone();
            let action_uuid = uuid.clone();
            card = card.push(
                button(text(format!(
                    "{}: {} {:?} {:02}:00",
                    profile.name, profile.per_slot, profile.weekday, profile.hour
                )))
                .on_press(Message::PresetDate {
                    uuid: action_uuid,
                    profile: name,
                }),
            );
        }
        let suggested = effective_schedule(row).is_some();
        let mut controls = row![
            text_input("YYYY-MM-DD", &date).on_input(move |value| Message::CustomDate {
                uuid: date_uuid.clone(),
                value
            }),
            text_input("HH:MM", &time).on_input(move |value| Message::CustomTime {
                uuid: time_uuid.clone(),
                value
            }),
            button("Save Custom").on_press(Message::SaveCustomDate(uuid.clone())),
        ]
        .spacing(6);
        if suggested {
            controls = controls
                .push(button("Use suggestion").on_press(Message::SuggestedDate(uuid.clone())));
            controls =
                controls.push(button("Approve date").on_press(Message::ApproveDate(uuid.clone())));
        } else {
            controls = controls.push(button("Use suggestion (unavailable)"));
            controls = controls.push(button("Approve date (unavailable)"));
        }
        card = card.push(controls);
        card = card.push(text(if suggested {
            "Suggested date available"
        } else {
            "No suggestion available; approval is disabled"
        }));
        cards = cards.push(card);
    }
    column![scrollable(cards).id(SCROLL_ID).height(Length::Fill)]
        .spacing(8)
        .into()
}

fn ordered_rows<'a>(rows: &'a [Value], target: Option<&str>) -> Vec<&'a Value> {
    let mut rows = rows
        .iter()
        .filter(|row| matches!(phase(row), Ok(Phase::Picked | Phase::Scheduled)))
        .collect::<Vec<_>>();
    if let Some(target) = target
        && let Some(index) = rows
            .iter()
            .position(|row| row.get("uuid").and_then(Value::as_str) == Some(target))
    {
        rows.swap(0, index);
    }
    rows
}

pub(super) fn scroll_to_target() -> Task<Message> {
    iced::widget::operation::scroll_to(
        SCROLL_ID,
        iced::widget::operation::AbsoluteOffset {
            x: None,
            y: Some(0.0),
        },
    )
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
