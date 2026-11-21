mod common;
use common::{Fixture, id, row};
use publicator::{
    publish_journal::{PublishJournal, pending_approval},
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
    let config = publicator::config::Config {
        tags: Some("tags.txt".into()),
        ..publicator::config::Config::default()
    };
    let store = publicator::store::Store::new(paths, config).unwrap();
    Fixture {
        root: f.root,
        store,
    }
}
fn job_for(f: &Fixture) -> (PublishJob, publicator::publish_journal::ApprovedSnapshot) {
    let (approved, _) = pending_approval(&f.store, &id(1)).unwrap().unwrap();
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
fn snapshot_resolves_tag_file_under_data_dir_for_external_json() {
    let f = Fixture::new();
    std::fs::create_dir(f.path("sub")).unwrap();
    std::fs::write(f.path("sub/tags.txt"), " first \n\nsecond\n").unwrap();
    std::fs::write(f.path("publicator.toml"), "tags='sub/tags.txt'\n").unwrap();
    let config = publicator::config::Config::load(f.root.path()).unwrap();
    let external = tempfile::tempdir().unwrap();
    let json_path = external.path().join("rows.json");
    let image = f.image("art.png", 4, 4);
    let mut publication = row(1, "DESCRIBED");
    publication["files"][0]["basename"] = json!("art.png");
    publication["files"][0]["sha512sum"] = json!(publicator::images::file_sha512(&image).unwrap());
    publication["apparitionTimestampIfDifferentThanSubmission"] = json!(1_700_002_800);
    publication["apparitions"] = json!([{
        "platformName":"deviantart", "state":"unpublished", "tiers":[], "galleries":[]
    }]);
    std::fs::write(&json_path, json!([publication]).to_string()).unwrap();
    let paths = publicator::config::Paths::resolve(Some(f.root.path()), Some(&json_path)).unwrap();
    let store = publicator::store::Store::new(paths, config).unwrap();
    let (approved, _) = pending_approval(&store, &id(1)).unwrap().unwrap();
    assert_eq!(approved.tags, ["first", "second"]);
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
fn worker_url_reaches_the_store_only_when_valid_and_never_blocks_the_schedule() {
    let deviation = "https://www.deviantart.com/artist/art/Title-1";
    for (sent, stored) in [(deviation, Some(deviation)), ("javascript:alert(1)", None)] {
        let f = publish_fixture();
        let (job, snap) = job_for(&f);
        let j = PublishJournal::new(f.root.path()).unwrap();
        j.begin(&job, &snap).unwrap();
        let mut event = scheduled(&job);
        if let WorkerEvent::Scheduled { url, .. } = &mut event {
            *url = Some(sent.into());
        }
        j.terminal(&event).unwrap();
        j.apply_scheduled(&f.store, &job.attempt_id).unwrap();
        let apparition = &f.read()[0]["apparitions"][0];
        assert_eq!(apparition["state"], "published_or_scheduled");
        assert_eq!(apparition.get("url").and_then(|url| url.as_str()), stored);
    }
}

#[test]
fn applied_delivery_appears_only_once_every_target_succeeded() {
    let f = publish_fixture();
    f.store
        .mutate(|rows| {
            rows[0]["apparitions"][1]["state"] = json!("published_or_scheduled");
            Ok(())
        })
        .unwrap();
    let (job, snap) = job_for(&f);
    let j = PublishJournal::new(f.root.path()).unwrap();
    j.begin(&job, &snap).unwrap();
    j.terminal(&scheduled(&job)).unwrap();
    j.apply_scheduled(&f.store, &job.attempt_id).unwrap();
    assert_eq!(f.read()[0]["state"], "APPEARED");
}

#[test]
fn locked_attempt_snapshot_is_read_only_and_only_unresolved_attempts_block() {
    use publicator::publish_journal::publisher_lock;
    let f = publish_fixture();
    let journal = PublishJournal::new(f.root.path()).unwrap();
    let _lock = publisher_lock(f.root.path()).unwrap();
    assert!(
        journal
            .blocked_attempts_locked(&f.store)
            .unwrap()
            .is_empty()
    );

    let (job, snap) = job_for(&f);
    journal.begin(&job, &snap).unwrap();
    let publication_before = std::fs::read(&f.store.paths().json).unwrap();
    let receipt_path = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        job.attempt_id
    ));
    let receipt_before = std::fs::read(&receipt_path).unwrap();
    assert_eq!(journal.blocked_attempts_locked(&f.store).unwrap().len(), 1);
    assert_eq!(std::fs::read(&receipt_path).unwrap(), receipt_before);
    assert_eq!(
        std::fs::read(&f.store.paths().json).unwrap(),
        publication_before
    );

    journal.terminal(&scheduled(&job)).unwrap();
    assert_eq!(journal.blocked_attempts_locked(&f.store).unwrap().len(), 1);
    journal.applied(&job.attempt_id).unwrap();
    assert!(
        journal
            .blocked_attempts_locked(&f.store)
            .unwrap()
            .is_empty()
    );

    let mut failed_job = job.clone();
    failed_job.attempt_id = uuid::Uuid::new_v4().to_string();
    journal.begin(&failed_job, &snap).unwrap();
    journal
        .terminal(&WorkerEvent::Failed {
            protocol: 1,
            attempt_id: failed_job.attempt_id.clone(),
            uuid: failed_job.uuid.clone(),
            error: "safe failure".into(),
        })
        .unwrap();
    assert!(
        journal
            .blocked_attempts_locked(&f.store)
            .unwrap()
            .is_empty()
    );

    let mut not_submitted_job = job.clone();
    not_submitted_job.attempt_id = uuid::Uuid::new_v4().to_string();
    journal.begin(&not_submitted_job, &snap).unwrap();
    drop(_lock);
    journal
        .reconcile(
            &f.store,
            &not_submitted_job.attempt_id,
            publicator::publish_journal::ReconcileOutcome::NotSubmitted,
        )
        .unwrap();
    let _lock = publisher_lock(f.root.path()).unwrap();
    assert!(
        journal
            .blocked_attempts_locked(&f.store)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn locked_attempt_snapshot_surfaces_busy_and_malformed_receipt() {
    use publicator::publish_journal::publisher_lock;
    let f = publish_fixture();
    let journal = PublishJournal::new(f.root.path()).unwrap();
    let _lock = publisher_lock(f.root.path()).unwrap();
    assert!(matches!(
        publisher_lock(f.root.path()),
        Err(publicator::error::Error::Busy(_))
    ));
    let (job, snap) = job_for(&f);
    journal.begin(&job, &snap).unwrap();
    let receipt = f.path(&format!(
        ".publicator/publish/{}/receipt.json",
        job.attempt_id
    ));
    std::fs::write(&receipt, b"{").unwrap();
    assert!(journal.blocked_attempts_locked(&f.store).is_err());
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
    // The Store write landed but the receipt was never marked applied (crash in between).
    f.store
        .mutate(|rows| {
            rows[0]["apparitions"][0]["state"] = json!("published_or_scheduled");
            Ok(())
        })
        .unwrap();
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

#[test]
fn interrupted_intent_directories_neither_block_nor_hide_real_receipts() {
    use publicator::publish_journal::{ReconcileOutcome, publisher_lock};
    let f = publish_fixture();
    let journal = PublishJournal::new(f.root.path()).unwrap();
    // A crash inside `begin` leaves an attempt directory with no receipt, possibly holding a
    // partial temporary file; nothing was dispatched for it.
    let empty = f.path(".publicator/publish/00000000-0000-4000-8000-000000000098");
    let partial = f.path(".publicator/publish/00000000-0000-4000-8000-000000000099");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join(".tmpAbC123"), b"{\"job\":").unwrap();
    let lock = publisher_lock(f.root.path()).unwrap();
    assert!(journal.recover(&f.store).unwrap().blocked.is_empty());
    assert!(
        journal
            .blocked_attempts_locked(&f.store)
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .attempt_statuses_locked(&f.store)
            .unwrap()
            .is_empty()
    );
    drop(lock);
    let error = journal
        .reconcile(
            &f.store,
            "00000000-0000-4000-8000-000000000098",
            ReconcileOutcome::NotSubmitted,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("nothing was dispatched"),
        "{error}"
    );
    assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);
    assert_eq!(
        std::fs::read(partial.join(".tmpAbC123")).unwrap(),
        b"{\"job\":"
    );

    let (job, snap) = job_for(&f);
    journal.begin(&job, &snap).unwrap();
    assert_eq!(journal.recover(&f.store).unwrap().blocked.len(), 1);
    std::fs::write(
        f.path(&format!(
            ".publicator/publish/{}/receipt.json",
            job.attempt_id
        )),
        b"{",
    )
    .unwrap();
    assert!(journal.recover(&f.store).is_err());
}

#[test]
fn resolved_receipts_accept_only_their_own_decision_and_terminal() {
    use publicator::{error::Error, publish_journal::ReconcileOutcome};
    let f = publish_fixture();
    let journal = PublishJournal::new(f.root.path()).unwrap();
    let durable = |job: &PublishJob| {
        let receipt = format!(".publicator/publish/{}/receipt.json", job.attempt_id);
        (
            std::fs::read(f.path(&receipt)).unwrap(),
            std::fs::read(&f.store.paths().json).unwrap(),
        )
    };
    let conflict = |job: &PublishJob, outcome| {
        assert!(matches!(
            journal.reconcile(&f.store, &job.attempt_id, outcome),
            Err(Error::ReconciliationRequired(_))
        ));
    };
    let unscheduled = ReconcileOutcome::Scheduled { actual_url: None };

    let (released, snap) = job_for(&f);
    journal.begin(&released, &snap).unwrap();
    journal
        .reconcile(
            &f.store,
            &released.attempt_id,
            ReconcileOutcome::NotSubmitted,
        )
        .unwrap();
    let before = durable(&released);
    journal
        .reconcile(
            &f.store,
            &released.attempt_id,
            ReconcileOutcome::NotSubmitted,
        )
        .unwrap();
    conflict(&released, unscheduled.clone());
    assert!(journal.terminal(&scheduled(&released)).is_err());
    assert_eq!(durable(&released), before);
    assert_eq!(f.read()[0]["apparitions"][0]["state"], "unpublished");

    let (applied, snap) = job_for(&f);
    journal.begin(&applied, &snap).unwrap();
    journal.terminal(&scheduled(&applied)).unwrap();
    journal
        .apply_scheduled(&f.store, &applied.attempt_id)
        .unwrap();
    let before = durable(&applied);
    journal.terminal(&scheduled(&applied)).unwrap();
    assert!(
        journal
            .terminal(&WorkerEvent::Failed {
                protocol: 1,
                attempt_id: applied.attempt_id.clone(),
                uuid: applied.uuid.clone(),
                error: "late conflicting terminal".into(),
            })
            .is_err()
    );
    conflict(&applied, ReconcileOutcome::NotSubmitted);
    conflict(
        &applied,
        ReconcileOutcome::Scheduled {
            actual_url: Some("https://www.deviantart.com/example/art/other".into()),
        },
    );
    journal
        .reconcile(&f.store, &applied.attempt_id, unscheduled)
        .unwrap();
    assert_eq!(durable(&applied), before);
}

/// Runs `act` while an unresolved attempt names publication 1: it must be refused with the
/// store unchanged. Then reconciles the attempt and returns `act`'s second result.
fn blocked_until_reconciled<R>(
    act: impl Fn(&publicator::application::Application) -> publicator::error::Result<R>,
) -> (Fixture, std::sync::Arc<publicator::store::Store>, R) {
    let f = publish_fixture();
    let (job, snap) = job_for(&f);
    let journal = PublishJournal::new(f.root.path()).unwrap();
    journal.begin(&job, &snap).unwrap();
    let store = std::sync::Arc::new(
        publicator::store::Store::new(f.store.paths().clone(), f.store.config().clone()).unwrap(),
    );
    let application = common::offline_app(store.clone());
    let before = std::fs::read(&f.store.paths().json).unwrap();
    assert!(matches!(
        act(&application),
        Err(publicator::error::Error::ReconciliationRequired(_))
    ));
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
    journal
        .reconcile(
            &store,
            &job.attempt_id,
            publicator::publish_journal::ReconcileOutcome::NotSubmitted,
        )
        .unwrap();
    let result = act(&application).unwrap();
    (f, store, result)
}

#[test]
fn unresolved_attempt_blocks_rolling_back_its_publication_until_reconciled() {
    let (_f, _, phase) = blocked_until_reconciled(|application| application.roll_back(&id(1)));
    assert_eq!(phase, publicator::model::Phase::Scheduled);
}

#[test]
fn unresolved_attempt_blocks_deleting_its_publication_until_reconciled() {
    let (_f, store, ()) = blocked_until_reconciled(|application| {
        application.delete_publication(&id(1), 1_800_000_000)
    });
    assert!(store.read().unwrap().is_empty());
}
