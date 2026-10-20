use crate::config::Config;
use crate::error::{Error, Result};
use crate::model::{Phase, Publications, effective_schedule, phase};
use chrono::{
    DateTime, Datelike, Days, LocalResult, NaiveDateTime, Offset, TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use toml::Value;

const SLOT_HORIZON_WEEKS: usize = 52;

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub weekday: Weekday,
    pub hour: u32,
    pub per_slot: usize,
    pub default_tiers: Vec<String>,
    pub default_price: Option<f64>,
}

fn config_error(message: impl Into<String>) -> Error {
    Error::Config(format!("schedule: {}", message.into()))
}

fn int(value: Option<&Value>, default: i64, name: &str) -> Result<i64> {
    match value {
        None => Ok(default),
        Some(v) => v
            .as_integer()
            .ok_or_else(|| config_error(format!("{name} must be an integer"))),
    }
}

fn week_day(value: Option<&Value>) -> Result<Weekday> {
    let name = match value {
        None => "tuesday",
        Some(Value::String(name)) => name,
        Some(_) => return Err(config_error("day must be a weekday name")),
    }
    .to_ascii_lowercase();
    match name.as_str() {
        "monday" => Ok(Weekday::Mon),
        "tuesday" => Ok(Weekday::Tue),
        "wednesday" => Ok(Weekday::Wed),
        "thursday" => Ok(Weekday::Thu),
        "friday" => Ok(Weekday::Fri),
        "saturday" => Ok(Weekday::Sat),
        "sunday" => Ok(Weekday::Sun),
        _ => Err(config_error(format!("day {name:?} is invalid"))),
    }
}

pub fn profiles(config: &Config) -> Result<Vec<Profile>> {
    let schedule = &config.schedule;
    let profile_values = match schedule.get("profiles") {
        None => vec![toml::Value::Table(schedule.clone())],
        Some(toml::Value::Array(values)) => values.clone(),
        Some(_) => return Err(config_error("profiles must be an array of tables")),
    };
    if profile_values.is_empty() {
        return Err(config_error("profiles is empty"));
    }
    let mut output = Vec::with_capacity(profile_values.len());
    let mut slots_seen = Vec::new();
    let mut names_seen = Vec::new();
    for (index, value) in profile_values.iter().enumerate() {
        let table = value
            .as_table()
            .ok_or_else(|| config_error(format!("profile {} must be a table", index + 1)))?;
        if let Some(freq) = table.get("frequency") {
            if freq.as_str() != Some("weekly") {
                return Err(config_error("frequency must be 'weekly'"));
            }
        }
        let name = match table.get("name") {
            None => "default".to_owned(),
            Some(Value::String(name)) => name.clone(),
            Some(_) => return Err(config_error("profile name must be a string")),
        };
        if name.trim().is_empty() {
            return Err(config_error("profile name must not be empty"));
        }
        if names_seen.iter().any(|old| old == &name) {
            return Err(config_error(format!("duplicate profile name {name:?}")));
        }
        names_seen.push(name.clone());
        let weekday = week_day(table.get("day"))?;
        let hour = int(table.get("hour"), 20, "hour")?;
        if !(0..=23).contains(&hour) {
            return Err(config_error(format!("hour must be 0..=23, got {hour}")));
        }
        let capacity = int(table.get("per_slot"), 2, "per_slot")?;
        if capacity < 1 {
            return Err(config_error(format!(
                "per_slot must be >= 1, got {capacity}"
            )));
        }
        if slots_seen
            .iter()
            .any(|(day, h)| *day == weekday && *h == hour as u32)
        {
            return Err(config_error(format!(
                "profiles collide on day/hour ({weekday}, {hour})"
            )));
        }
        slots_seen.push((weekday, hour as u32));
        let default_tiers = match table.get("default_tiers") {
            None => vec![],
            Some(Value::Array(values)) => values
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| config_error("default_tiers must contain only strings"))
                })
                .collect::<Result<Vec<_>>>()?,
            Some(_) => return Err(config_error("default_tiers must be an array of tier names")),
        };
        if let Some(tier) = default_tiers
            .iter()
            .find(|tier| !config.tiers.contains(tier))
        {
            return Err(config_error(format!(
                "profile {name:?} has unconfigured default tier {tier:?}"
            )));
        }
        let default_price = match table.get("default_price") {
            None => None,
            Some(v) => {
                let price = v
                    .as_float()
                    .or_else(|| v.as_integer().map(|n| n as f64))
                    .ok_or_else(|| {
                        config_error("default_price must be a finite nonnegative number")
                    })?;
                if !price.is_finite() || price < 0.0 {
                    return Err(config_error(
                        "default_price must be a finite nonnegative number",
                    ));
                }
                Some(price)
            }
        };
        output.push(Profile {
            name,
            weekday,
            hour: hour as u32,
            per_slot: capacity as usize,
            default_tiers,
            default_price,
        });
    }
    Ok(output)
}

fn resolve_naive(naive: NaiveDateTime, zone: Tz) -> Result<i64> {
    match zone.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value.timestamp()),
        LocalResult::Ambiguous(a, b) => Ok(a.timestamp().min(b.timestamp())),
        LocalResult::None => {
            // The wall time is in a spring-forward gap. Resolve it with the offset
            // immediately before the transition, matching zoneinfo fold=0.
            for minutes in 1..=48 * 60 {
                let prior = naive
                    .checked_sub_signed(chrono::Duration::minutes(minutes))
                    .ok_or_else(|| config_error("wall-clock gap search overflow"))?;
                if let LocalResult::Single(value) = zone.from_local_datetime(&prior) {
                    let offset = value.offset().fix().local_minus_utc();
                    return Ok(naive.and_utc().timestamp() - i64::from(offset));
                }
            }
            Err(config_error(
                "could not find pre-transition offset within 48 hours",
            ))
        }
    }
}

pub fn resolve_wall_clock(input: &str, zone: Tz) -> Result<i64> {
    let naive = NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .or_else(|| {
            DateTime::parse_from_rfc3339(input)
                .ok()
                .map(|v| v.naive_local())
        })
        .or_else(|| {
            DateTime::parse_from_str(input, "%Y-%m-%dT%H:%M%:z")
                .ok()
                .map(|v| v.naive_local())
        })
        .or_else(|| {
            DateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%:z")
                .ok()
                .map(|v| v.naive_local())
        })
        .ok_or_else(|| config_error(format!("invalid wall-clock {input:?}")))?;
    resolve_naive(naive, zone)
}

pub fn slots(profile: &Profile, zone: Tz, now: i64) -> Result<Vec<i64>> {
    if profile.hour > 23 || profile.per_slot == 0 {
        return Err(config_error("invalid profile hour or capacity"));
    }
    let current = DateTime::<Utc>::from_timestamp(now, 0)
        .ok_or_else(|| config_error("now is outside supported timestamp range"))?;
    let today = current.with_timezone(&zone).date_naive();
    let days_ahead = (i64::from(profile.weekday.num_days_from_monday())
        - i64::from(today.weekday().num_days_from_monday()))
    .rem_euclid(7) as u64;
    let mut day = today
        .checked_add_days(Days::new(days_ahead))
        .ok_or_else(|| config_error("slot date overflow"))?;
    let local = day.and_hms_opt(profile.hour, 0, 0).unwrap();
    if resolve_naive(local, zone)? <= now {
        day = day
            .checked_add_days(Days::new(7))
            .ok_or_else(|| config_error("slot date overflow"))?;
    }
    let mut output = Vec::with_capacity(SLOT_HORIZON_WEEKS);
    for _ in 0..SLOT_HORIZON_WEEKS {
        output.push(resolve_naive(
            day.and_hms_opt(profile.hour, 0, 0).unwrap(),
            zone,
        )?);
        day = day
            .checked_add_days(Days::new(7))
            .ok_or_else(|| config_error("slot date overflow"))?;
    }
    Ok(output)
}

fn schedule_zone(config: &Config) -> Result<Tz> {
    let name = match config.schedule.get("timezone") {
        None => "Europe/Paris",
        Some(Value::String(name)) => name,
        Some(_) => return Err(config_error("timezone must be an IANA timezone name")),
    };
    name.parse()
        .map_err(|_| config_error(format!("unknown timezone {name:?}")))
}

pub fn repack(rows: &mut Publications, config: &Config, now: i64) -> Result<()> {
    let all_profiles = profiles(config)?;
    let zone = schedule_zone(config)?;
    let profile_slots: Vec<_> = all_profiles
        .iter()
        .map(|profile| Ok((profile, slots(profile, zone, now)?)))
        .collect::<Result<_>>()?;
    let fallback = all_profiles
        .first()
        .ok_or_else(|| config_error("profiles is empty"))?;
    let mut occupancy: HashMap<i64, usize> = HashMap::new();
    for row in rows.iter().filter(|row| {
        phase(row).ok() != Some(Phase::Picked)
            || row.get("scheduleIsManual").and_then(JsonValue::as_bool) == Some(true)
    }) {
        if let Some(ts) = effective_schedule(row) {
            *occupancy.entry(ts).or_insert(0) += 1;
        }
    }
    for row in rows.iter_mut().filter(|row| {
        phase(row).ok() == Some(Phase::Picked)
            && row.get("scheduleIsManual").and_then(JsonValue::as_bool) != Some(true)
    }) {
        let requested = row.get("scheduleProfile").and_then(JsonValue::as_str);
        let profile = requested.and_then(|name| {
            profile_slots
                .iter()
                .find(|(profile, _)| profile.name == name)
        });
        let Some((profile, available)) = profile.or_else(|| {
            if requested.is_none() {
                profile_slots.iter().find(|(p, _)| p.name == fallback.name)
            } else {
                None
            }
        }) else {
            row.as_object_mut()
                .unwrap()
                .remove("apparitionTimestampIfDifferentThanSubmission");
            continue;
        };
        if requested.is_none() {
            row["scheduleProfile"] = JsonValue::String(profile.name.clone());
        }
        row["scheduleIsManual"] = JsonValue::Bool(false);
        row.as_object_mut()
            .unwrap()
            .remove("apparitionTimestampIfDifferentThanSubmission");
        if let Some(ts) = available
            .iter()
            .find(|ts| occupancy.get(ts).copied().unwrap_or(0) < profile.per_slot)
        {
            row["apparitionTimestampIfDifferentThanSubmission"] = JsonValue::from(*ts);
            *occupancy.entry(*ts).or_insert(0) += 1;
        }
    }
    Ok(())
}
