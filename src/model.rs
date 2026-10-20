use crate::error::{Error, Result};

pub type Publications = Vec<serde_json::Value>;
pub const GENERATE_MARKER: &str = "[[PUBLICATOR:TODO_GENERATE]]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Picked,
    Scheduled,
    Described,
    Appeared,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Unpublished,
    PublishedOrScheduled,
}

pub fn phase(row: &serde_json::Value) -> Result<Phase> {
    match row.get("state").and_then(serde_json::Value::as_str) {
        Some("PICKED") => Ok(Phase::Picked),
        Some("SCHEDULED") => Ok(Phase::Scheduled),
        Some("DESCRIBED") => Ok(Phase::Described),
        Some("APPEARED") => Ok(Phase::Appeared),
        _ => Err(Error::Validation(
            "publication state is missing or invalid".into(),
        )),
    }
}
pub fn effective_schedule(row: &serde_json::Value) -> Option<i64> {
    row.get("apparitionTimestampIfDifferentThanSubmission")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| {
            row.get("submissionTimestamp")
                .and_then(serde_json::Value::as_i64)
        })
}
pub fn has_delivery_success(row: &serde_json::Value) -> bool {
    row.get("apparitions")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|a| {
            a.get("state").and_then(serde_json::Value::as_str) == Some("published_or_scheduled")
        })
}
pub fn deliveries_complete(row: &serde_json::Value) -> bool {
    row.get("apparitions")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|apps| {
            !apps.is_empty()
                && apps.iter().all(|a| {
                    a.get("state").and_then(serde_json::Value::as_str)
                        == Some("published_or_scheduled")
                })
        })
}
pub fn validate_metadata(title: &str, description: &str) -> Result<()> {
    if [title, description]
        .iter()
        .any(|s| s.trim().is_empty() || s.contains(GENERATE_MARKER))
    {
        return Err(Error::Validation(
            "complete unmarked metadata required".into(),
        ));
    }
    if title.chars().count() > 50 {
        return Err(Error::Validation("title exceeds 50 characters".into()));
    }
    Ok(())
}
