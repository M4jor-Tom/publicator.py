use publicator::{config::Config, migration::migrate, model::Publications};
use serde_json::{Value, json};
use std::collections::HashMap;

fn legacy() -> Value {
    json!({"uuid":"00000000-0000-4000-8000-000000000001", "submissionTimestamp":1700000000,
        "description":"d", "files":[{"basename":"a.png", "sha512sum":"0".repeat(128)}],
        "apparitions":[{"platformName":"deviantart", "state":"published_or_scheduled",
            "urlElsePublicationName":"https://www.deviantart.com/user/art/title-123",
            "apparitionTimestampIfDifferentThanSubmission":1800000000}]})
}

fn migrate_rows(
    rows: &Publications,
    recovered: &[(&str, &str)],
) -> publicator::error::Result<Publications> {
    migrate(
        rows,
        &recovered
            .iter()
            .map(|(id, title)| ((*id).to_owned(), (*title).to_owned()))
            .collect(),
        &Config::default(),
    )
}

#[test]
fn migration_moves_shared_date_and_url_and_requires_verified_title() {
    let rows = vec![legacy()];
    let before = rows.clone();
    assert!(migrate_rows(&rows, &[]).is_err());
    assert_eq!(rows, before);
    let out = migrate_rows(
        &rows,
        &[("00000000-0000-4000-8000-000000000001", "Recovered Title")],
    )
    .unwrap();
    assert_eq!(rows, before);
    assert_eq!(out[0]["title"], "Recovered Title");
    assert_eq!(out[0]["state"], "APPEARED");
    assert_eq!(
        out[0]["apparitionTimestampIfDifferentThanSubmission"],
        1800000000
    );
    assert_eq!(
        out[0]["apparitions"][0]["url"],
        "https://www.deviantart.com/user/art/title-123"
    );
    assert!(
        out[0]["apparitions"][0]
            .get("urlElsePublicationName")
            .is_none()
    );
    assert!(
        out[0]["apparitions"][0]
            .get("apparitionTimestampIfDifferentThanSubmission")
            .is_none()
    );
}

#[test]
fn migration_rejects_conflicting_legacy_names_and_dates_without_mutation() {
    let mut row = legacy();
    row["apparitions"] = json!([
        {"platformName":"deviantart","state":"unpublished","urlElsePublicationName":"Name A","apparitionTimestampIfDifferentThanSubmission":1800000000},
        {"platformName":"pixiv","state":"unpublished","urlElsePublicationName":"Name B","apparitionTimestampIfDifferentThanSubmission":1800000600}
    ]);
    let rows = vec![row];
    let before = rows.clone();
    assert!(
        migrate_rows(&rows, &[])
            .unwrap_err()
            .to_string()
            .contains("conflicting legacy titles")
    );
    assert_eq!(rows, before);
    let mut dates = rows.clone();
    dates[0]["apparitions"][1]["urlElsePublicationName"] = json!("Name A");
    let before = dates.clone();
    assert!(
        migrate_rows(&dates, &[])
            .unwrap_err()
            .to_string()
            .contains("conflicting apparition dates")
    );
    assert_eq!(dates, before);
}

#[test]
fn migration_requires_recovery_for_empty_legacy_title_and_accepts_verified_title() {
    for absent in [false, true] {
        let mut row = legacy();
        if absent {
            row["apparitions"][0]
                .as_object_mut()
                .unwrap()
                .remove("urlElsePublicationName");
        } else {
            row["apparitions"][0]["urlElsePublicationName"] = json!("");
        }
        let rows = vec![row];
        let before = rows.clone();
        assert!(
            migrate_rows(&rows, &[])
                .unwrap_err()
                .to_string()
                .contains("recovered title required")
        );
        assert_eq!(rows, before);
        let out = migrate_rows(
            &rows,
            &[("00000000-0000-4000-8000-000000000001", "Verified")],
        )
        .unwrap();
        assert_eq!(out[0]["title"], "Verified");
        assert_eq!(out[0]["state"], "APPEARED");
        assert_eq!(rows, before);
    }
}

#[test]
fn migration_preserves_new_blank_drafts_and_is_idempotent() {
    for state in ["PICKED", "SCHEDULED"] {
        let row = json!({"uuid":"00000000-0000-4000-8000-000000000002", "state":state,
            "title":"", "description":"", "submissionTimestamp":1700000000,
            "files":[{"basename":"a.png", "sha512sum":"0".repeat(128)}], "apparitions":[]});
        let first = migrate_rows(&vec![row.clone()], &[]).unwrap();
        assert_eq!(first, vec![row]);
        assert_eq!(
            migrate(&first, &HashMap::new(), &Config::default()).unwrap(),
            first
        );
    }
}

#[test]
fn legacy_metadata_markers_keep_publication_scheduled() {
    for marker in [
        " [[PUBLICATOR:TODO_GENERATE]] ",
        "before [[PUBLICATOR:TODO_GENERATE]] after",
    ] {
        for field in ["title", "description"] {
            let mut row = legacy();
            row["apparitions"][0]["state"] = json!("unpublished");
            row["title"] = json!("Title");
            row["description"] = json!("Description");
            row[field] = json!(marker);
            row["apparitions"][0]["urlElsePublicationName"] = row["title"].clone();
            let original = vec![row];
            let before = original.clone();
            let out = migrate_rows(&original, &[]).unwrap();
            assert_eq!(original, before);
            assert_eq!(out[0]["state"], "SCHEDULED");
            assert_eq!(out[0][field], marker);
            assert!(
                original[0]["apparitions"][0]
                    .get("urlElsePublicationName")
                    .is_some()
            );
        }
    }
}

#[test]
fn migration_rejects_existing_title_and_url_conflicts_without_mutation() {
    let mut title_conflict = legacy();
    title_conflict["title"] = json!("Different");
    title_conflict["apparitions"][0]["urlElsePublicationName"] = json!("Legacy title");
    let rows = vec![title_conflict];
    let before = rows.clone();
    assert!(
        migrate_rows(&rows, &[])
            .unwrap_err()
            .to_string()
            .contains("conflicting legacy titles")
    );
    assert_eq!(rows, before);

    let mut url_conflict = legacy();
    url_conflict["apparitions"][0]["url"] = json!("https://example.org/other");
    let rows = vec![url_conflict];
    let before = rows.clone();
    assert!(
        migrate_rows(&rows, &[])
            .unwrap_err()
            .to_string()
            .contains("conflicting apparition urls")
    );
    assert_eq!(rows, before);
}

#[test]
fn migration_infers_described_for_pending_legacy_metadata() {
    let mut row = legacy();
    row["apparitions"][0]["state"] = json!("unpublished");
    row["apparitions"][0]["urlElsePublicationName"] = json!("Title");
    let out = migrate_rows(&vec![row], &[]).unwrap();
    assert_eq!(out[0]["state"], "DESCRIBED");
    assert_eq!(out[0]["title"], "Title");
}
