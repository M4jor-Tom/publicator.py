use crate::{
    config::Config,
    error::{Error, Result},
    images::{Candidate, file_sha512},
    model::{Phase, has_delivery_success, phase},
    scheduling::{profiles, repack, resolve_wall_clock},
    store::Store,
};
use chrono_tz::Tz;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DateEdit {
    Preset(String),
    Custom(String),
    Epoch(i64),
    Suggested,
}

fn find_mut<'a>(rows: &'a mut [Value], uuid: &str) -> Result<&'a mut Value> {
    rows.iter_mut()
        .find(|row| row.get("uuid").and_then(Value::as_str) == Some(uuid))
        .ok_or_else(|| Error::NotFound(uuid.to_owned()))
}

fn schedule_zone(config: &Config) -> Result<Tz> {
    config
        .schedule
        .get("timezone")
        .and_then(toml::Value::as_str)
        .unwrap_or("Europe/Paris")
        .parse()
        .map_err(|_| Error::Config("schedule: invalid timezone".into()))
}

pub fn pick(store: &Store, candidate: &Candidate, now: i64) -> Result<String> {
    let sha = file_sha512(&candidate.path)?;
    let sha = if candidate.sha512.eq_ignore_ascii_case(&sha) {
        sha
    } else {
        return Err(Error::Validation(
            "candidate content changed since discovery".into(),
        ));
    };
    store.mutate(|rows| {
        for row in rows.iter() {
            for files in ["files", "previewFilesIfNotFree"] {
                if row.get(files).and_then(Value::as_array).into_iter().flatten().any(|file| {
                    ["sha512sum", "fileSha512sum"].iter().any(|key| file.get(*key)
                        .and_then(Value::as_str).is_some_and(|value| value.eq_ignore_ascii_case(&sha)))
                }) {
                    return row.get("uuid").and_then(Value::as_str)
                        .map(str::to_owned).ok_or_else(|| Error::Validation("publication UUID missing".into()));
                }
            }
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        rows.push(json!({
            "uuid": uuid, "state":"PICKED", "title":"", "description":"",
            "submissionTimestamp":now,
            "files":[{"basename":candidate.path.file_name().and_then(|n| n.to_str()).unwrap_or_default(),"sha512sum":sha}],
            "apparitions":[]
        }));
        repack(rows, store.config(), now)?;
        Ok(uuid)
    })
}

fn require_editable(row: &Value) -> Result<()> {
    if has_delivery_success(row) || phase(row)? != Phase::Picked {
        return Err(Error::InvalidState(
            "date editing requires undelivered PICKED".into(),
        ));
    }
    Ok(())
}

pub fn edit_date(store: &Store, uuid: &str, edit: DateEdit, now: i64) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_editable(row)?;
        match edit {
            DateEdit::Custom(wall) => {
                let ts = resolve_wall_clock(&wall, schedule_zone(store.config())?)?;
                row["apparitionTimestampIfDifferentThanSubmission"] = json!(ts);
                row["scheduleIsManual"] = json!(true);
            }
            DateEdit::Epoch(ts) => {
                row["apparitionTimestampIfDifferentThanSubmission"] = json!(ts);
                row["scheduleIsManual"] = json!(true);
            }
            DateEdit::Preset(name) => {
                if !profiles(store.config())?
                    .iter()
                    .any(|profile| profile.name == name)
                {
                    return Err(Error::Validation(format!(
                        "unknown schedule profile {name:?}"
                    )));
                }
                row["scheduleProfile"] = json!(name);
                row["scheduleIsManual"] = json!(false);
                row.as_object_mut()
                    .unwrap()
                    .remove("apparitionTimestampIfDifferentThanSubmission");
            }
            DateEdit::Suggested => {
                row["scheduleIsManual"] = json!(false);
                row.as_object_mut()
                    .unwrap()
                    .remove("apparitionTimestampIfDifferentThanSubmission");
                row.as_object_mut().unwrap().remove("scheduleProfile");
            }
        }
        repack(rows, store.config(), now)
    })
}

pub fn approve_date(store: &Store, uuid: &str, now: i64) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_editable(row)?;
        if row
            .get("apparitionTimestampIfDifferentThanSubmission")
            .and_then(Value::as_i64)
            .is_none()
        {
            return Err(Error::InvalidState("date required".into()));
        }
        row["state"] = json!("SCHEDULED");
        repack(rows, store.config(), now)
    })
}

pub fn return_to_scheduling(store: &Store, uuid: &str) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        if has_delivery_success(row) || !matches!(phase(row)?, Phase::Scheduled | Phase::Described)
        {
            return Err(Error::InvalidState(
                "return to scheduling requires undelivered SCHEDULED or DESCRIBED".into(),
            ));
        }
        if row
            .get("apparitionTimestampIfDifferentThanSubmission")
            .and_then(Value::as_i64)
            .is_some()
            && !profiles(store.config())?.iter().any(|profile| {
                row.get("scheduleProfile").and_then(Value::as_str) == Some(profile.name.as_str())
            })
        {
            row["scheduleIsManual"] = json!(true);
        }
        row["state"] = json!("PICKED");
        Ok(())
    })
}
