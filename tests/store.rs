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
fn malformed_json_mutation_preserves_original_bytes_without_running_callback() {
    let f = Fixture::new();
    std::fs::write(&f.store.paths().json, b"{broken").unwrap();
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let called = std::cell::Cell::new(false);
    assert!(
        f.store
            .mutate(|_| {
                called.set(true);
                Ok(())
            })
            .is_err()
    );
    assert!(!called.get());
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
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
fn mutation_returns_callback_value_and_persists_successful_edit() {
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    let result = f
        .store
        .mutate(|rows| {
            rows[0]["title"] = "Updated".into();
            Ok(None::<()>)
        })
        .unwrap();
    assert_eq!(result, None);
    assert_eq!(f.store.read().unwrap()[0]["title"], "Updated");
}

#[test]
fn rejected_mutation_callback_preserves_original_bytes() {
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let result: Result<(), Error> = f.store.mutate(|rows| {
        rows[0]["title"] = "uncommitted change".into();
        Err(Error::Validation("rejected".into()))
    });
    assert!(matches!(result, Err(Error::Validation(message)) if message == "rejected"));
    assert_eq!(std::fs::read(&f.store.paths().json).unwrap(), before);
}

#[test]
fn failed_staging_preserves_original_after_mutation_callback() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
    f.store.read().unwrap(); // Create the persistent lock before making the directory unwritable.
    let before = std::fs::read(&f.store.paths().json).unwrap();
    let called = std::cell::Cell::new(false);
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = f.store.mutate(|rows| {
        called.set(true);
        rows.push(row(2, "PICKED"));
        Ok(())
    });
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        called.get(),
        "mutation callback must run before staging failure"
    );
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

#[cfg(unix)]
#[test]
fn json_symlink_aliases_share_the_target_and_persistent_lock() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let target = f.path("target.json");
    std::fs::write(
        &target,
        serde_json::to_vec(&json!([row(1, "PICKED")])).unwrap(),
    )
    .unwrap();
    let alias = f.path("alias.json");
    symlink(&target, &alias).unwrap();
    let target_store = publicator::store::Store::new(
        Paths::resolve(Some(f.root.path()), Some(&target)).unwrap(),
        Config::default(),
    )
    .unwrap();
    let alias_store = publicator::store::Store::new(
        Paths::resolve(Some(f.root.path()), Some(&alias)).unwrap(),
        Config::default(),
    )
    .unwrap();
    assert_eq!(target_store.paths().json, alias_store.paths().json);
    alias_store
        .mutate(|rows| {
            rows[0]["future"] = json!(true);
            Ok(())
        })
        .unwrap();
    assert!(alias.is_symlink());
    assert_eq!(target_store.read().unwrap()[0]["future"], true);
    assert!(target.with_extension("json.lock").exists());
}

#[cfg(unix)]
#[test]
fn dangling_json_symlink_is_rejected_instead_of_becoming_an_empty_database() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let alias = f.path("dangling.json");
    symlink(f.path("missing-target.json"), &alias).unwrap();
    assert!(Paths::resolve(Some(f.root.path()), Some(&alias)).is_err());
}

#[test]
fn child_store_mutation_waits_for_parent_lock() {
    const CHILD: &str = "PUBLICATOR_STORE_LOCK_CHILD";
    if let Ok(path) = std::env::var(CHILD) {
        let json_path = std::path::PathBuf::from(path);
        let ready = json_path.with_extension("ready");
        let completed = json_path.with_extension("completed");
        let paths = Paths::resolve(json_path.parent(), Some(&json_path)).unwrap();
        let store = publicator::store::Store::new(paths, Config::default()).unwrap();
        std::fs::write(ready, b"ready").unwrap();
        store
            .mutate(|rows| {
                rows.push(row(2, "PICKED"));
                Ok(())
            })
            .unwrap();
        std::fs::write(completed, b"completed").unwrap();
        return;
    }
    let f = Fixture::new();
    f.write(json!([row(1, "PICKED")]));
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
            "child_store_mutation_waits_for_parent_lock",
            "--nocapture",
        ])
        .env(CHILD, &f.store.paths().json)
        .spawn()
        .unwrap();
    let ready = f.store.paths().json.with_extension("ready");
    let completed = f.store.paths().json.with_extension("completed");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !ready.exists() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        ready.exists(),
        "child must signal readiness before Store mutation"
    );
    let held_deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
    while !completed.exists() && std::time::Instant::now() < held_deadline {
        std::thread::yield_now();
    }
    assert!(
        !completed.exists(),
        "Store mutation must not complete while parent holds the lock"
    );
    fs2::FileExt::unlock(&lock).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !completed.exists() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        completed.exists(),
        "Store mutation must complete after release"
    );
    assert!(child.wait().unwrap().success());
    assert_eq!(f.store.read().unwrap().len(), 2);
}
