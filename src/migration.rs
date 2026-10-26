use crate::{
    config::{Config, validate_publications},
    error::{Error, Result},
    model::{GENERATE_MARKER, Publications},
};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const OLD_TITLE: &str = "urlElsePublicationName";
const OLD_DATE: &str = "apparitionTimestampIfDifferentThanSubmission";

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn inferred_state(row: &Value) -> String {
    if let Some(state) = row.get("state").and_then(Value::as_str)
        && matches!(state, "PICKED" | "SCHEDULED" | "DESCRIBED" | "APPEARED")
    {
        return state.to_owned();
    }
    let apparitions = row
        .get("apparitions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !apparitions.is_empty()
        && apparitions
            .iter()
            .all(|app| app.get("state").and_then(Value::as_str) == Some("published_or_scheduled"))
    {
        return "APPEARED".into();
    }
    let complete = ["title", "description"].iter().all(|key| {
        row.get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty() && !value.contains(GENERATE_MARKER))
    });
    if row.get(OLD_DATE).is_some() {
        if complete { "DESCRIBED" } else { "SCHEDULED" }.into()
    } else {
        "PICKED".into()
    }
}

/// Convert legacy publication documents without mutating the caller's snapshot.
pub fn migrate(
    rows: &Publications,
    recovered: &HashMap<String, String>,
    config: &Config,
) -> Result<Publications> {
    let mut out = rows.clone();
    for row in &mut out {
        migrate_row(row, recovered)?;
    }
    validate_publications(&out, config)?;
    Ok(out)
}

fn legacy_values(apps: &[Value], uuid: &str) -> Result<(HashSet<String>, HashSet<i64>)> {
    let mut names = HashSet::new();
    let mut dates = HashSet::new();
    for (index, app) in apps.iter().enumerate() {
        let app = app.as_object().ok_or_else(|| {
            Error::Validation(format!("{uuid}: apparition {index} must be an object"))
        })?;
        if let Some(old) = app
            .get(OLD_TITLE)
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
        {
            if is_http_url(old) {
                validate_legacy_url(app, old, uuid)?;
            } else {
                names.insert(old.to_owned());
            }
        }
        if let Some(date) = app.get(OLD_DATE) {
            let ts = date.as_i64().ok_or_else(|| {
                Error::Validation(format!("{uuid}: invalid legacy apparition date"))
            })?;
            dates.insert(ts);
        }
    }
    if names.len() > 1 {
        return Err(Error::Validation(format!(
            "{uuid}: conflicting legacy titles"
        )));
    }
    if dates.len() > 1 {
        return Err(Error::Validation(format!(
            "{uuid}: conflicting apparition dates"
        )));
    }
    Ok((names, dates))
}

fn validate_legacy_url(app: &Map<String, Value>, old: &str, uuid: &str) -> Result<()> {
    if let Some(existing) = app.get("url")
        && !existing.is_null()
        && existing.as_str() != Some(old)
    {
        return Err(Error::Validation(format!(
            "{uuid}: conflicting apparition urls"
        )));
    }
    Ok(())
}

fn migrate_row(row: &mut Value, recovered: &HashMap<String, String>) -> Result<()> {
    let uuid = row
        .get("uuid")
        .and_then(Value::as_str)
        .unwrap_or("publication")
        .to_owned();
    let apps = row
        .get("apparitions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let new_draft = matches!(
        row.get("state").and_then(Value::as_str),
        Some("PICKED" | "SCHEDULED")
    ) && row.get("title").is_some()
        && !apps
            .iter()
            .any(|app| app.get(OLD_TITLE).is_some() || app.get(OLD_DATE).is_some());
    let (names, dates) = legacy_values(&apps, &uuid)?;
    let object = row
        .as_object_mut()
        .ok_or_else(|| Error::Validation(format!("{uuid}: publication must be an object")))?;
    migrate_date(object, dates.iter().next().copied(), &uuid)?;
    for app in object
        .get_mut("apparitions")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        migrate_apparition(app, &uuid)?;
    }
    migrate_title(object, &names, new_draft, recovered, &uuid)?;
    object
        .entry("apparitions")
        .or_insert_with(|| Value::Array(vec![]));
    object
        .entry("description")
        .or_insert_with(|| Value::String(String::new()));
    let state = inferred_state(row);
    row.as_object_mut()
        .unwrap()
        .insert("state".into(), Value::String(state));
    Ok(())
}

fn migrate_date(row: &mut Map<String, Value>, date: Option<i64>, uuid: &str) -> Result<()> {
    let Some(ts) = date else {
        return Ok(());
    };
    if let Some(existing) = row.get(OLD_DATE) {
        let existing = existing
            .as_i64()
            .ok_or_else(|| Error::Validation(format!("{uuid}: invalid apparition date")))?;
        if existing != ts {
            return Err(Error::Validation(format!(
                "{uuid}: conflicting apparition dates"
            )));
        }
    }
    row.insert(OLD_DATE.into(), Value::from(ts));
    Ok(())
}

fn migrate_apparition(app: &mut Value, uuid: &str) -> Result<()> {
    let object = app
        .as_object_mut()
        .ok_or_else(|| Error::Validation(format!("{uuid}: apparition must be an object")))?;
    if let Some(old) = object
        .remove(OLD_TITLE)
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|v| !v.trim().is_empty())
        && is_http_url(&old)
    {
        object.insert("url".into(), Value::String(old));
    }
    object.remove(OLD_DATE);
    Ok(())
}

fn migrate_title(
    row: &mut Map<String, Value>,
    names: &HashSet<String>,
    new_draft: bool,
    recovered: &HashMap<String, String>,
    uuid: &str,
) -> Result<()> {
    let current = row.get("title").and_then(Value::as_str).unwrap_or("");
    if !current.trim().is_empty() {
        if !names.is_empty() && !names.contains(current) {
            return Err(Error::Validation(format!(
                "{uuid}: conflicting legacy titles"
            )));
        }
        return Ok(());
    }
    if let Some(name) = names.iter().next() {
        row.insert("title".into(), Value::String(name.clone()));
    } else if !new_draft {
        let title = recovered
            .get(uuid)
            .filter(|title| !title.trim().is_empty())
            .ok_or_else(|| Error::Validation(format!("{uuid}: recovered title required")))?;
        row.insert("title".into(), Value::String(title.clone()));
    }
    Ok(())
}
