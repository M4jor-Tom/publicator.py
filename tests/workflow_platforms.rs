mod common;
use common::{Fixture, id, row};
use publicator::{
    config::Config,
    workflow::{PlatformOptions, REGISTERED_PLATFORMS, record_delivery, save_options, select},
};
use serde_json::json;

#[test]
fn historical_pending_prevents_appeared() {
    let f = Fixture::new();
    let mut p = row(1, "DESCRIBED");
    p["apparitions"] = json!([
        {"platformName":"pixiv","state":"unpublished"},
        {"platformName":"deviantart","state":"unpublished"}
    ]);
    f.write(json!([p]));
    record_delivery(&f.store, &id(1), "deviantart", None).unwrap();
    assert_eq!(f.read()[0]["state"], "DESCRIBED");
}

#[test]
fn registry_and_select_all_add_only_deviantart_in_one_mutation() {
    let f = Fixture::new();
    f.write(json!([row(1, "DESCRIBED"), row(2, "SCHEDULED")]));
    assert_eq!(REGISTERED_PLATFORMS, &["deviantart"]);
    select(&f.store, None, None, true).unwrap();
    select(&f.store, Some(&id(1)), Some("deviantart"), true).unwrap();
    let saved = f.read();
    assert_eq!(
        saved[0]["apparitions"],
        json!([
            {"platformName":"deviantart","state":"unpublished"}
        ])
    );
    assert_eq!(saved[1]["apparitions"], json!([]));
}

#[test]
fn deselect_all_leaves_historical_pending_cells_untouched() {
    let f = Fixture::new();
    let mut p = row(1, "DESCRIBED");
    p["apparitions"] = json!([{"platformName":"pixiv","state":"unpublished"}]);
    f.write(json!([p]));
    let before = f.read();
    select(&f.store, None, None, false).unwrap();
    assert_eq!(f.read(), before);
}

#[test]
fn selection_reopens_appeared_and_successful_cell_is_immutable() {
    let f = Fixture::new();
    let mut p = row(1, "APPEARED");
    p["apparitions"] = json!([
        {"platformName":"deviantart","state":"published_or_scheduled"}
    ]);
    f.write(json!([p]));
    assert!(select(&f.store, Some(&id(1)), Some("deviantart"), false).is_err());
    select(&f.store, Some(&id(1)), Some("deviantart"), true).unwrap();
    assert_eq!(f.read()[0]["state"], "APPEARED");
    assert_eq!(f.read()[0]["apparitions"].as_array().unwrap().len(), 1);
}

#[test]
fn new_pending_target_reopens_appeared() {
    let f = Fixture::new();
    let mut p = row(1, "APPEARED");
    let historical = json!({
        "platformName":"pixiv",
        "state":"published_or_scheduled",
        "url":"https://pixiv.example/art/old",
        "priceIfNotFree":4.0
    });
    p["apparitions"] = json!([historical.clone()]);
    f.write(json!([p]));
    select(&f.store, Some(&id(1)), Some("deviantart"), true).unwrap();
    let saved = f.read();
    assert_eq!(saved[0]["state"], "DESCRIBED");
    assert_eq!(
        saved[0]["apparitions"],
        json!([
            historical,
            {"platformName":"deviantart","state":"unpublished"}
        ])
    );
}

#[test]
fn deselect_all_removes_only_the_pending_target_and_restores_appeared() {
    let f = Fixture::new();
    let mut p = row(1, "DESCRIBED");
    let historical = json!({
        "platformName":"pixiv",
        "state":"published_or_scheduled",
        "url":"https://pixiv.example/art/old",
        "options":"preserved"
    });
    p["apparitions"] = json!([
        historical.clone(),
        {"platformName":"deviantart","state":"unpublished"}
    ]);
    f.write(json!([p]));
    select(&f.store, None, None, false).unwrap();
    let saved = f.read();
    assert_eq!(saved[0]["state"], "APPEARED");
    assert_eq!(saved[0]["apparitions"], json!([historical]));
}

#[test]
fn only_registered_platform_can_be_selected_or_recorded() {
    let f = Fixture::new();
    f.write(json!([row(1, "DESCRIBED")]));
    assert!(select(&f.store, Some(&id(1)), Some("pixiv"), true).is_err());
    assert!(record_delivery(&f.store, &id(1), "pixiv", None).is_err());
    assert_eq!(f.read()[0]["apparitions"], json!([]));
}

#[test]
fn profile_defaults_apply_once_to_new_deviantart_target_not_custom() {
    let mut config = Config::default();
    config.tiers = vec!["Gold".into()];
    config.galleries = vec!["Art".into()];
    config.schedule = toml::from_str::<toml::Table>(
        r#"
        profiles = [{name="Standard", day="tuesday", default_tiers=["Gold"], default_price=3.5}]
    "#,
    )
    .unwrap();
    let f = Fixture::with_config(config);
    let mut with_profile = row(1, "DESCRIBED");
    with_profile["scheduleProfile"] = json!("Standard");
    let mut custom = row(2, "DESCRIBED");
    custom["scheduleProfile"] = json!("Custom");
    f.write(json!([with_profile, custom]));
    select(&f.store, Some(&id(1)), Some("deviantart"), true).unwrap();
    select(&f.store, Some(&id(2)), Some("deviantart"), true).unwrap();
    let saved = f.read();
    assert_eq!(saved[0]["apparitions"][0]["tiers"], json!(["Gold"]));
    assert_eq!(saved[0]["apparitions"][0]["priceIfNotFree"], json!(3.5));
    assert!(saved[1]["apparitions"][0].get("tiers").is_none());
    assert!(saved[1]["apparitions"][0].get("priceIfNotFree").is_none());
}

#[test]
fn options_edit_deviantart_not_first_historical_apparition_and_keep_unedited_price() {
    let mut config = Config::default();
    config.tiers = vec!["Gold".into()];
    config.galleries = vec!["Art".into()];
    let f = Fixture::with_config(config);
    let mut p = row(1, "DESCRIBED");
    p["apparitions"] = json!([
        {"platformName":"pixiv","state":"unpublished","priceIfNotFree":99},
        {"platformName":"deviantart","state":"unpublished","priceIfNotFree":0}
    ]);
    f.write(json!([p]));
    save_options(
        &f.store,
        &id(1),
        PlatformOptions {
            price: None,
            tiers: Some(vec!["Gold".into()]),
            galleries: Some(vec!["Art".into()]),
        },
    )
    .unwrap();
    let saved = f.read();
    assert_eq!(saved[0]["apparitions"][0]["priceIfNotFree"], json!(99));
    assert_eq!(saved[0]["apparitions"][1]["priceIfNotFree"], json!(0));
    save_options(
        &f.store,
        &id(1),
        PlatformOptions {
            price: Some(None),
            tiers: None,
            galleries: None,
        },
    )
    .unwrap();
    assert!(
        f.read()[0]["apparitions"][1]
            .get("priceIfNotFree")
            .is_none()
    );
    save_options(
        &f.store,
        &id(1),
        PlatformOptions {
            price: Some(Some(0.0)),
            tiers: None,
            galleries: None,
        },
    )
    .unwrap();
    assert_eq!(
        f.read()[0]["apparitions"][1]["priceIfNotFree"]
            .as_f64()
            .unwrap(),
        0.0
    );
}

#[test]
fn options_reject_invalid_price_and_unconfigured_values_without_writing() {
    let f = Fixture::new();
    let mut p = row(1, "DESCRIBED");
    p["apparitions"] = json!([{"platformName":"deviantart","state":"unpublished"}]);
    f.write(json!([p]));
    for options in [
        PlatformOptions {
            price: Some(Some(-1.0)),
            tiers: None,
            galleries: None,
        },
        PlatformOptions {
            price: Some(Some(f64::INFINITY)),
            tiers: None,
            galleries: None,
        },
        PlatformOptions {
            price: None,
            tiers: Some(vec!["Nope".into()]),
            galleries: None,
        },
        PlatformOptions {
            price: None,
            tiers: None,
            galleries: Some(vec!["Nope".into()]),
        },
    ] {
        assert!(save_options(&f.store, &id(1), options).is_err());
        assert!(f.read()[0]["apparitions"][0].get("tiers").is_none());
    }
}

#[test]
fn delivery_records_url_and_completes_only_when_every_apparition_succeeded() {
    let f = Fixture::new();
    let mut p = row(1, "DESCRIBED");
    p["apparitions"] = json!([
        {"platformName":"deviantart","state":"unpublished"},
        {"platformName":"pixiv","state":"published_or_scheduled"}
    ]);
    f.write(json!([p]));
    record_delivery(
        &f.store,
        &id(1),
        "deviantart",
        Some("https://www.deviantart.com/a"),
    )
    .unwrap();
    assert_eq!(f.read()[0]["state"], "APPEARED");
    assert_eq!(
        f.read()[0]["apparitions"][0]["url"],
        "https://www.deviantart.com/a"
    );
    assert!(record_delivery(&f.store, &id(1), "deviantart", None).is_err());
}

#[test]
fn recording_delivery_without_url_does_not_invent_one() {
    let f = Fixture::new();
    let mut p = row(1, "DESCRIBED");
    p["apparitions"] = json!([{"platformName":"deviantart","state":"unpublished"}]);
    f.write(json!([p]));
    record_delivery(&f.store, &id(1), "deviantart", None).unwrap();
    assert!(f.read()[0]["apparitions"][0].get("url").is_none());
}
