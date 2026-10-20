mod common;

use common::{Fixture, row};
use publicator::{
    config::Config,
    images::{
        Candidate, Search, delete_candidate, ensure_thumb, file_sha512, find_candidates,
        prepare_ai_image, prepare_upload, thumb_name,
    },
};
use serde_json::json;
use std::fs;

#[test]
fn search_accepts_only_configured_unique_paths_and_supported_limits() {
    let mut config = Config::default();
    config.publicable = vec!["picked".into(), "more".into()];
    let f = Fixture::with_config(config);
    fs::create_dir_all(f.path("picked")).unwrap();
    fs::create_dir_all(f.path("more")).unwrap();
    f.image("picked/a.png", 10, 10);
    f.image("more/b.png", 10, 10);
    for limit in [0, 25, 50, 75, 100] {
        let got = find_candidates(
            &f.store,
            &Search {
                limit,
                paths: vec!["picked".into()],
            },
        )
        .unwrap();
        assert_eq!(got.len(), if limit == 0 { 0 } else { 1 });
    }
    for (limit, paths) in [
        (1, vec!["picked".into()]),
        (25, vec!["../outside".into()]),
        (25, vec!["picked".into(), "picked".into()]),
    ] {
        assert!(find_candidates(&f.store, &Search { limit, paths }).is_err());
    }
}

#[test]
fn discovery_deduplicates_both_hash_keys_in_files_and_previews() {
    let mut config = Config::default();
    config.publicable = vec!["picked".into()];
    let f = Fixture::with_config(config);
    fs::create_dir_all(f.path("picked")).unwrap();
    let a = f.image("picked/a.png", 10, 10);
    let b = f.image("picked/b.png", 10, 10);
    let c = f.image("picked/c.png", 10, 10);
    let d = f.image("picked/d.png", 10, 10);
    let mut published = row(1, "APPEARED");
    published["files"] =
        json!([{"basename":"a.png", "sha512sum": file_sha512(&a).unwrap().to_uppercase()}]);
    published["previewFilesIfNotFree"] = json!([
        {"basename":"b.png", "fileSha512sum": file_sha512(&b).unwrap()},
        {"basename":"c.png", "sha512sum": file_sha512(&c).unwrap()},
        {"basename":"d.png", "fileSha512sum": file_sha512(&d).unwrap()}
    ]);
    f.write(json!([published]));
    assert!(
        find_candidates(
            &f.store,
            &Search {
                limit: 100,
                paths: vec!["picked".into()]
            }
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        !f.path(".thumbs").exists(),
        "discovery must not eagerly thumbnail history"
    );
}

#[test]
fn referenced_basename_blocks_delete_even_when_art_bytes_changed() {
    let mut config = Config::default();
    config.publicable = vec!["art".into()];
    let f = Fixture::with_config(config);
    fs::create_dir_all(f.path("art")).unwrap();
    let p = f.image("art/changed.png", 12, 12);
    let candidate = Candidate {
        path: p.clone(),
        sha512: file_sha512(&p).unwrap(),
    };
    let mut published = row(1, "APPEARED");
    published["files"][0]["basename"] = json!("changed.png");
    published["files"][0]["sha512sum"] = json!("f".repeat(128));
    f.write(json!([published]));
    assert!(delete_candidate(&f.store, &[candidate.clone()], &p).is_err());
    assert!(p.exists());
    assert!(delete_candidate(&f.store, &[], &p).is_err());
}

#[test]
fn ai_uses_a_real_cached_image_and_upload_keeps_non_webp_source() {
    let f = Fixture::new();
    let source = f.image("image.png", 900, 600);
    let ai = prepare_ai_image(&source, &f.path(".thumbs"), None).unwrap();
    assert_eq!(ai.mime, "image/png");
    assert_eq!(
        std::process::Command::new("identify")
            .args(["-format", "%wx%h"])
            .arg(ai.path)
            .output()
            .unwrap()
            .stdout,
        b"300x200"
    );
    assert_eq!(prepare_upload(&source, &f.path("attempt")).unwrap(), source);
}

#[test]
fn cache_reuses_content_identity_and_really_resizes() {
    let f = Fixture::new();
    let p = f.image("large.png", 900, 600);
    let sha = file_sha512(&p).unwrap();
    let name = thumb_name(&p, &sha).unwrap();
    assert_eq!(name, format!("{}.png", &sha[..32]));
    let t = ensure_thumb(&p, &f.path(".thumbs"), &name).unwrap();
    let out = std::process::Command::new("identify")
        .args(["-format", "%wx%h"])
        .arg(&t)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "300x200");
    fs::rename(&p, f.path("renamed.png")).unwrap();
    assert!(
        ensure_thumb(&f.path("renamed.png"), &f.path(".thumbs"), &name)
            .unwrap()
            .exists()
    );
}

#[test]
fn mixed_case_webp_upload_converts_inside_attempt_directory() {
    let f = Fixture::new();
    let p = f.image("source.WeBp", 20, 10);
    let out = prepare_upload(&p, &f.path("attempt")).unwrap();
    assert_eq!(out.parent(), Some(f.path("attempt").as_path()));
    assert_eq!(out.extension().and_then(|e| e.to_str()), Some("png"));
    assert_eq!(
        std::process::Command::new("identify")
            .args(["-format", "%m"])
            .arg(out)
            .output()
            .unwrap()
            .stdout,
        b"PNG"
    );
}

#[test]
fn ai_preparation_rejects_video_instead_of_claiming_image_mime() {
    let f = Fixture::new();
    let video = f.path("clip.mp4");
    fs::write(&video, b"not an image").unwrap();
    assert!(prepare_ai_image(&video, &f.path(".thumbs"), None).is_err());
}

#[test]
fn basename_lookup_ignores_hidden_runtime_directories() {
    let f = Fixture::new();
    fs::create_dir_all(f.path("art")).unwrap();
    fs::create_dir_all(f.path(".thumbs")).unwrap();
    fs::write(f.path("art/same.png"), b"art").unwrap();
    fs::write(f.path(".thumbs/same.png"), b"cache").unwrap();
    let result = publicator::images::index_by_basename(f.root.path()).unwrap();
    assert_eq!(result.get("same.png").unwrap(), &f.path("art/same.png"));
    assert!(!result.contains_key("private.png"));
}
