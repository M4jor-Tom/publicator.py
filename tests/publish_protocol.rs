use publicator::publish_protocol::{
    PublishJob, ScheduleParts, SessionOpen, WorkerCommand, WorkerEvent,
};
use std::path::PathBuf;

#[test]
fn decodes_open_submit_and_close_protocol_frames() {
    let open = r#"{"protocol":1,"type":"open","login_profile":"/data/.deviantart-login","work_profile":"/data/.deviantart-session","headless":false,"timezone":"Europe/Paris"}"#;
    let command: WorkerCommand = serde_json::from_str(open).unwrap();
    assert_eq!(
        command,
        WorkerCommand::Open(SessionOpen {
            protocol: 1,
            login_profile: PathBuf::from("/data/.deviantart-login"),
            work_profile: PathBuf::from("/data/.deviantart-session"),
            headless: false,
            timezone: "Europe/Paris".into()
        })
    );
    let submit = r#"{"protocol":1,"type":"submit","attempt_id":"00000000-0000-4000-8000-000000000999","uuid":"00000000-0000-4000-8000-000000000001","image_path":"/data/.publicator/publish/00000000-0000-4000-8000-000000000999/upload.png","title":"Title","description":"Story","tags":["art"],"schedule":{"epoch":1800039600,"timezone":"Europe/Paris","year":2027,"month":1,"day":15,"hour":20,"minute":0},"price":0,"tiers":["Gold"],"galleries":["Featured"]}"#;
    let command: WorkerCommand = serde_json::from_str(submit).unwrap();
    let WorkerCommand::Submit(job) = command else {
        panic!("submit decoded as another command")
    };
    assert_eq!(job.protocol, 1);
    assert_eq!(job.price, Some(0.0));
    assert_eq!(
        job.schedule,
        ScheduleParts {
            epoch: 1800039600,
            timezone: "Europe/Paris".into(),
            year: 2027,
            month: 1,
            day: 15,
            hour: 20,
            minute: 0
        }
    );
    assert_eq!(
        serde_json::from_str::<WorkerCommand>(r#"{"protocol":1,"type":"close"}"#).unwrap(),
        WorkerCommand::Close { protocol: 1 }
    );
}

#[test]
fn rejects_wrong_versions_unknown_fields_and_invalid_schedule_parts() {
    assert!(
        serde_json::from_str::<WorkerCommand>(r#"{"protocol":2,"type":"close"}"#)
            .unwrap()
            .validate()
            .is_err()
    );
    assert!(
        serde_json::from_str::<WorkerCommand>(r#"{"protocol":1,"type":"close","surprise":true}"#)
            .is_err()
    );
    let invalid = r#"{"protocol":1,"type":"submit","attempt_id":"a","uuid":"b","image_path":"x.png","title":"t","description":"d","tags":[],"schedule":{"epoch":1,"timezone":"UTC","year":2027,"month":1,"day":15,"hour":20,"minute":1},"tiers":[],"galleries":[]}"#;
    assert!(
        serde_json::from_str::<WorkerCommand>(invalid)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[test]
fn bounded_frame_reader_rejects_oversized_line() {
    let oversized = format!("{}\\n", " ".repeat(1024 * 1024));
    let mut reader = std::io::Cursor::new(oversized.as_bytes());
    assert!(
        publicator::publish_protocol::read_frame(&mut reader)
            .unwrap_err()
            .contains("1 MiB")
    );
}

#[test]
fn submit_accepts_first_fold_occurrence_matching_schedule_fold_zero() {
    let job = PublishJob {
        protocol: 1,
        attempt_id: "00000000-0000-4000-8000-000000000999".into(),
        uuid: "00000000-0000-4000-8000-000000000001".into(),
        image_path: PathBuf::from("x.png"),
        title: "Title".into(),
        description: "Description".into(),
        tags: vec![],
        schedule: ScheduleParts {
            epoch: 1_793_509_200,
            timezone: "America/New_York".into(),
            year: 2026,
            month: 11,
            day: 1,
            hour: 1,
            minute: 0,
        },
        price: None,
        tiers: vec![],
        galleries: vec![],
    };
    assert!(WorkerCommand::Submit(job).validate().is_ok());
}
#[test]
fn events_are_explicitly_versioned_and_keep_zero_price_distinct() {
    let job = PublishJob {
        protocol: 1,
        attempt_id: "a".into(),
        uuid: "b".into(),
        image_path: PathBuf::from("x.png"),
        title: "t".into(),
        description: "d".into(),
        tags: vec![],
        schedule: ScheduleParts {
            epoch: 1800039600,
            timezone: "Europe/Paris".into(),
            year: 2027,
            month: 1,
            day: 15,
            hour: 20,
            minute: 0,
        },
        price: Some(0.0),
        tiers: vec![],
        galleries: vec![],
    };
    let event = WorkerEvent::Scheduled {
        protocol: 1,
        attempt_id: job.attempt_id.clone(),
        uuid: job.uuid.clone(),
        url: None,
    };
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        serde_json::json!({"protocol":1,"type":"scheduled","attempt_id":"a","uuid":"b"})
    );
    assert_eq!(job.price, Some(0.0));
    let without_price = PublishJob { price: None, ..job };
    assert!(
        serde_json::to_value(without_price)
            .unwrap()
            .get("price")
            .is_none()
    );
}
