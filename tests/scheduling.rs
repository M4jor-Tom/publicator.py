use chrono::{Timelike, Weekday};
use publicator::config::Config;
use publicator::model::Publications;
use publicator::scheduling::{Profile, profiles, repack, resolve_wall_clock, slots};
use serde_json::json;

fn config(text: &str) -> Config {
    Config {
        schedule: toml::from_str(text).unwrap(),
        tiers: vec!["gold".into()],
        ..Config::default()
    }
}

fn epoch(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
}

#[test]
fn custom_times_match_zoneinfo_fold_zero() {
    let tz = chrono_tz::Europe::Paris;
    assert_eq!(
        resolve_wall_clock("2026-10-25T02:30", tz).unwrap(),
        epoch("2026-10-25T00:30:00Z")
    );
    assert_eq!(
        resolve_wall_clock("2026-03-29T02:30", tz).unwrap(),
        epoch("2026-03-29T01:30:00Z")
    );
}

#[test]
fn flat_and_named_profiles_normalize_with_defaults() {
    let flat = profiles(&config("day = 'friday'\nhour = 18\nper_slot = 1")).unwrap();
    assert_eq!(
        flat,
        vec![Profile {
            name: "default".into(),
            weekday: Weekday::Fri,
            hour: 18,
            per_slot: 1,
            default_tiers: vec![],
            default_price: None
        }]
    );
    let named = profiles(&config(
        "[[profiles]]\nname='paid'\nday='wednesday'\ndefault_tiers=['gold']\ndefault_price=0.0",
    ))
    .unwrap();
    assert_eq!(
        named,
        vec![Profile {
            name: "paid".into(),
            weekday: Weekday::Wed,
            hour: 20,
            per_slot: 2,
            default_tiers: vec!["gold".into()],
            default_price: Some(0.0)
        }]
    );
}

#[test]
fn profiles_reject_empty_collision_invalid_values_and_unknown_tiers() {
    for text in [
        "profiles = []",
        "[[profiles]]\nname='a'\nday='tuesday'\nhour=20\n[[profiles]]\nname='b'\nday='tuesday'\nhour=20",
        "hour = 24",
        "per_slot = 0",
        "default_price = -1.0",
        "day = 4",
        "name = 7",
        "[[profiles]]\ndefault_tiers=['silver']",
    ] {
        assert!(profiles(&config(text)).is_err(), "accepted {text}");
    }
}

#[test]
fn slots_keep_local_wall_clock_across_dst_and_are_bounded_to_52() {
    let profile = Profile {
        name: "tuesday".into(),
        weekday: Weekday::Tue,
        hour: 20,
        per_slot: 2,
        default_tiers: vec![],
        default_price: None,
    };
    let out = slots(
        &profile,
        chrono_tz::Europe::Paris,
        epoch("2026-03-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(out.len(), 52);
    assert!(out.windows(2).all(|w| w[0] < w[1]));
    assert!(out.iter().all(|ts| {
        chrono::DateTime::from_timestamp(*ts, 0)
            .unwrap()
            .with_timezone(&chrono_tz::Europe::Paris)
            .hour()
            == 20
    }));
}

#[test]
fn repack_counts_each_historical_publication_once_and_preserves_manual_pins() {
    let cfg =
        config("timezone='UTC'\n[[profiles]]\nname='default'\nday='tuesday'\nhour=20\nper_slot=1");
    let first = epoch("2026-01-06T20:00:00Z");
    let mut rows: Publications = vec![
        json!({"state":"APPEARED", "submissionTimestamp":first, "apparitions":[{}, {}, {}]}),
        json!({"state":"PICKED", "scheduleIsManual":true, "apparitionTimestampIfDifferentThanSubmission":first}),
        json!({"state":"PICKED", "scheduleProfile":"default", "scheduleIsManual":false}),
    ];
    repack(&mut rows, &cfg, first - 3600).unwrap();
    assert_eq!(
        rows[2]["apparitionTimestampIfDifferentThanSubmission"],
        first + 7 * 86400
    );
    assert_eq!(
        rows[1]["apparitionTimestampIfDifferentThanSubmission"],
        first
    );
}

#[test]
fn repack_keeps_explicit_overcapacity_pins_and_custom_profile_mismatch() {
    let cfg =
        config("timezone='UTC'\n[[profiles]]\nname='default'\nday='tuesday'\nhour=20\nper_slot=1");
    let pin = epoch("2026-01-06T20:00:00Z");
    let mut rows: Publications = vec![
        json!({"state":"PICKED", "scheduleIsManual":true, "scheduleProfile":"Custom", "apparitionTimestampIfDifferentThanSubmission":pin}),
        json!({"state":"PICKED", "scheduleIsManual":true, "scheduleProfile":"Custom", "apparitionTimestampIfDifferentThanSubmission":pin}),
        json!({"state":"PICKED", "scheduleIsManual":false}),
    ];
    repack(&mut rows, &cfg, pin - 3600).unwrap();
    assert_eq!(rows[0]["apparitionTimestampIfDifferentThanSubmission"], pin);
    assert_eq!(rows[1]["apparitionTimestampIfDifferentThanSubmission"], pin);
    assert_eq!(rows[0]["scheduleProfile"], "Custom");
    assert_eq!(rows[1]["scheduleProfile"], "Custom");
    assert_eq!(
        rows[2]["apparitionTimestampIfDifferentThanSubmission"],
        pin + 7 * 86400
    );
}

#[test]
fn repack_clears_automatic_suggestions_after_the_52_slot_horizon() {
    let cfg =
        config("timezone='UTC'\n[[profiles]]\nname='default'\nday='tuesday'\nhour=20\nper_slot=1");
    let now = epoch("2026-01-01T00:00:00Z");
    let pinned = slots(
        &Profile {
            name: "default".into(),
            weekday: Weekday::Tue,
            hour: 20,
            per_slot: 1,
            default_tiers: vec![],
            default_price: None,
        },
        chrono_tz::UTC,
        now,
    )
    .unwrap();
    let mut rows: Publications = pinned
        .iter()
        .map(|ts| json!({"state":"SCHEDULED", "apparitionTimestampIfDifferentThanSubmission":ts}))
        .collect();
    rows.push(json!({"state":"PICKED", "scheduleIsManual":false, "apparitionTimestampIfDifferentThanSubmission":1}));
    repack(&mut rows, &cfg, now).unwrap();
    assert!(
        rows.last()
            .unwrap()
            .get("apparitionTimestampIfDifferentThanSubmission")
            .is_none()
    );
}

#[test]
fn repack_rejects_an_invalid_timezone_type() {
    let cfg = config("timezone = 7");
    let mut rows = vec![];
    assert!(repack(&mut rows, &cfg, 1).is_err());
}

#[test]
fn resolve_wall_clock_ignores_embedded_offset() {
    assert_eq!(
        resolve_wall_clock("2026-10-15T20:00+09:00", chrono_tz::Europe::Paris).unwrap(),
        epoch("2026-10-15T18:00:00Z")
    );
    assert!(resolve_wall_clock("2026-10-15T20:00garbage", chrono_tz::Europe::Paris).is_err());
}
