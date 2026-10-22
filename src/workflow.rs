use crate::{
    config::Config,
    error::{Error, Result},
    images::{Candidate, file_sha512},
    model::{
        GENERATE_MARKER, Phase, deliveries_complete, has_delivery_success, phase, validate_metadata,
    },
    scheduling::{profiles, repack, resolve_wall_clock},
    store::Store,
};
use chrono_tz::Tz;
use serde_json::{Value, json};

pub const REGISTERED_PLATFORMS: &[&str] = &["deviantart"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlatformOptions {
    pub price: Option<Option<f64>>,
    pub tiers: Option<Vec<String>>,
    pub galleries: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DateEdit {
    Preset(String),
    Custom(String),
    Epoch(i64),
    Suggested,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetadataPatch {
    pub title: Option<String>,
    pub description: Option<String>,
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
    pick_with_hooks(store, candidate, now, || {}, || {})
}

fn pick_with_hooks(
    store: &Store,
    candidate: &Candidate,
    now: i64,
    after_hash: impl FnOnce(),
    before_commit: impl FnOnce(),
) -> Result<String> {
    let identity = crate::images::file_identity(&candidate.path)?;
    let sha = file_sha512(&candidate.path)?;
    if candidate.sha512.eq_ignore_ascii_case(&sha) {
        if crate::images::file_identity(&candidate.path)? != identity {
            return Err(Error::InvalidState(
                "candidate changed since discovery".into(),
            ));
        }
    } else {
        return Err(Error::Validation(
            "candidate content changed since discovery".into(),
        ));
    }
    after_hash();
    store.mutate(|rows| {
        if crate::images::file_identity(&candidate.path)? != identity {
            return Err(Error::InvalidState(
                "candidate changed or disappeared since discovery".into(),
            ));
        }
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
        before_commit();
        Ok(uuid)
    })
}

fn require_scheduled_metadata(row: &Value) -> Result<()> {
    if has_delivery_success(row) || phase(row)? != Phase::Scheduled {
        return Err(Error::InvalidState(
            "metadata operations require undelivered SCHEDULED publication".into(),
        ));
    }
    Ok(())
}

pub fn save_metadata(store: &Store, uuid: &str, patch: &MetadataPatch) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_scheduled_metadata(row)?;
        if patch
            .title
            .as_ref()
            .is_some_and(|title| title.chars().count() > 50)
        {
            return Err(Error::Validation("title exceeds 50 characters".into()));
        }
        if let Some(title) = &patch.title {
            row["title"] = json!(title);
        }
        if let Some(description) = &patch.description {
            row["description"] = json!(description);
        }
        Ok(())
    })
}

pub fn approve_metadata(store: &Store, uuid: &str) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_scheduled_metadata(row)?;
        validate_metadata(
            row.get("title").and_then(Value::as_str).unwrap_or_default(),
            row.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )?;
        row["state"] = json!("DESCRIBED");
        Ok(())
    })
}

pub fn save_generated_pair(store: &Store, uuid: &str, pair: &Metadata) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_scheduled_metadata(row)?;
        validate_metadata(&pair.title, &pair.description)?;
        row["title"] = json!(pair.title);
        row["description"] = json!(pair.description);
        row["state"] = json!("DESCRIBED");
        Ok(())
    })
}

pub fn fail_generation(store: &Store, uuid: &str) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_scheduled_metadata(row)?;
        row["title"] = json!(GENERATE_MARKER);
        row["description"] = json!(GENERATE_MARKER);
        Ok(())
    })
}

pub fn merge_generated_markers(store: &Store, uuid: &str, pair: &Metadata) -> Result<()> {
    validate_metadata(&pair.title, &pair.description)?;
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        require_scheduled_metadata(row)?;
        let mut applied = false;
        for (key, value) in [("title", &pair.title), ("description", &pair.description)] {
            if row.get(key).and_then(Value::as_str) == Some(GENERATE_MARKER) {
                row[key] = json!(value);
                applied = true;
            }
        }
        if applied
            && validate_metadata(
                row.get("title").and_then(Value::as_str).unwrap_or_default(),
                row.get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
            .is_ok()
        {
            row["state"] = json!("DESCRIBED");
        }
        Ok(())
    })
}

pub fn return_to_metadata(store: &Store, uuid: &str) -> Result<()> {
    store.mutate(|rows| {
        let row = find_mut(rows, uuid)?;
        if has_delivery_success(row) || phase(row)? != Phase::Described {
            return Err(Error::InvalidState(
                "return to metadata requires undelivered DESCRIBED publication".into(),
            ));
        }
        row["state"] = json!("SCHEDULED");
        Ok(())
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

pub fn select(
    store: &Store,
    uuid: Option<&str>,
    platform: Option<&str>,
    selected: bool,
) -> Result<()> {
    if let Some(platform) = platform
        && !REGISTERED_PLATFORMS.contains(&platform)
    {
        return Err(Error::Validation(format!(
            "unregistered platform: {platform}"
        )));
    }
    if uuid.is_some() != platform.is_some() {
        return Err(Error::Validation(
            "uuid and platform must be provided together".into(),
        ));
    }
    store.mutate(|rows| {
        let selected_rows: Vec<&mut Value> = if let Some(uuid) = uuid {
            vec![find_mut(rows, uuid)?]
        } else {
            rows.iter_mut().collect()
        };
        for pub_ in selected_rows {
            let is_target = matches!(phase(pub_)?, Phase::Described | Phase::Appeared);
            if !is_target {
                if uuid.is_some() {
                    return Err(Error::InvalidState(
                        "platform selection requires DESCRIBED or APPEARED publication".into(),
                    ));
                }
                continue;
            }
            let names: Vec<&str> = platform
                .map(|name| vec![name])
                .unwrap_or_else(|| REGISTERED_PLATFORMS.to_vec());
            for name in names {
                let title = pub_
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let description = pub_
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let profile_name = pub_
                    .get("scheduleProfile")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let apps = pub_
                    .get_mut("apparitions")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| Error::Validation("apparitions must be an array".into()))?;
                let existing = apps
                    .iter()
                    .position(|app| app.get("platformName").and_then(Value::as_str) == Some(name));
                match (selected, existing) {
                    (_, Some(index))
                        if apps[index].get("state").and_then(Value::as_str)
                            == Some("published_or_scheduled") =>
                    {
                        if uuid.is_some() && !selected {
                            return Err(Error::InvalidState(
                                "successful selection is locked".into(),
                            ));
                        }
                    }
                    (true, Some(_)) => {}
                    (true, None) => {
                        validate_metadata(&title, &description)?;
                        let mut app = json!({"platformName": name, "state": "unpublished"});
                        if name == "deviantart"
                            && let Some(profile) =
                                profiles(store.config())?.into_iter().find(|profile| {
                                    Some(profile.name.as_str()) == profile_name.as_deref()
                                })
                        {
                            if !profile.default_tiers.is_empty() {
                                app["tiers"] = json!(profile.default_tiers);
                            }
                            if let Some(price) = profile.default_price {
                                app["priceIfNotFree"] = json!(price);
                            }
                        }
                        apps.push(app);
                        pub_["state"] = json!("DESCRIBED");
                    }
                    (false, Some(index)) => {
                        apps.remove(index);
                    }
                    (false, None) => {}
                }
            }
            if pub_
                .get("apparitions")
                .and_then(Value::as_array)
                .is_some_and(|apps| !apps.is_empty())
            {
                pub_["state"] = if deliveries_complete(pub_) {
                    json!("APPEARED")
                } else {
                    json!("DESCRIBED")
                };
            }
        }
        Ok(())
    })
}

pub fn save_options(store: &Store, uuid: &str, options: PlatformOptions) -> Result<()> {
    if options
        .price
        .flatten()
        .is_some_and(|price| !price.is_finite() || price < 0.0)
    {
        return Err(Error::Validation(
            "price must be a finite nonnegative number".into(),
        ));
    }
    store.mutate(|rows| {
        let pub_ = find_mut(rows, uuid)?;
        if phase(pub_)? != Phase::Described {
            return Err(Error::InvalidState("options require DESCRIBED".into()));
        }
        let apps = pub_
            .get_mut("apparitions")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| Error::Validation("apparitions must be an array".into()))?;
        let app = apps
            .iter_mut()
            .find(|app| app.get("platformName").and_then(Value::as_str) == Some("deviantart"))
            .ok_or_else(|| Error::InvalidState("undelivered selection required".into()))?;
        if app.get("state").and_then(Value::as_str) != Some("unpublished") {
            return Err(Error::InvalidState("undelivered selection required".into()));
        }
        if let Some(price) = options.price {
            if let Some(price) = price {
                app["priceIfNotFree"] = json!(price);
            } else if let Some(object) = app.as_object_mut() {
                object.remove("priceIfNotFree");
            }
        }
        if let Some(tiers) = options.tiers {
            app["tiers"] = json!(tiers);
        }
        if let Some(galleries) = options.galleries {
            app["galleries"] = json!(galleries);
        }
        Ok(())
    })
}

pub fn record_delivery(
    store: &Store,
    uuid: &str,
    platform: &str,
    actual_url: Option<&str>,
) -> Result<()> {
    if !REGISTERED_PLATFORMS.contains(&platform) {
        return Err(Error::Validation(format!(
            "unregistered platform: {platform}"
        )));
    }
    store.mutate(|rows| {
        let pub_ = find_mut(rows, uuid)?;
        let apps = pub_
            .get_mut("apparitions")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| Error::Validation("apparitions must be an array".into()))?;
        let app = apps
            .iter_mut()
            .find(|app| app.get("platformName").and_then(Value::as_str) == Some(platform))
            .ok_or_else(|| Error::NotFound(format!("{uuid}:{platform}")))?;
        if app.get("state").and_then(Value::as_str) != Some("unpublished") {
            return Err(Error::InvalidState("delivery target is not pending".into()));
        }
        app["state"] = json!("published_or_scheduled");
        if let Some(url) = actual_url {
            app["url"] = json!(url);
        }
        pub_["state"] = if deliveries_complete(pub_) {
            json!("APPEARED")
        } else {
            json!("DESCRIBED")
        };
        Ok(())
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

#[cfg(test)]
mod pick_delete_race_tests {
    use super::pick_with_hooks;
    use crate::{
        config::{Config, Paths},
        error::Error,
        images::{Candidate, delete_candidate_with_hooks, file_sha512},
        store::Store,
    };
    use serde_json::json;
    use std::{fs, sync::mpsc, thread};

    fn fixture() -> (tempfile::TempDir, Store, Candidate) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("art")).unwrap();
        let path = root.path().join("art/candidate.png");
        fs::write(&path, b"disposable candidate").unwrap();
        let store = Store::new(
            Paths::resolve(Some(root.path()), None).unwrap(),
            Config {
                publicable: vec!["art".into()],
                ..Config::default()
            },
        )
        .unwrap();
        store
            .mutate(|rows| {
                rows.push(json!({
                    "uuid":"00000000-0000-4000-8000-000000000001",
                    "state":"PICKED", "title":"Title", "description":"Description",
                    "submissionTimestamp":1700000000,
                    "files":[{"basename":"unrelated.png","sha512sum":"0".repeat(128)}],
                    "apparitions":[]
                }));
                Ok(())
            })
            .unwrap();
        let candidate = Candidate {
            path,
            sha512: file_sha512(&root.path().join("art/candidate.png")).unwrap(),
        };
        (root, store, candidate)
    }

    #[test]
    fn delete_finishing_before_pick_admission_prevents_missing_reference_and_keeps_fresh_edits() {
        let (_root, store, candidate) = fixture();
        let (hashed_tx, hashed_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let pick_store = store.clone();
        let pick_candidate = candidate.clone();
        let pick = thread::spawn(move || {
            pick_with_hooks(
                &pick_store,
                &pick_candidate,
                1_700_000_001,
                || {
                    hashed_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                },
                || {},
            )
        });
        hashed_rx.recv().unwrap();

        delete_candidate_with_hooks(
            &store,
            std::slice::from_ref(&candidate),
            &candidate.path,
            || {},
            || {},
        )
        .unwrap();
        assert!(!candidate.path.exists());
        store
            .mutate(|rows| {
                rows[0]["description"] = json!("unrelated concurrent edit");
                Ok(())
            })
            .unwrap();
        resume_tx.send(()).unwrap();
        assert!(matches!(pick.join().unwrap(), Err(Error::Io(_))));
        let rows = store.read().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["description"], "unrelated concurrent edit");
    }

    #[test]
    fn pick_committing_before_delete_keeps_referenced_art_and_unrelated_rows() {
        let (_root, store, candidate) = fixture();
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let (resume_pick_tx, resume_pick_rx) = mpsc::channel();
        let (hashed_delete_tx, hashed_delete_rx) = mpsc::channel();
        let (resume_delete_tx, resume_delete_rx) = mpsc::channel();
        let pick_store = store.clone();
        let pick_candidate = candidate.clone();
        let pick = thread::spawn(move || {
            pick_with_hooks(
                &pick_store,
                &pick_candidate,
                1_700_000_001,
                || {},
                || {
                    admitted_tx.send(()).unwrap();
                    resume_pick_rx.recv().unwrap();
                },
            )
        });
        admitted_rx.recv().unwrap();

        let delete_store = store.clone();
        let delete_candidate = candidate.clone();
        let delete = thread::spawn(move || {
            delete_candidate_with_hooks(
                &delete_store,
                std::slice::from_ref(&delete_candidate),
                &delete_candidate.path,
                || {
                    hashed_delete_tx.send(()).unwrap();
                    resume_delete_rx.recv().unwrap();
                },
                || {},
            )
        });
        hashed_delete_rx.recv().unwrap();
        resume_pick_tx.send(()).unwrap();
        assert!(pick.join().unwrap().is_ok());
        resume_delete_tx.send(()).unwrap();
        assert!(matches!(
            delete.join().unwrap(),
            Err(Error::InvalidState(_))
        ));
        assert!(candidate.path.exists());
        let rows = store.read().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["description"], "Description");
        assert_eq!(rows[1]["files"][0]["basename"], "candidate.png");
    }
}
