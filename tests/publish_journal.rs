mod common;
use common::{Fixture, id, row};
use publicator::{
    publish_journal::{PublishJournal, snapshot},
    publish_protocol::{PublishJob, ScheduleParts, WorkerEvent},
};
use serde_json::json;

fn publish_fixture() -> Fixture {
    let f = Fixture::new();
    let mut item = row(1, "DESCRIBED");
    item["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    item["apparitions"] = json!([{"platformName":"deviantart","state":"unpublished","tiers":[],"galleries":[]},{"platformName":"historical-platform","state":"unpublished"}]);
    let image = f.image("1.png", 3, 2);
    item["files"][0]["basename"] = json!("1.png");
    item["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
    f.write(json!([item]));
    std::fs::write(f.path("tags.txt"), "one\ntwo\n").unwrap();
    let paths = publicator::config::Paths::resolve(Some(f.root.path()), None).unwrap();
    let mut config = publicator::config::Config::default();
    config.tags = Some("tags.txt".into());
    let store = publicator::store::Store::new(paths, config).unwrap();
    Fixture {
        root: f.root,
        store,
    }
}
fn job_for(f: &Fixture) -> (PublishJob, publicator::publish_journal::ApprovedSnapshot) {
    let approved = snapshot(&f.store, &id(1)).unwrap();
    let job = PublishJob {
        protocol: 1,
        attempt_id: uuid::Uuid::new_v4().to_string(),
        uuid: id(1),
        image_path: f.path("1.png"),
        title: approved.title.clone(),
        description: approved.description.clone(),
        tags: approved.tags.clone(),
        schedule: ScheduleParts {
            epoch: approved.schedule_epoch,
            timezone: "UTC".into(),
            year: 2023,
            month: 11,
            day: 14,
            hour: 23,
            minute: 0,
        },
        price: approved.price,
        tiers: approved.tiers.clone(),
        galleries: approved.galleries.clone(),
    };
    (job, approved)
}
fn scheduled(job: &PublishJob) -> WorkerEvent {
    WorkerEvent::Scheduled {
        protocol: 1,
        attempt_id: job.attempt_id.clone(),
        uuid: job.uuid.clone(),
        url: None,
    }
}

#[test]
fn success_receipt_replay_does_not_need_a_browser() {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let j = PublishJournal::new(f.root.path()).unwrap();
    j.begin(&job, &snap).unwrap();
    j.terminal(&scheduled(&job)).unwrap();
    assert_eq!(
        j.recover(&f.store).unwrap().replayed,
        vec![job.attempt_id.clone()]
    );
    assert!(j.recover(&f.store).unwrap().replayed.is_empty());
    assert_eq!(
        f.read()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert!(
        f.read()[0]["apparitions"][0].get("url").is_none(),
        "missing worker URL must not be invented"
    );
    assert_eq!(
        f.read()[0]["state"],
        "DESCRIBED",
        "historical pending platforms remain pending"
    );
}

#[test]
fn pending_attempt_blocks_automatic_retry_and_explicit_not_submitted_releases_it() {
    use publicator::publish_journal::ReconcileOutcome;
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    let report = journal.recover(&f.store).unwrap();
    assert!(report.replayed.is_empty());
    assert_eq!(report.blocked.len(), 1);
    journal
        .reconcile(&f.store, &job.attempt_id, ReconcileOutcome::NotSubmitted)
        .unwrap();
    assert!(journal.recover(&f.store).unwrap().blocked.is_empty());
    assert_eq!(f.read()[0]["apparitions"][0]["state"], "unpublished");
}

#[test]
fn changed_approved_content_blocks_scheduled_replay() {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    journal.terminal(&scheduled(&job)).unwrap();
    f.store
        .mutate(|rows| {
            rows[0]["title"] = json!("New approved title");
            Ok(())
        })
        .unwrap();
    let report = journal.recover(&f.store).unwrap();
    assert!(report.replayed.is_empty());
    assert_eq!(report.blocked.len(), 1);
    assert_eq!(f.read()[0]["apparitions"][0]["state"], "unpublished");
}

#[test]
fn recorded_store_success_is_replayed_once_even_with_historical_pending_platform() {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    journal.terminal(&scheduled(&job)).unwrap();
    publicator::workflow::record_delivery(&f.store, &job.uuid, "deviantart", None).unwrap();
    assert_eq!(
        journal.recover(&f.store).unwrap().replayed,
        vec![job.attempt_id.clone()]
    );
    assert!(journal.recover(&f.store).unwrap().replayed.is_empty());
    let value = f.read();
    assert_eq!(
        value[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
    assert_eq!(value[0]["apparitions"][1]["state"], "unpublished");
    assert_eq!(value[0]["state"], "DESCRIBED");
}

#[test]
fn receipt_for_external_database_never_replays_into_another_json_file() {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    journal.terminal(&scheduled(&job)).unwrap();
    let other_path = f.path("other.json");
    let paths = publicator::config::Paths::resolve(Some(f.root.path()), Some(&other_path)).unwrap();
    let other = publicator::store::Store::new(
        paths,
        publicator::config::Config {
            tags: Some("tags.txt".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(journal.recover(&other).unwrap().blocked.len(), 1);
    assert!(other.read().unwrap().is_empty());
    assert!(journal.apply_scheduled(&other, &job.attempt_id).is_err());
}

#[test]
fn terminal_receipt_survives_a_store_read_failure_for_later_replay() {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    journal.terminal(&scheduled(&job)).unwrap();
    let saved = std::fs::read(&f.store.paths().json).unwrap();
    std::fs::write(&f.store.paths().json, b"{").unwrap();
    assert!(journal.apply_scheduled(&f.store, &job.attempt_id).is_err());
    std::fs::write(&f.store.paths().json, saved).unwrap();
    assert_eq!(
        journal.recover(&f.store).unwrap().replayed,
        vec![job.attempt_id]
    );
}

#[test]
fn reconciliation_refuses_a_held_publisher_lock_without_rewriting_receipt() {
    use publicator::publish_journal::{ReconcileOutcome, publisher_lock};
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    let before = std::fs::read(f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        job.attempt_id
    )))
    .unwrap();
    let _lock = publisher_lock(f.root.path()).unwrap();
    assert!(matches!(
        journal.reconcile(&f.store, &job.attempt_id, ReconcileOutcome::NotSubmitted),
        Err(publicator::error::Error::Busy(_))
    ));
    assert_eq!(
        std::fs::read(f.path(&format!(
            ".publicator/publish/{}/receipt.json",
            job.attempt_id
        )))
        .unwrap(),
        before
    );
    assert_eq!(f.read()[0]["apparitions"][0]["state"], "unpublished");
}

#[test]
fn human_scheduled_reconciliation_resolves_unknown_and_is_idempotent() {
    use publicator::publish_journal::ReconcileOutcome;
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    journal
        .terminal(&WorkerEvent::Unknown {
            protocol: 1,
            attempt_id: job.attempt_id.clone(),
            uuid: job.uuid.clone(),
            error: "lost confirmation".into(),
        })
        .unwrap();
    let outcome = ReconcileOutcome::Scheduled {
        actual_url: Some("https://www.deviantart.com/example/art/work".into()),
    };
    journal
        .reconcile(&f.store, &job.attempt_id, outcome.clone())
        .unwrap();
    journal
        .reconcile(&f.store, &job.attempt_id, outcome)
        .unwrap();
    assert_eq!(
        f.read()[0]["apparitions"][0]["url"],
        "https://www.deviantart.com/example/art/work"
    );
    assert_eq!(
        f.read()[0]["apparitions"][0]["state"],
        "published_or_scheduled"
    );
}

#[cfg(unix)]
#[test]
fn receipt_identity_must_match_its_attempt_directory() {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    let path = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        job.attempt_id
    ));
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    receipt["job"]["attempt_id"] = json!("00000000-0000-4000-8000-000000000099");
    std::fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    assert!(journal.applied(&job.attempt_id).is_err());
}

#[cfg(unix)]
#[test]
fn attempt_directory_symlinks_cannot_redirect_receipt_reads() {
    use std::os::unix::fs::symlink;
    let f = publish_fixture();
    let journal = PublishJournal::new(f.root.path()).unwrap();
    let outside = f.path("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir_all(f.path(".publicator/publish")).unwrap();
    symlink(
        &outside,
        f.path(".publicator/publish/00000000-0000-4000-8000-000000000099"),
    )
    .unwrap();
    assert!(
        journal
            .applied("00000000-0000-4000-8000-000000000099")
            .is_err()
    );
}

#[test]
fn unsafe_attempt_ids_and_empty_receipts_are_rejected() {
    let f = publish_fixture();
    let journal = PublishJournal::new(f.root.path()).unwrap();
    assert!(journal.applied("../escape").is_err());
    let empty = f.path(".publicator/publish/00000000-0000-4000-8000-000000000099");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("receipt.json"), []).unwrap();
    assert!(journal.recover(&f.store).is_err());
}
