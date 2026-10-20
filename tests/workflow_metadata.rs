mod common;
use common::{Fixture, id, row};
use publicator::{
    model::{GENERATE_MARKER, Phase, phase},
    workflow::{
        Metadata, MetadataPatch, approve_metadata, fail_generation, merge_generated_markers,
        return_to_metadata, save_generated_pair, save_metadata,
    },
};
use serde_json::json;
use std::sync::{Arc, Barrier};

#[test]
fn metadata_draft_does_not_approve_until_explicit_approval() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: Some("Draft".into()),
            description: Some("Details".into()),
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["state"], "SCHEDULED");
    assert_eq!(saved[0]["title"], "Draft");
    approve_metadata(&f.store, &id(1)).unwrap();
    assert_eq!(
        phase(&f.store.read().unwrap()[0]).unwrap(),
        Phase::Described
    );
}

#[test]
fn generated_pair_advances_only_with_complete_valid_metadata() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    let pair = Metadata {
        title: "A title".into(),
        description: "A description".into(),
    };
    save_generated_pair(&f.store, &id(1), &pair).unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "A title");
    assert_eq!(phase(&saved[0]).unwrap(), Phase::Described);
}

#[test]
fn accepted_generation_failure_marks_both_fields_without_advancing() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    fail_generation(&f.store, &id(1)).unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], GENERATE_MARKER);
    assert_eq!(saved[0]["description"], GENERATE_MARKER);
    assert_eq!(phase(&saved[0]).unwrap(), Phase::Scheduled);
}

#[test]
fn any_marker_occurrence_and_blank_metadata_block_approval() {
    for (title, description) in [
        ("Title", "text [[PUBLICATOR:TODO_GENERATE]] tail"),
        ("", "Description"),
    ] {
        let f = Fixture::new();
        let mut publication = row(1, "SCHEDULED");
        publication["title"] = json!(title);
        publication["description"] = json!(description);
        f.write(json!([publication]));
        assert!(approve_metadata(&f.store, &id(1)).is_err());
        assert_eq!(f.store.read().unwrap()[0]["state"], "SCHEDULED");
    }
}

#[test]
fn title_limit_counts_unicode_characters_not_utf8_bytes() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("é".repeat(50));
    publication["description"] = json!("Description");
    f.write(json!([publication]));
    approve_metadata(&f.store, &id(1)).unwrap();

    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("é".repeat(51));
    f.write(json!([publication]));
    assert!(approve_metadata(&f.store, &id(1)).is_err());
    assert_eq!(f.store.read().unwrap()[0]["state"], "SCHEDULED");
}

#[test]
fn merge_does_not_overwrite_a_manual_field() {
    let f = Fixture::new();
    let mut p = row(1, "SCHEDULED");
    p["title"] = "Manual".into();
    p["description"] = GENERATE_MARKER.into();
    f.write(json!([p]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "AI".into(),
            description: "Story".into(),
        },
    )
    .unwrap();
    assert_eq!(f.read()[0]["title"], "Manual");
    assert_eq!(f.read()[0]["state"], "DESCRIBED");
}

#[test]
fn provider_result_merges_only_fields_still_marked_after_concurrent_edit() {
    let f = Arc::new(Fixture::new());
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!(GENERATE_MARKER);
    publication["description"] = json!(GENERATE_MARKER);
    f.write(json!([publication]));
    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker = {
        let (f, started, release) = (Arc::clone(&f), Arc::clone(&started), Arc::clone(&release));
        std::thread::spawn(move || {
            started.wait();
            release.wait();
            merge_generated_markers(
                &f.store,
                &id(1),
                &Metadata {
                    title: "AI title".into(),
                    description: "AI description".into(),
                },
            )
            .unwrap();
        })
    };
    started.wait(); // Provider has started and is blocked before returning its generated pair.
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: Some("New manual title".into()),
            description: None,
        },
    )
    .unwrap();
    release.wait();
    worker.join().unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "New manual title");
    assert_eq!(saved[0]["description"], "AI description");
    assert_eq!(saved[0]["state"], "DESCRIBED");
}

#[test]
fn returning_to_metadata_and_editing_are_forbidden_after_delivery() {
    let f = Fixture::new();
    let mut publication = row(1, "DESCRIBED");
    publication["apparitions"] =
        json!([{"platformName":"old-platform","state":"published_or_scheduled"}]);
    f.write(json!([publication]));
    assert!(return_to_metadata(&f.store, &id(1)).is_err());
    assert!(
        save_metadata(
            &f.store,
            &id(1),
            &MetadataPatch {
                title: Some("Changed".into()),
                description: None
            }
        )
        .is_err()
    );
    assert_eq!(f.store.read().unwrap()[0]["title"], "Title");
}

#[test]
fn return_to_metadata_preserves_draft_for_correction_and_reapproval() {
    let f = Fixture::new();
    let mut publication = row(1, "DESCRIBED");
    publication["title"] = json!("Keep title");
    publication["description"] = json!("Keep description");
    f.write(json!([publication]));
    return_to_metadata(&f.store, &id(1)).unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["state"], "SCHEDULED");
    assert_eq!(saved[0]["title"], "Keep title");
    assert_eq!(saved[0]["description"], "Keep description");
    approve_metadata(&f.store, &id(1)).unwrap();
    assert_eq!(f.store.read().unwrap()[0]["state"], "DESCRIBED");
}

#[test]
fn invalid_generated_pair_does_not_advance_or_partially_overwrite() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("Manual title");
    publication["description"] = json!(GENERATE_MARKER);
    f.write(json!([publication]));
    assert!(
        merge_generated_markers(
            &f.store,
            &id(1),
            &Metadata {
                title: "AI".into(),
                description: "".into()
            }
        )
        .is_err()
    );
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "Manual title");
    assert_eq!(saved[0]["description"], GENERATE_MARKER);
    assert_eq!(saved[0]["state"], "SCHEDULED");
}

#[test]
fn metadata_patch_preserves_omitted_field_and_requires_scheduled_state() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: Some("New title".into()),
            description: None,
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "New title");
    assert_eq!(saved[0]["description"], "Description");

    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    assert!(
        save_metadata(
            &f.store,
            &id(1),
            &MetadataPatch {
                title: Some("Not allowed".into()),
                description: None
            }
        )
        .is_err()
    );
}
