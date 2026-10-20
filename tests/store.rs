mod common;
use common::{Fixture, row};
use publicator::{
    config::{Config, Paths},
    error::Error,
};
use serde_json::json;
use std::sync::{Arc, Barrier};

#[test]
fn round_trips_unknown_fields_and_missing_files_are_empty() {
    let f = Fixture::new();
    assert!(f.store.read().unwrap().is_empty());
    let mut value = row(1, "PICKED");
    value["futureField"] = json!({"kept": true});
    f.write(json!([value]));
    assert_eq!(
        f.store.read().unwrap()[0]["futureField"],
        json!({"kept": true})
    );
}

#[test]
fn malformed_json_is_not_treated_as_an_empty_queue() {
    let f = Fixture::new();
    std::fs::write(&f.store.paths().json, b"{").unwrap();
    assert!(matches!(f.store.read(), Err(Error::Json(_))));
}

#[test]
fn invalid_edit_does_not_replace_original() {
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let result = f.store.mutate(|rows| {
        rows[0]["state"] = "wrong".into();
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
}

#[test]
fn failed_replacement_preserves_original_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = f.store.mutate(|rows| {
        rows.push(row(2, "PICKED"));
        Ok(())
    });
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
}

#[test]
fn separate_store_instances_merge_fresh_thread_edits_and_keep_lock() {
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    let second = publicator::store::Store::new(f.store.paths().clone(), Config::default()).unwrap();
    let stores = [f.store.clone(), second];
    let barrier = Arc::new(Barrier::new(3));
    let threads: Vec<_> = stores
        .into_iter()
        .enumerate()
        .map(|(i, store)| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .mutate(|rows| {
                        rows.push(row(i as u64 + 2, "PICKED"));
                        Ok(())
                    })
                    .unwrap();
            })
        })
        .collect();
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(f.store.read().unwrap().len(), 3);
    assert!(f.store.paths().json.with_extension("json.lock").exists());
}

#[test]
fn external_json_path_uses_data_directory_allow_lists() {
    let f = Fixture::new();
    let json_path = f.path("elsewhere.json");
    let paths = Paths::resolve(Some(f.root.path()), Some(&json_path)).unwrap();
    let config = Config {
        tiers: vec!["allowed".into()],
        ..Config::default()
    };
    let store = publicator::store::Store::new(paths, config).unwrap();
    let mut publication = row(1, "PICKED");
    publication["apparitions"] =
        json!([{"platformName":"deviantart", "state":"unpublished", "tiers":["allowed"]}]);
    store
        .mutate(|rows| {
            rows.push(publication.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store.read().unwrap()[0]["apparitions"][0]["tiers"][0],
        "allowed"
    );
    publication["apparitions"][0]["tiers"] = json!(["forbidden"]);
    assert!(
        store
            .mutate(|rows| {
                *rows = vec![publication];
                Ok(())
            })
            .is_err()
    );
}

#[test]
fn child_process_waits_for_parent_lock() {
    const CHILD: &str = "PUBLICATOR_LOCK_CHILD";
    if let Ok(path) = std::env::var(CHILD) {
        let path = std::path::PathBuf::from(path);
        let ready = path.with_extension("ready");
        let acquired = path.with_extension("acquired");
        std::fs::write(ready, b"ready").unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        std::fs::write(acquired, b"acquired").unwrap();
        return;
    }
    let f = Fixture::new();
    let lock_path = f.store.paths().json.with_extension("json.lock");
    std::fs::File::create(&lock_path).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_process_waits_for_parent_lock",
            "--nocapture",
        ])
        .env(CHILD, &lock_path)
        .spawn()
        .unwrap();
    let ready = lock_path.with_extension("ready");
    let acquired = lock_path.with_extension("acquired");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(ready.exists(), "child must signal readiness before locking");
    assert!(
        !acquired.exists(),
        "child must block while parent holds the lock"
    );
    fs2::FileExt::unlock(&lock).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !acquired.exists() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        acquired.exists(),
        "child must acquire after parent releases lock"
    );
    assert!(child.wait().unwrap().success());
}
