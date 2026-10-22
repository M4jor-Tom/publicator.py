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
    let before = std::fs::read(&f.store.paths().json).unwrap();
    assert!(
        save_metadata(
            &f.store,
            &id(1),
            &MetadataPatch {
                title: Some("é".repeat(51)),
                description: None,
            },
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
}

#[test]
fn saving_an_embedded_marker_keeps_a_draft_that_cannot_be_approved() {
    let f = Fixture::new();
    f.write(json!([row(1, "SCHEDULED")]));
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: Some("[[PUBLICATOR:TODO_GENERATE]] embedded".into()),
            description: None,
        },
    )
    .unwrap();
    let saved = &f.store.read().unwrap()[0];
    assert_eq!(saved["title"], "[[PUBLICATOR:TODO_GENERATE]] embedded");
    assert_eq!(saved["state"], "SCHEDULED");
    let before_approval = std::fs::read(&f.store.paths().json).unwrap();
    assert!(approve_metadata(&f.store, &id(1)).is_err());
    assert_eq!(
        std::fs::read(&f.store.paths().json).unwrap(),
        before_approval
    );
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
fn generated_title_preserves_a_manual_description() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!(GENERATE_MARKER);
    publication["description"] = json!("Manual description");
    f.write(json!([publication]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "Generated".into(),
            description: "Ignored stale description".into(),
        },
    )
    .unwrap();
    let saved = f.read();
    assert_eq!(saved[0]["title"], "Generated");
    assert_eq!(saved[0]["description"], "Manual description");
    assert_eq!(saved[0]["state"], "DESCRIBED");
}

#[test]
fn valid_generated_field_persists_beside_a_blank_manual_title_without_approving() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("");
    publication["description"] = json!(GENERATE_MARKER);
    f.write(json!([publication]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "AI title".into(),
            description: "AI description".into(),
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "");
    assert_eq!(saved[0]["description"], "AI description");
    assert_eq!(saved[0]["state"], "SCHEDULED");
}

#[test]
fn valid_generated_field_persists_beside_a_blank_manual_description_without_approving() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!(GENERATE_MARKER);
    publication["description"] = json!("");
    f.write(json!([publication]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "AI title".into(),
            description: "AI description".into(),
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "AI title");
    assert_eq!(saved[0]["description"], "");
    assert_eq!(saved[0]["state"], "SCHEDULED");
}

#[test]
fn valid_generated_field_preserves_nonexact_marker_manual_counterpart() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("manual [[PUBLICATOR:TODO_GENERATE]] text");
    publication["description"] = json!(GENERATE_MARKER);
    f.write(json!([publication]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "AI title".into(),
            description: "AI description".into(),
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(
        saved[0]["title"],
        "manual [[PUBLICATOR:TODO_GENERATE]] text"
    );
    assert_eq!(saved[0]["description"], "AI description");
    assert_eq!(saved[0]["state"], "SCHEDULED");
}

#[test]
fn stale_generated_result_with_no_markers_never_approves_manual_draft() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!("Manual title");
    publication["description"] = json!("Manual description");
    f.write(json!([publication]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "Stale AI title".into(),
            description: "Stale AI description".into(),
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "Manual title");
    assert_eq!(saved[0]["description"], "Manual description");
    assert_eq!(saved[0]["state"], "SCHEDULED");
}

#[test]
fn merging_both_markers_with_valid_pair_advances_to_described() {
    let f = Fixture::new();
    let mut publication = row(1, "SCHEDULED");
    publication["title"] = json!(GENERATE_MARKER);
    publication["description"] = json!(GENERATE_MARKER);
    f.write(json!([publication]));
    merge_generated_markers(
        &f.store,
        &id(1),
        &Metadata {
            title: "AI title".into(),
            description: "AI description".into(),
        },
    )
    .unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "AI title");
    assert_eq!(saved[0]["description"], "AI description");
    assert_eq!(saved[0]["state"], "DESCRIBED");
    let before_reapproval = std::fs::read(&f.store.paths().json).unwrap();
    assert!(approve_metadata(&f.store, &id(1)).is_err());
    assert_eq!(
        std::fs::read(&f.store.paths().json).unwrap(),
        before_reapproval
    );
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
fn provider_result_preserves_a_newer_manual_description_and_generates_title() {
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
                    description: "Stale AI description".into(),
                },
            )
            .unwrap();
        })
    };
    started.wait();
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: None,
            description: Some("Concurrent draft".into()),
        },
    )
    .unwrap();
    release.wait();
    worker.join().unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "AI title");
    assert_eq!(saved[0]["description"], "Concurrent draft");
    assert_eq!(saved[0]["state"], "DESCRIBED");
}

#[test]
fn blocked_provider_persists_remaining_marked_field_after_blank_manual_edit() {
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
    started.wait();
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: Some("".into()),
            description: None,
        },
    )
    .unwrap();
    release.wait();
    worker.join().unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "");
    assert_eq!(saved[0]["description"], "AI description");
    assert_eq!(saved[0]["state"], "SCHEDULED");
}

#[test]
fn blocked_stale_provider_result_does_not_approve_manual_edits() {
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
                    title: "Stale AI title".into(),
                    description: "Stale AI description".into(),
                },
            )
            .unwrap();
        })
    };
    started.wait();
    save_metadata(
        &f.store,
        &id(1),
        &MetadataPatch {
            title: Some("New manual title".into()),
            description: Some("New manual description".into()),
        },
    )
    .unwrap();
    release.wait();
    worker.join().unwrap();
    let saved = f.store.read().unwrap();
    assert_eq!(saved[0]["title"], "New manual title");
    assert_eq!(saved[0]["description"], "New manual description");
    assert_eq!(saved[0]["state"], "SCHEDULED");
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
