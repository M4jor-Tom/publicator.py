use crate::{
    error::{Error, Result},
    store::Store,
};
use openssl::hash::{Hasher, MessageDigest};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::Builder;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "mp4"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub limit: usize,
    pub paths: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub sha512: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedImage {
    pub path: PathBuf,
    pub mime: String,
}

pub fn file_sha512(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new(MessageDigest::sha512())
        .map_err(|e| Error::Config(format!("SHA-512 unavailable: {e}")))?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher
            .update(&buffer[..count])
            .map_err(|e| Error::Config(e.to_string()))?;
    }
    Ok(hasher
        .finish()
        .map_err(|e| Error::Config(e.to_string()))?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub fn find_candidates(store: &Store, search: &Search) -> Result<Vec<Candidate>> {
    if ![0, 25, 50, 75, 100].contains(&search.limit) {
        return Err(Error::Validation(format!(
            "unsupported search limit {}",
            search.limit
        )));
    }
    let mut seen = HashSet::new();
    for selected in &search.paths {
        if !seen.insert(selected) {
            return Err(Error::Validation(format!(
                "duplicate search path {selected:?}"
            )));
        }
        if !store.config().publicable.contains(selected) {
            return Err(Error::Validation(format!(
                "search path {selected:?} is not configured as publicable"
            )));
        }
    }
    if search.limit == 0 {
        return Ok(Vec::new());
    }

    let data_root = fs::canonicalize(&store.paths().data_dir)?;
    let mut paths = Vec::new();
    for selected in &search.paths {
        let configured = store.paths().data_dir.join(selected);
        let Ok(root) = fs::canonicalize(configured) else {
            continue;
        };
        if !root.starts_with(&data_root) {
            return Err(Error::Validation(format!(
                "configured search path escapes data directory: {selected:?}"
            )));
        }
        collect_images(&root, &mut paths)?;
    }
    paths.sort_by_key(|_| uuid::Uuid::new_v4());
    let references = published_hashes(&store.read()?);
    let mut candidates = Vec::new();
    for path in paths {
        match file_sha512(&path) {
            Ok(sha512) if !references.contains(&sha512) => {
                candidates.push(Candidate { path, sha512 })
            }
            Ok(_) => {}
            Err(error) => eprintln!("Error hashing {}: {error}", path.display()),
        }
        if candidates.len() == search.limit {
            break;
        }
    }
    Ok(candidates)
}

fn collect_images(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_images(&path, output)?;
        } else if kind.is_file()
            && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                IMAGE_EXTENSIONS
                    .iter()
                    .any(|ext| e.eq_ignore_ascii_case(ext))
            })
        {
            output.push(path);
        }
    }
    Ok(())
}

fn published_hashes(rows: &[Value]) -> HashSet<String> {
    let mut hashes = HashSet::new();
    for row in rows {
        for key in ["files", "previewFilesIfNotFree"] {
            if let Some(files) = row.get(key).and_then(Value::as_array) {
                for file in files {
                    for key in ["sha512sum", "fileSha512sum"] {
                        if let Some(hash) = file
                            .get(key)
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            hashes.insert(hash.to_ascii_lowercase());
                        }
                    }
                }
            }
        }
    }
    hashes
}

pub fn thumb_name(path: &Path, sha: &str) -> Result<String> {
    if sha.len() != 128 || !sha.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(Error::Validation(
            "thumbnail identity must be a SHA-512 hex digest".into(),
        ));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    Ok(if ext.is_empty() {
        sha[..32].to_ascii_lowercase()
    } else {
        format!("{}.{}", &sha[..32].to_ascii_lowercase(), ext)
    })
}

pub fn ensure_thumb(path: &Path, cache: &Path, name: &str) -> Result<PathBuf> {
    if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) || name.is_empty() {
        return Err(Error::Validation("invalid thumbnail cache name".into()));
    }
    fs::create_dir_all(cache)?;
    let target = cache.join(name);
    if target.exists() {
        return Ok(target);
    }
    if !actual_mime(path)?.starts_with("image/") {
        return Err(Error::InvalidState(format!(
            "preview unavailable for non-image {}",
            path.display()
        )));
    }
    let extension = target.extension().and_then(|e| e.to_str()).unwrap_or("png");
    let mut temp = Builder::new()
        .prefix(".thumb-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(cache)?;
    let status = Command::new("magick")
        .arg(path)
        .args(["-resize", "300x300>"])
        .arg(temp.path())
        .status();
    if !matches!(status, Ok(status) if status.success()) {
        eprintln!(
            "ImageMagick thumbnail conversion failed for {}; caching the original",
            path.display()
        );
        fs::copy(path, temp.path())?;
    }
    temp.as_file_mut().sync_all()?;
    match temp.persist(&target) {
        Ok(_) => Ok(target),
        Err(_error) if target.exists() => Ok(target),
        Err(error) => Err(error.error.into()),
    }
}

pub fn prepare_ai_image(
    path: &Path,
    cache: &Path,
    known_sha: Option<&str>,
) -> Result<PreparedImage> {
    let sha = match known_sha {
        Some(sha) => sha.to_owned(),
        None => file_sha512(path)?,
    };
    let name = thumb_name(path, &sha)?;
    let prepared = ensure_thumb(path, cache, &name)?;
    let mime = actual_mime(&prepared)?;
    if !mime.starts_with("image/") {
        return Err(Error::InvalidState(format!(
            "AI input is not an image: {mime}"
        )));
    }
    Ok(PreparedImage {
        path: prepared,
        mime,
    })
}

fn actual_mime(path: &Path) -> Result<String> {
    let output = Command::new("identify")
        .args(["-format", "%m"])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(Error::InvalidState(format!(
            "unsupported or unreadable image {}",
            path.display()
        )));
    }
    let format = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    let mime = match format.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => {
            return Err(Error::InvalidState(format!(
                "unsupported image format {format:?}"
            )));
        }
    };
    Ok(mime.into())
}

pub fn validate_upload(path: &Path) -> Result<()> {
    actual_mime(path).map(|_| ())
}

pub fn prepare_upload(source: &Path, attempt_dir: &Path) -> Result<PathBuf> {
    if !source
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("webp"))
    {
        return Ok(source.to_owned());
    }
    fs::create_dir_all(attempt_dir)?;
    let target = attempt_dir.join("upload.png");
    let mut temp = Builder::new()
        .prefix(".upload-")
        .suffix(".png")
        .tempfile_in(attempt_dir)?;
    let status = Command::new("magick")
        .arg(source)
        .arg(temp.path())
        .status()?;
    if !status.success() {
        return Err(Error::InvalidState(format!(
            "WebP conversion failed for {}",
            source.display()
        )));
    }
    temp.as_file_mut().sync_all()?;
    temp.persist(&target).map_err(|e| e.error)?;
    Ok(target)
}

pub fn delete_candidate(store: &Store, authorized: &[Candidate], path: &Path) -> Result<()> {
    let candidate = authorized
        .iter()
        .find(|candidate| candidate.path == path)
        .ok_or_else(|| {
            Error::Validation(format!(
                "candidate is not currently authorized: {}",
                path.display()
            ))
        })?;
    let current_path = fs::canonicalize(path)?;
    let data_root = fs::canonicalize(&store.paths().data_dir)?;
    let mut in_configured_root = false;
    for configured in &store.config().publicable {
        if let Ok(root) = fs::canonicalize(store.paths().data_dir.join(configured)) {
            if root.starts_with(&data_root) && current_path.starts_with(root) {
                in_configured_root = true;
                break;
            }
        }
    }
    if !in_configured_root {
        return Err(Error::Validation(format!(
            "candidate is outside configured paths: {}",
            path.display()
        )));
    }
    let current_sha = file_sha512(path)?;
    if current_sha != candidate.sha512 {
        return Err(Error::InvalidState(
            "candidate changed since authorization".into(),
        ));
    }
    let basename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let rows = store.read()?;
    if published_hashes(&rows).contains(&current_sha) {
        return Err(Error::InvalidState(format!(
            "referenced artwork cannot be deleted: {basename}"
        )));
    }
    for row in rows {
        for key in ["files", "previewFilesIfNotFree"] {
            if let Some(files) = row.get(key).and_then(Value::as_array) {
                for file in files {
                    let referenced_name = file.get("basename").and_then(Value::as_str);
                    if referenced_name == Some(basename)
                        || file.get("fileName").and_then(Value::as_str) == Some(basename)
                    {
                        return Err(Error::InvalidState(format!(
                            "referenced artwork cannot be deleted: {basename}"
                        )));
                    }
                }
            }
        }
    }
    fs::remove_file(path)?;
    Ok(())
}

pub fn index_by_basename(root: &Path) -> Result<std::collections::HashMap<String, PathBuf>> {
    fn walk(root: &Path, index: &mut std::collections::HashMap<String, PathBuf>) -> Result<()> {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if !entry.file_name().to_string_lossy().starts_with('.') {
                    walk(&path, index)?;
                }
            } else if entry.file_type()?.is_file() {
                index
                    .entry(entry.file_name().to_string_lossy().into_owned())
                    .or_insert(path);
            }
        }
        Ok(())
    }
    let mut index = std::collections::HashMap::new();
    walk(root, &mut index)?;
    Ok(index)
}
