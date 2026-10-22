mod common;
use common::{Fixture, epoch, row};
use publicator::{
    images::{Candidate, file_sha512},
    model::Phase,
    workflow::{DateEdit, approve_date, edit_date, pick, return_to_scheduling},
};
use serde_json::{Value, json};

fn candidate(f: &Fixture, name: &str) -> Candidate {
    let path = f.image(name, 10, 10);
    Candidate {
        sha512: file_sha512(&path).unwrap(),
        path,
    }
}

#[test]
fn pick_is_idempotent_by_content_and_creates_no_platform_or_metadata() {
    let f = Fixture::new();
    let c = candidate(&f, "new.png");
    let uuid = pick(&f.store, &c, 1_800_000_000).unwrap();
    assert_eq!(pick(&f.store, &c, 1_800_000_000).unwrap(), uuid);
    let rows = f.store.read().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "PICKED");
    assert_eq!(rows[0]["apparitions"], json!([]));
    assert_eq!(rows[0]["title"], "");
    assert_eq!(rows[0]["description"], "");
    assert_eq!(rows[0]["files"][0]["sha512sum"], c.sha512);
    assert_eq!(rows[0]["submissionTimestamp"], 1_800_000_000);
    assert!(rows[0]["apparitionTimestampIfDifferentThanSubmission"].is_i64());
}

#[test]
fn pick_deduplicates_legacy_hash_case_insensitively_and_keeps_existing_order() {
    let f = Fixture::new();
    let c = candidate(&f, "already.png");
    let mut existing = row(1, "PICKED");
    existing["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("sha512sum");
    existing["files"][0]["fileSha512sum"] = Value::String(c.sha512.to_ascii_uppercase());
    f.write(json!([existing]));
    assert_eq!(
        pick(&f.store, &c, 1_800_000_000).unwrap(),
        "00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(f.store.read().unwrap().len(), 1);
}

#[test]
fn custom_date_pins_without_inventing_a_profile_and_approval_freezes_it() {
    let f = Fixture::new();
    let mut publication = row(1, "PICKED");
    publication["apparitionTimestampIfDifferentThanSubmission"] = json!(1_800_000_000);
    publication["scheduleProfile"] = json!("removed-profile");
    f.write(json!([publication]));
    let custom = epoch("2027-01-01T18:30:00Z");
    edit_date(
        &f.store,
        &common::id(1),
        DateEdit::Custom("2027-01-01T19:30".into()),
        custom,
    )
    .unwrap();
    let saved = &f.store.read().unwrap()[0];
    assert_eq!(
        saved["apparitionTimestampIfDifferentThanSubmission"],
        custom
    );
    assert_eq!(saved["scheduleIsManual"], true);
    assert_eq!(saved["scheduleProfile"], "removed-profile");
    approve_date(&f.store, &common::id(1), custom).unwrap();
    assert_eq!(f.store.read().unwrap()[0]["state"], "SCHEDULED");
    assert!(approve_date(&f.store, &common::id(1), custom).is_err());
}

#[test]
fn approval_requires_date_and_returning_keeps_missing_custom_date_absent() {
    let f = Fixture::new();
    let mut publication = row(1, "PICKED");
    publication
        .as_object_mut()
        .unwrap()
        .remove("apparitionTimestampIfDifferentThanSubmission");
    f.write(json!([publication]));
    assert!(approve_date(&f.store, &common::id(1), 1_800_000_000).is_err());
    let mut scheduled = row(1, "SCHEDULED");
    scheduled
        .as_object_mut()
        .unwrap()
        .remove("apparitionTimestampIfDifferentThanSubmission");
    f.write(json!([scheduled]));
    return_to_scheduling(&f.store, &common::id(1)).unwrap();
    let saved = &f.store.read().unwrap()[0];
    assert_eq!(saved["state"], "PICKED");
    assert!(
        saved
            .get("apparitionTimestampIfDifferentThanSubmission")
            .is_none()
    );
}

#[test]
fn preset_and_suggested_edits_clear_manual_pin_and_reallocate() {
    let f = Fixture::new();
    let mut publication = row(1, "PICKED");
    publication["apparitionTimestampIfDifferentThanSubmission"] = json!(1_800_000_000);
    publication["scheduleIsManual"] = json!(true);
    publication["scheduleProfile"] = json!("removed-profile");
    let historical = json!({
        "platformName":"pixiv",
        "state":"unpublished",
        "url":"https://pixiv.example/art/old",
        "extra":"preserved"
    });
    publication["apparitions"] = json!([historical.clone()]);
    f.write(json!([publication]));
    edit_date(
        &f.store,
        &common::id(1),
        DateEdit::Preset("default".into()),
        1_700_000_000,
    )
    .unwrap();
    let preset = &f.store.read().unwrap()[0];
    assert_eq!(preset["scheduleProfile"], "default");
    assert_eq!(preset["scheduleIsManual"], false);
    assert_ne!(
        preset["apparitionTimestampIfDifferentThanSubmission"],
        1_800_000_000
    );
    assert_eq!(preset["apparitions"], json!([historical.clone()]));
    edit_date(&f.store, &common::id(1), DateEdit::Suggested, 1_700_000_000).unwrap();
    let suggested = &f.store.read().unwrap()[0];
    assert_eq!(suggested["scheduleProfile"], "default");
    assert_eq!(suggested["scheduleIsManual"], false);
    assert_eq!(suggested["apparitions"], json!([historical]));
}

#[test]
fn date_edits_and_backwards_transition_are_locked_after_delivery_success() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["apparitionTimestampIfDifferentThanSubmission"] = json!(1_800_000_000);
    publication["apparitions"] =
        json!([{"platformName":"old-platform","state":"published_or_scheduled"}]);
    f.write(json!([publication]));
    assert!(
        edit_date(
            &f.store,
            &common::id(1),
            DateEdit::Epoch(1_810_000_000),
            1_800_000_000
        )
        .is_err()
    );
    assert!(return_to_scheduling(&f.store, &common::id(1)).is_err());
}

#[test]
fn return_to_scheduling_keeps_described_draft_fields() {
    let f = Fixture::new();
    let mut publication = row(1, "DESCRIBED");
    publication["title"] = json!("Approved title");
    publication["description"] = json!("Approved description");
    f.write(json!([publication]));
    return_to_scheduling(&f.store, &common::id(1)).unwrap();
    let saved = &f.store.read().unwrap()[0];
    assert_eq!(saved["state"], "PICKED");
    assert_eq!(saved["title"], "Approved title");
    assert_eq!(saved["description"], "Approved description");
}

#[test]
fn return_to_scheduling_clears_approval_but_preserves_manual_custom_date() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["apparitionTimestampIfDifferentThanSubmission"] = json!(1_800_000_000);
    publication["scheduleIsManual"] = json!(true);
    publication["scheduleProfile"] = json!("removed-profile");
    f.write(json!([publication]));
    return_to_scheduling(&f.store, &common::id(1)).unwrap();
    let saved = &f.store.read().unwrap()[0];
    assert_eq!(saved["state"], "PICKED");
    assert_eq!(saved["scheduleProfile"], "removed-profile");
    assert_eq!(
        saved["apparitionTimestampIfDifferentThanSubmission"],
        1_800_000_000
    );
    assert_eq!(publicator::model::phase(saved).unwrap(), Phase::Picked);
}
