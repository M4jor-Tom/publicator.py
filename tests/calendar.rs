use chrono::{NaiveDate, TimeZone};
use chrono_tz::Europe::Paris;
use publicator::{
    calendar::{months, timeline},
    model::{Delivery, Publications},
};
use serde_json::json;

fn row(n: usize, title: &str, timestamp: i64) -> serde_json::Value {
    json!({"uuid":format!("row-{n}"),"state":"DESCRIBED","title":title,"description":"d",
        "submissionTimestamp":timestamp,"files":[{"basename":"art.png","sha512sum":"a".repeat(128)}],
        "apparitions":[]})
}

#[test]
fn calendar_keeps_two_apparitions_with_one_shared_title() {
    let mut p = row(1, "Shared title", 1700000000);
    p["apparitions"] = json!([
        {"platformName":"deviantart","state":"unpublished","url":"https://da.example/artwork/123"},
        {"platformName":"pixiv","state":"published_or_scheduled","url":"https://pixiv.example/artwork/123"}
    ]);
    let ev = timeline(&vec![p]).unwrap();
    assert_eq!(ev.len(), 2);
    assert_ne!(ev[0].apparition_index, ev[1].apparition_index);
    assert_eq!(ev[0].title, "Shared title");
    assert_eq!(ev[1].title, "Shared title");
    assert_eq!(ev[0].platform, "deviantart");
    assert_eq!(ev[1].platform, "pixiv");
    assert_eq!(ev[0].url.as_deref(), Some("https://da.example/artwork/123"));
    assert_eq!(
        ev[1].url.as_deref(),
        Some("https://pixiv.example/artwork/123")
    );
    assert_eq!(ev[0].delivery, Delivery::Unpublished);
    assert_eq!(ev[1].delivery, Delivery::PublishedOrScheduled);
    assert_eq!(ev[0].basename.as_deref(), Some("art.png"));
    let expected_hash = "a".repeat(128);
    assert_eq!(ev[0].sha512.as_deref(), Some(expected_hash.as_str()));
}

#[test]
fn calendar_labels_marker_metadata_and_preserves_file_name() {
    let mut p = row(1, "real", 1700000000);
    p["title"] = json!("[[PUBLICATOR:TODO_GENERATE]] title");
    p["files"][0] = json!({"basename":"old.png", "fileSha512sum":"legacy"});
    p["apparitions"] = json!([{"platformName":"other","state":"unpublished"}]);
    let event = &timeline(&vec![p]).unwrap()[0];
    assert_eq!(event.title, "AI metadata pending");
    assert_eq!(event.url, None);
    assert_eq!(event.basename.as_deref(), Some("old.png"));
    assert_eq!(event.sha512.as_deref(), Some("legacy"));
    assert_eq!(event.timestamp, 1700000000);
}

#[test]
fn calendar_orders_events_and_months_include_now() {
    let mut later = row(
        2,
        "Later",
        Paris
            .with_ymd_and_hms(2026, 3, 3, 20, 0, 0)
            .unwrap()
            .timestamp(),
    );
    later["apparitions"] = json!([{"platformName":"pixiv","state":"unpublished"}]);
    let mut earlier = row(
        1,
        "Earlier",
        Paris
            .with_ymd_and_hms(2025, 11, 3, 20, 0, 0)
            .unwrap()
            .timestamp(),
    );
    earlier["apparitions"] =
        json!([{"platformName":"historical","state":"published_or_scheduled"}]);
    let rows: Publications = vec![later, earlier];
    let events = timeline(&rows).unwrap();
    assert_eq!(
        events.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
        vec!["Earlier", "Later"]
    );
    let now = Paris
        .with_ymd_and_hms(2026, 1, 6, 12, 0, 0)
        .unwrap()
        .timestamp();
    assert_eq!(
        months(&events, Paris, now).unwrap(),
        vec![
            NaiveDate::from_ymd_opt(2025, 11, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        ]
    );
    assert_eq!(
        months(&[], Paris, now).unwrap(),
        vec![NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()]
    );
}

#[test]
fn calendar_falls_back_to_submission_time_and_handles_publications_without_files() {
    let mut p = row(1, "Fallback", 1700000000);
    p.as_object_mut()
        .unwrap()
        .remove("apparitionTimestampIfDifferentThanSubmission");
    p["files"] = json!([]);
    p["apparitions"] = json!([{"platformName":"deviantart","state":"unpublished"}]);
    let event = &timeline(&[p]).unwrap()[0];
    assert_eq!(event.timestamp, 1700000000);
    assert_eq!(event.basename, None);
    assert_eq!(event.sha512, None);
    assert_eq!(event.url, None);
}

#[test]
fn calendar_keeps_equal_timestamp_apparitions_in_source_order() {
    let mut first = row(1, "First", 1700000000);
    first["apparitions"] = json!([
        {"platformName":"first-platform","state":"unpublished"},
        {"platformName":"second-platform","state":"unpublished"}
    ]);
    let mut second = row(2, "Second", 1700000000);
    second["apparitions"] = json!([{"platformName":"third-platform","state":"unpublished"}]);
    let events = timeline(&[first, second]).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.platform.as_str())
            .collect::<Vec<_>>(),
        ["first-platform", "second-platform", "third-platform"]
    );
}

#[test]
fn calendar_treats_a_publication_without_apparitions_as_empty() {
    let mut p = row(1, "No delivery", 1700000000);
    p.as_object_mut().unwrap().remove("apparitions");
    assert!(timeline(&vec![p]).unwrap().is_empty());
}

#[test]
fn calendar_uses_publication_schedule_for_every_platform() {
    let mut p = row(1, "Shared", 1700000000);
    p["apparitionTimestampIfDifferentThanSubmission"] = json!(
        Paris
            .with_ymd_and_hms(2026, 2, 1, 0, 30, 0)
            .unwrap()
            .timestamp()
    );
    p["apparitions"] = json!([{"platformName":"one","state":"unpublished"},{"platformName":"two","state":"unpublished"}]);
    let events = timeline(&vec![p]).unwrap();
    assert_eq!(events[0].timestamp, events[1].timestamp);
    assert_eq!(
        events[0].timestamp,
        Paris
            .with_ymd_and_hms(2026, 2, 1, 0, 30, 0)
            .unwrap()
            .timestamp()
    );
}
