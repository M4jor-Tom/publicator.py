use super::{App, Message, style};
use crate::{
    artwork,
    calendar::{self, Event},
    prompts::PromptMatch,
    scheduling,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use iced::{
    Element, Length, Rectangle, Task, Vector,
    advanced::widget::{Id, Operation, operation},
    widget::{button, column, container, row, space, stack, text},
};

/// The id of the current month's week row that contains today.
const TODAY_WEEK: &str = "calendar-today-week";
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct Preparation {
    /// Per row UUID with events: `artwork::preview_source` (the file and its recorded SHA-512),
    /// or why it has no artwork.
    pub artwork: HashMap<String, Result<(PathBuf, String), String>>,
    pub prompts: Vec<(PathBuf, PromptMatch)>,
}

pub fn view(state: &App) -> Element<'_, Message> {
    let rows = state.publications.as_deref().unwrap_or_default();
    let mut content = column![].spacing(8).max_width(1100);
    if let Some(error) = &state.calendar_error {
        return text(format!("Calendar unavailable: {error}"))
            .color(style::ERROR)
            .into();
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
    let today = DateTime::<Utc>::from_timestamp(now, 0)
        .map(|instant| instant.with_timezone(&zone).date_naive());
    content = content.push(text(format!("All apparition dates — {zone}")).color(style::MUTED));
    match calendar::months(&events, zone, now) {
        Ok(months) => {
            for month in months {
                content = content.push(month_view(state, &events, month, today, now, zone));
            }
        }
        Err(error) => content = content.push(text(error.to_string())),
    }
    super::tab_scrollable(super::Tab::Calendar, content).into()
}

pub(super) fn month_view<'a>(
    state: &'a App,
    events: &[Event],
    month: NaiveDate,
    today: Option<NaiveDate>,
    now: i64,
    zone: chrono_tz::Tz,
) -> Element<'a, Message> {
    let current = today.and_then(|today| today.with_day(1)) == Some(month);
    let title = if current {
        format!("{} — current month", month.format("%B %Y"))
    } else {
        month.format("%B %Y").to_string()
    };
    let days = days_in_month(month);
    let first_weekday = month.weekday().num_days_from_monday() as usize;
    // A grid cell is its background and, laid out apart, its content (see the week stack below).
    let background = || container(space()).width(Length::Fill).height(Length::Fill);
    let blank = || -> (Element<'a, Message>, Element<'a, Message>) {
        (
            background().style(style::other_day).into(),
            space().width(Length::Fill).into(),
        )
    };
    let section = column![
        text(title)
            .font(style::BOLD)
            .color(if current { style::TODAY } else { style::MUTED }),
        row(WEEKDAYS.map(|day| text(day)
            .size(12)
            .color(style::MUTED)
            .center()
            .width(Length::Fill)
            .into())),
    ]
    .spacing(4);
    let mut cells = (0..first_weekday).map(|_| blank()).collect::<Vec<_>>();
    for day in 1..=days {
        let date = month.with_day(day).unwrap();
        let mut cell =
            column![text(day.to_string()).size(11).color(iced::color!(0x777777))].spacing(2);
        for event in events
            .iter()
            .filter(|event| event_date(event, zone) == Some(date))
        {
            cell = cell.push(event_view(state, event, now, zone));
        }
        // Like the web's 78px table cell: a minimum that grows to show every event.
        cells.push((
            background().style(style::day(Some(date) == today)).into(),
            container(row![space().height(74.0), cell])
                .width(Length::Fill)
                .padding(2)
                .into(),
        ));
    }
    while !cells.len().is_multiple_of(7) {
        cells.push(blank());
    }
    let mut weeks = column![];
    // The grid row holding today, counting the leading blanks.
    let today_week = today
        .filter(|_| current)
        .map(|today| (today.day() as usize - 1 + first_weekday) / 7);
    let mut week_index = 0;
    while !cells.is_empty() {
        // Every cell of a week is as tall as its tallest day. A Shrink row gives Fill-height
        // children only the height of the siblings laid out before them (iced 0.14 flex), but a
        // stack lays its under layer out at its base layer's size: the contents set the height.
        let (backgrounds, contents): (Vec<_>, Vec<_>) = cells.drain(..7).unzip();
        let week = stack![row(contents)].push_under(row(backgrounds));
        weeks = weeks.push(if today_week == Some(week_index) {
            // `today_week_offset` finds this week to open the Calendar on it.
            container(week).id(TODAY_WEEK).into()
        } else {
            Element::from(week)
        });
        week_index += 1;
    }
    section.push(weeks).into()
}

fn event_key(event: &Event) -> String {
    format!("{}:{}", event.uuid, event.apparition_index)
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
    let artwork = state.calendar_art(&event.uuid).map(|(path, _)| path);
    let data_dir = &state.application.store().paths().data_dir;
    state
        .prompt_matches
        .get(&prompt_path(artwork, data_dir, event)?)
        .map(super::prompt::view)
}

/// Where an event's prompt is matched: its resolved artwork, else `data_dir/<basename>`.
fn prompt_path(artwork: Option<&Path>, data_dir: &Path, event: &Event) -> Option<PathBuf> {
    artwork
        .map(Path::to_owned)
        .or_else(|| event.basename.as_deref().map(|name| data_dir.join(name)))
}

fn event_date(event: &Event, zone: chrono_tz::Tz) -> Option<NaiveDate> {
    DateTime::<Utc>::from_timestamp(event.timestamp, 0)
        .map(|instant| instant.with_timezone(&zone).date_naive())
}

fn event_view<'a>(
    state: &'a App,
    event: &Event,
    now: i64,
    zone: chrono_tz::Tz,
) -> Element<'a, Message> {
    let local_time = DateTime::<Utc>::from_timestamp(event.timestamp, 0)
        .map(|instant| instant.with_timezone(&zone).format("%H:%M").to_string())
        .unwrap_or_else(|| "time unavailable".into());
    let key = event_key(event);
    let expanded = state.calendar_expanded.contains(&key);
    let chip = button(text(format!("{local_time} {}", event.title)).size(11))
        .padding([2, 4])
        .width(Length::Fill)
        .style(style::event(event.timestamp >= now))
        .on_press(Message::ToggleCalendarEvent(key));
    if !expanded {
        return chip.into();
    }
    let mut item = column![
        chip,
        text(format!(
            "{local_time} — {} — {}",
            event.platform, event.title
        ))
        .size(12),
        artwork_view(state, event),
    ]
    .spacing(4);
    if let Some(prompt) = event_prompt_view(state, event) {
        item = item.push(prompt);
    }
    if let Some(url) = event
        .url
        .as_deref()
        .filter(|url| actual_http_url(url).is_ok())
    {
        item = item.push(
            button(text("Open actual URL").size(12))
                .on_press(Message::OpenActualUrl(url.to_owned())),
        );
    }
    if state.publication_exists(&event.uuid) {
        item = item.push(
            button(text("Open publication").size(12))
                .on_press(Message::JumpToPublication(event.uuid.clone())),
        );
    }
    item.into()
}

fn artwork_view<'a>(state: &'a App, event: &Event) -> Element<'a, Message> {
    match state.calendar_artwork.get(&event.uuid) {
        None => text("Checking artwork…").into(),
        Some(Err(error)) => text(format!("Artwork unavailable: {error}")).into(),
        Some(Ok((original, _))) => match state.calendar_previews.get(&event.uuid) {
            None | Some(None) => text("Loading thumbnail…").into(),
            Some(Some(Ok(path))) => style::opens(
                iced::widget::image(iced::widget::image::Handle::from_path(path))
                    .width(Length::Fixed(120.0))
                    .height(Length::Fixed(120.0)),
                original,
                path,
            ),
            Some(Some(Err(error))) => text(format!("Preview unavailable: {error}")).into(),
        },
    }
}

/// The laid-out offset of today's week inside the Calendar list; nothing when it isn't shown.
pub(super) fn today_week_offset() -> Task<f32> {
    #[derive(Default)]
    struct Find {
        /// The list content's top and how far it can scroll.
        list: Option<(f32, f32)>,
        week: Option<f32>,
    }
    impl Operation<f32> for Find {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
            operate(self);
        }
        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            if id == Some(&Id::new(TODAY_WEEK)) {
                self.week = Some(bounds.y);
            }
        }
        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: Rectangle,
            content_bounds: Rectangle,
            _translation: Vector,
            _state: &mut dyn operation::Scrollable,
        ) {
            if id == Some(&Id::new(super::Tab::Calendar.scroll_id())) {
                let end = (content_bounds.height - bounds.height).max(0.0);
                self.list = Some((content_bounds.y, end));
            }
        }
        fn finish(&self) -> operation::Outcome<f32> {
            match (self.list, self.week) {
                // Clamped as drawn: an offset past the end would follow later growth down.
                (Some((top, end)), Some(week)) => operation::Outcome::Some((week - top).min(end)),
                _ => operation::Outcome::None,
            }
        }
    }
    iced::advanced::widget::operate(Find::default())
}

pub(super) fn prepare_task(state: &mut App) -> Task<Message> {
    if state.calendar_prepared || state.calendar_preparing {
        return Task::none();
    }
    let generation = state.rows_generation;
    let rows = state.publications.as_deref().unwrap_or_default().to_vec();
    let events = match calendar::timeline(&rows) {
        Ok(events) => events,
        Err(error) => {
            return Task::done(Message::CalendarPrepared(
                generation,
                Err(error.to_string()),
            ));
        }
    };
    let data_dir = state.application.store().paths().data_dir.clone();
    let index = state.artwork_index.clone();
    let archive = state.prompt_archive.clone();
    let archive_error = state.prompt_archive_error.clone();
    state.calendar_preparing = true;
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let index = super::artwork_index(&index, &data_dir)?;
                let with_events = events
                    .iter()
                    .map(|event| event.uuid.as_str())
                    .collect::<HashSet<_>>();
                let artwork = rows
                    .iter()
                    .filter_map(|row| {
                        let uuid = row.get("uuid")?.as_str()?;
                        if !with_events.contains(uuid) {
                            return None;
                        }
                        let source = artwork::preview_source(index, row);
                        Some((uuid.to_owned(), source.map_err(|error| error.to_string())))
                    })
                    .collect::<HashMap<_, _>>();
                let paths = events
                    .iter()
                    .filter_map(|event| {
                        let found = artwork.get(&event.uuid).and_then(|art| art.as_ref().ok());
                        prompt_path(found.map(|(path, _)| path.as_path()), &data_dir, event)
                    })
                    .collect();
                let prompts = super::resolve_prompt_matches(
                    archive.as_deref(),
                    archive_error.as_deref(),
                    paths,
                );
                Ok(Preparation { artwork, prompts })
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()))
        },
        move |result| Message::CalendarPrepared(generation, result),
    )
}

/// Thumbnails for expanded events; collapsed chips show none.
pub(super) fn preview_tasks(state: &mut App) -> Task<Message> {
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
    let mut seen = HashSet::new();
    let records = events
        .iter()
        .filter(|event| state.calendar_expanded.contains(&event_key(event)))
        .filter_map(|event| {
            let (source, sha) = state.calendar_art(&event.uuid)?;
            if !seen.insert(&event.uuid) || state.calendar_previews.contains_key(&event.uuid) {
                return None;
            }
            Some((event.uuid.clone(), source.to_owned(), sha.to_owned()))
        })
        .take(capacity)
        .collect::<Vec<_>>();
    let cache = state.application.store().paths().data_dir.join(".thumbs");
    let generation = state.rows_generation;
    let mut tasks = Vec::new();
    for (uuid, source, sha) in records {
        state.calendar_previews.insert(uuid.clone(), None);
        let task_uuid = uuid.clone();
        let cache = cache.clone();
        tasks.push(Task::perform(
            super::preview::prepare(source, sha, cache),
            move |result| Message::CalendarPreviewFinished {
                generation,
                uuid: task_uuid,
                result,
            },
        ));
    }
    Task::batch(tasks)
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
