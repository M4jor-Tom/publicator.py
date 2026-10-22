use super::{App, Message};
use crate::{
    calendar::{self, Event},
    images,
    prompts::PromptMatch,
    scheduling,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, scrollable, text},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const SCROLL_ID: &str = "calendar-events";
// ponytail: 280px/event estimates visible cells; use widget bounds if tall day stacks miss previews.
const EVENT_HEIGHT: f32 = 280.0;

#[derive(Clone, Debug)]
pub struct Preparation {
    pub artwork: HashMap<String, Option<PathBuf>>,
    pub prompts: Vec<(PathBuf, PromptMatch)>,
}

pub fn view(state: &App) -> Element<'_, Message> {
    let rows = state.publications.as_deref().unwrap_or_default();
    let mut content = column![text("Calendar")].spacing(8);
    if let Some(error) = &state.calendar_error {
        content = content.push(text(format!("Calendar unavailable: {error}")));
        return content.into();
    }
    let events = match calendar::timeline(rows) {
        Ok(events) => events,
        Err(error) => return column![text("Calendar unavailable"), text(error.to_string())].into(),
    };
    let zone = match scheduling::timezone(state.application.store().config()) {
        Ok(zone) => zone,
        Err(error) => {
            return column![
                text("Calendar timezone unavailable"),
                text(error.to_string())
            ]
            .into();
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let current_month = DateTime::<Utc>::from_timestamp(now, 0).map(|instant| {
        instant
            .with_timezone(&zone)
            .date_naive()
            .with_day(1)
            .unwrap()
    });
    content = content.push(text(format!("All apparition dates — {}", zone)));
    match calendar::months(&events, zone, now) {
        Ok(months) => {
            for month in months {
                content = content.push(month_view(state, &events, month, current_month, zone));
            }
        }
        Err(error) => content = content.push(text(error.to_string())),
    }
    scrollable(content)
        .id(SCROLL_ID)
        .height(Length::Fill)
        .on_scroll(|viewport| Message::CalendarViewport {
            offset: viewport.absolute_offset().y,
            height: viewport.bounds().height,
        })
        .into()
}

fn month_view<'a>(
    state: &'a App,
    events: &[Event],
    month: NaiveDate,
    current_month: Option<NaiveDate>,
    zone: chrono_tz::Tz,
) -> Element<'a, Message> {
    let title = if Some(month) == current_month {
        format!("{} — current month", month.format("%B %Y"))
    } else {
        month.format("%B %Y").to_string()
    };
    let days = days_in_month(month);
    let first_weekday = month.weekday().num_days_from_monday() as usize;
    let mut section = column![text(title)].spacing(4);
    let mut week = row![].spacing(4);
    for _ in 0..first_weekday {
        week = week.push(container(text("")).width(Length::Fill));
    }
    for day in 1..=days {
        let date = month.with_day(day).unwrap();
        let mut cell = column![text(day.to_string())].spacing(4);
        for event in events
            .iter()
            .filter(|event| event_date(event, zone) == Some(date))
        {
            cell = cell.push(event_view(state, event, zone));
        }
        week = week.push(container(cell).width(Length::Fill).padding(4));
        if (first_weekday + day as usize) % 7 == 0 {
            section = section.push(week);
            week = row![].spacing(4);
        }
    }
    if (first_weekday + days as usize) % 7 != 0 {
        for _ in (first_weekday + days as usize) % 7..7 {
            week = week.push(container(text("")).width(Length::Fill));
        }
        section = section.push(week);
    }
    section.into()
}

fn days_in_month(month: NaiveDate) -> u32 {
    let next = if month.month() == 12 {
        NaiveDate::from_ymd_opt(month.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(month.year(), month.month() + 1, 1).unwrap()
    };
    (next - month).num_days() as u32
}

pub(super) fn event_prompt_view<'a>(state: &'a App, event: &Event) -> Option<Element<'a, Message>> {
    let prompt_path = event.basename.as_deref().map(|basename| {
        state
            .calendar_artwork
            .get(basename)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_else(|| state.application.store().paths().data_dir.join(basename))
    });
    prompt_path
        .as_deref()
        .and_then(|path| state.prompt_matches.get(path))
        .map(super::prompt::view)
}

fn event_date(event: &Event, zone: chrono_tz::Tz) -> Option<NaiveDate> {
    DateTime::<Utc>::from_timestamp(event.timestamp, 0)
        .map(|instant| instant.with_timezone(&zone).date_naive())
}

fn event_view<'a>(state: &'a App, event: &Event, zone: chrono_tz::Tz) -> Element<'a, Message> {
    let local_time = DateTime::<Utc>::from_timestamp(event.timestamp, 0)
        .map(|instant| instant.with_timezone(&zone).format("%H:%M").to_string())
        .unwrap_or_else(|| "time unavailable".into());
    let mut item = column![text(format!(
        "{local_time} — {} — {}",
        event.platform, event.title
    ))]
    .spacing(4);
    item = item.push(artwork_view(state, event));
    if let Some(prompt) = event_prompt_view(state, event) {
        item = item.push(prompt);
    }
    if let Some(url) = event
        .url
        .as_deref()
        .filter(|url| actual_http_url(url).is_ok())
    {
        item =
            item.push(button("Open actual URL").on_press(Message::OpenActualUrl(url.to_owned())));
    }
    if state.publication_exists(&event.uuid) {
        item = item.push(
            button("Open publication").on_press(Message::JumpToPublication(event.uuid.clone())),
        );
    }
    item.into()
}

fn artwork_view<'a>(state: &'a App, event: &Event) -> Element<'a, Message> {
    let Some(basename) = event.basename.as_deref() else {
        return text("Artwork missing").into();
    };
    match state.calendar_artwork.get(basename) {
        None => text("Checking artwork…").into(),
        Some(None) => text("Artwork missing").into(),
        Some(Some(_)) => match state.calendar_previews.get(basename) {
            None | Some(None) => text("Loading thumbnail…").into(),
            Some(Some(Ok(path))) => {
                iced::widget::image(iced::widget::image::Handle::from_path(path))
                    .width(Length::Fixed(120.0))
                    .height(Length::Fixed(120.0))
                    .into()
            }
            Some(Some(Err(error))) => text(format!("Preview unavailable: {error}")).into(),
        },
    }
}

pub(super) fn prepare_task(state: &mut App) -> Task<Message> {
    if state.calendar_prepared || state.calendar_preparing {
        return Task::none();
    }
    let rows = state.publications.as_deref().unwrap_or_default().to_vec();
    let events = match calendar::timeline(&rows) {
        Ok(events) => events,
        Err(error) => return Task::done(Message::CalendarPrepared(Err(error.to_string()))),
    };
    let basenames = events
        .iter()
        .filter_map(|event| event.basename.clone())
        .collect::<HashSet<_>>();
    let data_dir = state.application.store().paths().data_dir.clone();
    let archive = state.prompt_archive.clone();
    let archive_error = state.prompt_archive_error.clone();
    state.calendar_preparing = true;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let index = images::index_by_basename(&data_dir)?;
                let artwork = basenames
                    .into_iter()
                    .map(|basename| {
                        let path = index.get(&basename).cloned();
                        (basename, path)
                    })
                    .collect::<HashMap<_, _>>();
                let paths = events
                    .iter()
                    .filter_map(|event| event.basename.as_ref())
                    .map(|basename| {
                        artwork
                            .get(basename)
                            .and_then(Option::as_ref)
                            .cloned()
                            .unwrap_or_else(|| data_dir.join(basename))
                    })
                    .collect();
                let prompts = super::resolve_prompt_matches(
                    archive.as_deref(),
                    archive_error.as_deref(),
                    paths,
                );
                Ok::<_, crate::error::Error>(Preparation { artwork, prompts })
            })
            .await
            .unwrap_or_else(|error| Err(crate::error::Error::Config(error.to_string())))
            .map_err(|error| error.to_string())
        },
        Message::CalendarPrepared,
    )
}

pub(super) fn preview_tasks(state: &mut App, offset: f32, height: f32) -> Task<Message> {
    let Some(rows) = state.publications.as_deref() else {
        return Task::none();
    };
    let Ok(events) = calendar::timeline(rows) else {
        return Task::none();
    };
    let capacity = 4usize.saturating_sub(
        state
            .calendar_previews
            .values()
            .filter(|result| result.is_none())
            .count(),
    );
    let start = (offset.max(0.0) / EVENT_HEIGHT) as usize;
    let end = ((offset.max(0.0) + height.max(0.0)) / EVENT_HEIGHT).ceil() as usize + 2;
    let mut seen = HashSet::new();
    let records = events
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .filter_map(|event| {
            let basename = event.basename.as_ref()?;
            let source = state.calendar_artwork.get(basename)?.as_ref()?;
            if !seen.insert(basename.clone()) || state.calendar_previews.contains_key(basename) {
                return None;
            }
            let sha = event.sha512.clone();
            Some((basename.clone(), source.clone(), sha))
        })
        .take(capacity)
        .collect::<Vec<_>>();
    let cache = state.application.store().paths().data_dir.join(".thumbs");
    let mut tasks = Vec::new();
    for (basename, source, sha) in records {
        state.calendar_previews.insert(basename.clone(), None);
        let task_basename = basename.clone();
        let cache = cache.clone();
        tasks.push(Task::perform(
            async move { super::preview::prepare_lazy(source, sha, cache).await },
            move |result| Message::CalendarPreviewFinished {
                basename: task_basename,
                result,
            },
        ));
    }
    Task::batch(tasks)
}

pub fn scroll_to_target() -> Task<Message> {
    iced::widget::operation::scroll_to(
        SCROLL_ID,
        iced::widget::operation::AbsoluteOffset {
            x: None,
            y: Some(0.0),
        },
    )
}

pub fn actual_http_url(value: &str) -> Result<url::Url, String> {
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err("actual URL must be a valid HTTP(S) URL".into());
    }
    let url = url::Url::parse(value).map_err(|_| "actual URL must be a valid HTTP(S) URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("actual URL must be a valid HTTP(S) URL".into());
    }
    Ok(url)
}

pub fn open_stored_actual_url_with(
    rows: &[Value],
    value: &str,
    opener: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let url = actual_http_url(value)?;
    let stored = rows
        .iter()
        .flat_map(|row| row.get("apparitions").and_then(Value::as_array))
        .flatten()
        .any(|apparition| apparition.get("url").and_then(Value::as_str) == Some(value));
    if !stored {
        return Err("URL is not stored on an apparition".into());
    }
    opener(url.as_str())
}

pub(super) fn system_open_url(value: &str) -> Result<(), String> {
    let url = actual_http_url(value)?;
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("rundll32.exe")
        .arg("url.dll,FileProtocolHandler")
        .arg(url.as_str())
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url.as_str()).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .spawn();
    result
        .map(|_| ())
        .map_err(|error| format!("could not open actual URL: {error}"))
}

#[cfg(test)]
mod tests {
    use super::event_date;
    use crate::calendar::timeline;
    use chrono::{NaiveDate, TimeZone, Utc};
    use chrono_tz::Europe::Paris;
    use serde_json::json;

    #[test]
    fn event_cell_date_uses_configured_timezone_instead_of_utc_date() {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 1, 5, 23, 30, 0)
            .unwrap()
            .timestamp();
        let row = json!({
            "uuid":"time-zone-fixture", "state":"DESCRIBED", "title":"Local day",
            "description":"d", "submissionTimestamp":timestamp,
            "files":[{"basename":"art.png","sha512sum":"a".repeat(128)}],
            "apparitions":[{"platformName":"deviantart","state":"unpublished"}]
        });
        let event = timeline(&[row]).unwrap().remove(0);
        let cell_date = event_date(&event, Paris);
        assert_eq!(
            cell_date,
            Some(NaiveDate::from_ymd_opt(2026, 1, 6).unwrap())
        );
        assert_ne!(
            cell_date,
            Some(Utc.timestamp_opt(timestamp, 0).unwrap().date_naive())
        );
    }
}
