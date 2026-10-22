use crate::{
    config::{Config, ImageIdentity, PromptConfig, PromptGrammar},
    error::{Error, Result},
    images::{Candidate, Search, file_sha512},
    model::Publications,
    store::Store,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, RwLock},
    time::{Duration, Instant},
};

const HEAD_PROBE_TTL: Duration = Duration::from_secs(2);
const SEARCH_GROUP_LIMIT: usize = 200;
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "mp4"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptVersion {
    pub digest: String,
    pub text: String,
    pub paths: Vec<String>,
    pub committed: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lineage {
    pub path: String,
    pub versions: Vec<PromptVersion>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptMatch {
    Exact(PromptVersion),
    Nearest(Vec<Lineage>),
    Unknown(String),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchMatches {
    pub exact: Vec<Candidate>,
    pub maybe: Vec<Candidate>,
    pub skipped: usize,
    pub truncated: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditCounts {
    pub files: usize,
    pub parsed: usize,
    pub exact: usize,
    pub nearest: usize,
    pub unknown: usize,
    pub exact_versions: usize,
    pub nearest_versions: usize,
}

#[derive(Default)]
struct Index {
    head: String,
    probed: Option<Instant>,
    versions: BTreeMap<String, PromptVersion>,
    by_path: BTreeMap<String, Vec<String>>,
    by_basename: BTreeMap<String, Vec<String>>,
}
pub struct PromptArchive {
    repo: PathBuf,
    grammar: PromptGrammar,
    index: RwLock<Index>,
    refresh_lock: Mutex<()>,
}
impl PromptArchive {
    pub fn new(data_dir: &Path, config: &PromptConfig) -> Result<Self> {
        Ok(Self {
            repo: data_dir.join(&config.repo),
            grammar: PromptGrammar::compile(config)?,
            index: RwLock::new(Index::default()),
            refresh_lock: Mutex::new(()),
        })
    }

    pub fn from_config(data_dir: &Path, config: &PromptConfig) -> Result<Option<Self>> {
        let archive = Self::new(data_dir, config)?;
        match fs::metadata(&archive.repo) {
            Ok(metadata) if metadata.is_dir() => Ok(Some(archive)),
            Ok(_) => Err(Error::Config(format!(
                "prompt archive is not a directory: {}",
                archive.repo.display()
            ))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .output()
            .map_err(|error| {
                Error::PromptArchiveUnavailable(format!(
                    "git unavailable in {}: {error}",
                    self.repo.display()
                ))
            })?;
        if output.status.success() && output.stderr.is_empty() {
            Ok(output.stdout)
        } else {
            Err(Error::PromptArchiveUnavailable(format!(
                "git {} returned {} in {}: {}",
                args.join(" "),
                output.status,
                self.repo.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn head(&self) -> Result<Option<String>> {
        let args = ["rev-parse", "--verify", "--quiet", "HEAD"];
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .output()
            .map_err(|error| {
                Error::PromptArchiveUnavailable(format!(
                    "git unavailable in {}: {error}",
                    self.repo.display()
                ))
            })?;
        if output.status.success() && output.stderr.is_empty() {
            return Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            ));
        }
        // A valid, freshly initialized archive may not have a HEAD yet.
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return Ok(None);
        }
        Err(Error::PromptArchiveUnavailable(format!(
            "git {} returned {} in {}: {}",
            args.join(" "),
            output.status,
            self.repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }

    fn refresh(&self) -> Result<()> {
        let _refresh = self
            .refresh_lock
            .lock()
            .map_err(|_| Error::Busy("prompt archive refresh lock poisoned".into()))?;
        let now = Instant::now();
        let (previous_head, initialized, probed) = {
            let index = self
                .index
                .read()
                .map_err(|_| Error::Busy("prompt archive index lock poisoned".into()))?;
            (index.head.clone(), index.probed.is_some(), index.probed)
        };
        if probed.is_some_and(|at| now.duration_since(at) < HEAD_PROBE_TTL) {
            return Ok(());
        }
        let head = self.head()?;
        if initialized && head.as_deref().is_some_and(|head| head == previous_head) {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::Busy("prompt archive index lock poisoned".into()))?;
            index.probed = Some(Instant::now());
            return Ok(());
        }

        let stream = self.git(&["cat-file", "--batch-all-objects", "--batch"])?;
        let (by_oid, blobs) = self.read_blobs(&stream).map_err(|error| match error {
            Error::Protocol(reason) => Error::PromptArchiveUnavailable(reason),
            other => other,
        })?;
        let (paths, times) = if head.is_some()
            || !self
                .git(&["for-each-ref", "--format=%(refname)"])?
                .is_empty()
        {
            let log = self.git(&[
                "-c",
                "core.quotePath=false",
                "log",
                "--all",
                "--raw",
                "--no-abbrev",
                "--no-renames",
                "--format=%ct",
            ])?;
            history(&log, &by_oid)
        } else {
            (BTreeMap::new(), BTreeMap::new())
        };
        let mut versions = BTreeMap::new();
        for (digest, body) in blobs {
            versions.insert(
                digest.clone(),
                PromptVersion {
                    digest: digest.clone(),
                    text: String::from_utf8_lossy(&body).into_owned(),
                    paths: paths.get(&digest).cloned().unwrap_or_default(),
                    committed: times.get(&digest).copied().unwrap_or(0),
                },
            );
        }
        let mut by_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_basename: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for version in versions.values() {
            for path in &version.paths {
                by_path
                    .entry(path.clone())
                    .or_default()
                    .push(version.digest.clone());
                let base = Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path);
                let all = by_basename.entry(base.to_owned()).or_default();
                if !all.contains(path) {
                    all.push(path.clone());
                }
            }
        }
        for digests in by_path.values_mut() {
            digests.sort_by_key(|d| (std::cmp::Reverse(versions[d].committed), d.clone()));
        }
        // Build all derived maps before this assignment: readers see one complete index.
        let mut index = self
            .index
            .write()
            .map_err(|_| Error::Busy("prompt archive index lock poisoned".into()))?;
        *index = Index {
            head: head.unwrap_or_default(),
            probed: Some(Instant::now()),
            versions,
            by_path,
            by_basename,
        };
        Ok(())
    }

    fn read_blobs(
        &self,
        stream: &[u8],
    ) -> Result<(BTreeMap<String, String>, BTreeMap<String, Vec<u8>>)> {
        let mut by_oid = BTreeMap::new();
        let mut blobs = BTreeMap::new();
        let mut cursor = 0usize;
        while cursor < stream.len() {
            let Some(relative_newline) = stream[cursor..].iter().position(|b| *b == b'\n') else {
                break;
            };
            let newline = cursor + relative_newline;
            let header = &stream[cursor..newline];
            let mut fields = header.split(|b| *b == b' ');
            let (Some(oid), Some(kind), Some(size), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                cursor = newline + 1;
                continue;
            };
            let Ok(size) = std::str::from_utf8(size).unwrap_or("").parse::<usize>() else {
                cursor = newline + 1;
                continue;
            };
            let start = newline + 1;
            let end = start
                .checked_add(size)
                .ok_or_else(|| Error::Protocol("blob size overflow".into()))?;
            let body = stream
                .get(start..end)
                .ok_or_else(|| Error::Protocol("truncated blob payload".into()))?;
            let next = end
                .checked_add(1)
                .ok_or_else(|| Error::Protocol("blob framing overflow".into()))?;
            if stream.get(end) != Some(&b'\n') {
                return Err(Error::Protocol("missing blob payload terminator".into()));
            }
            if kind == b"blob" {
                let digest = self.grammar.digest(body)?;
                by_oid.insert(String::from_utf8_lossy(oid).into_owned(), digest.clone());
                blobs.insert(digest, body.to_vec());
            }
            cursor = next;
        }
        Ok((by_oid, blobs))
    }

    pub fn resolve_path(&self, path: &Path) -> Result<PromptMatch> {
        let basename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let Some(identity) = self.grammar.identity(basename)? else {
            return Ok(PromptMatch::Unknown(
                "filename does not match the configured grammar".into(),
            ));
        };
        self.resolve(&identity, path.parent().unwrap_or(Path::new("")))
    }

    fn resolve(&self, identity: &ImageIdentity, near: &Path) -> Result<PromptMatch> {
        match self.refresh() {
            Ok(()) => {}
            Err(Error::PromptArchiveUnavailable(reason)) => {
                let diagnostic = format!("prompt archive unavailable: {reason}");
                eprintln!("{diagnostic}");
                return Ok(PromptMatch::Unknown(diagnostic));
            }
            Err(error) => return Err(error),
        }
        let index = self
            .index
            .read()
            .map_err(|_| Error::Busy("prompt archive index lock poisoned".into()))?;
        if let Some(version) = index.versions.get(&identity.version) {
            return Ok(PromptMatch::Exact(version.clone()));
        }
        let Some(lineage) = identity.lineage.as_deref() else {
            return Ok(PromptMatch::Unknown(
                "prompt not archived; configured grammar has no lineage".into(),
            ));
        };
        let paths = index.by_basename.get(lineage).cloned().unwrap_or_default();
        let paths = rank_paths(paths, near);
        let versions = &index.versions;
        let candidates = paths
            .into_iter()
            .filter_map(|path| {
                let versions = index
                    .by_path
                    .get(&path)?
                    .iter()
                    .filter_map(|digest| versions.get(digest).cloned())
                    .collect::<Vec<_>>();
                (!versions.is_empty()).then_some(Lineage { path, versions })
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            Ok(PromptMatch::Unknown(format!(
                "prompt not archived; no known versions of {lineage:?}"
            )))
        } else {
            Ok(PromptMatch::Nearest(candidates))
        }
    }
}

fn history(
    log: &[u8],
    by_oid: &BTreeMap<String, String>,
) -> (BTreeMap<String, Vec<String>>, BTreeMap<String, i64>) {
    let text = String::from_utf8_lossy(log);
    let mut paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut times = BTreeMap::new();
    let mut commit_time = 0i64;
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if !line.starts_with(':') {
            if let Ok(time) = line.parse() {
                commit_time = time;
            }
            continue;
        }
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let fields = meta.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 || path.is_empty() {
            continue;
        }
        let Some(digest) = by_oid.get(fields[3]) else {
            continue;
        };
        paths
            .entry(digest.clone())
            .or_default()
            .insert(path.to_owned());
        times
            .entry(digest.clone())
            .and_modify(|time: &mut i64| *time = (*time).min(commit_time))
            .or_insert(commit_time);
    }
    (
        paths
            .into_iter()
            .map(|(d, paths)| (d, paths.into_iter().collect()))
            .collect(),
        times,
    )
}

fn rank_paths(mut paths: Vec<String>, near: &Path) -> Vec<String> {
    paths.sort_by(|a, b| {
        let score_a = similarity(near, Path::new(a));
        let score_b = similarity(near, Path::new(b));
        score_b.total_cmp(&score_a).then_with(|| a.cmp(b))
    });
    paths
}
fn similarity(image_dir: &Path, prompt_path: &Path) -> f64 {
    let a = normalize(
        image_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default(),
    );
    let b = normalize(
        prompt_path
            .parent()
            .unwrap_or(Path::new(""))
            .to_str()
            .unwrap_or_default(),
    );
    matching_ratio(&a, &b)
}
fn normalize(s: &str) -> Vec<char> {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}
fn matching_ratio(a: &[char], b: &[char]) -> f64 {
    fn longest(
        a: &[char],
        b: &[char],
        ai: usize,
        aj: usize,
        bi: usize,
        bj: usize,
    ) -> (usize, usize, usize) {
        let mut previous = vec![0usize; bj - bi + 1];
        let mut best = (ai, bi, 0);
        for i in ai..aj {
            let mut current = vec![0usize; bj - bi + 1];
            for j in bi..bj {
                if a[i] == b[j] {
                    current[j - bi + 1] = previous[j - bi] + 1;
                    let n = current[j - bi + 1];
                    if n > best.2 {
                        best = (i + 1 - n, j + 1 - n, n);
                    }
                }
            }
            previous = current;
        }
        best
    }
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let mut ranges = vec![(0, a.len(), 0, b.len())];
    let mut total = 0usize;
    while let Some((ai, aj, bi, bj)) = ranges.pop() {
        let (i, j, n) = longest(a, b, ai, aj, bi, bj);
        if n == 0 {
            continue;
        }
        total += n;
        if ai < i && bi < j {
            ranges.push((ai, i, bi, j));
        }
        if i + n < aj && j + n < bj {
            ranges.push((i + n, aj, j + n, bj));
        }
    }
    (2 * total) as f64 / (a.len() + b.len()) as f64
}

fn collect_images(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for item in fs::read_dir(root)? {
        let item = item?;
        let path = item.path();
        let kind = item.file_type()?;
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
fn configured_files(data_dir: &Path, configured: &[String]) -> Result<Vec<PathBuf>> {
    let base = fs::canonicalize(data_dir)?;
    let mut output = Vec::new();
    for name in configured {
        let path = data_dir.join(name);
        let Ok(root) = fs::canonicalize(path) else {
            continue;
        };
        if !root.starts_with(&base) {
            return Err(Error::Validation(format!(
                "configured prompt search path escapes data directory: {name:?}"
            )));
        }
        collect_images(&root, &mut output)?;
    }
    output.sort();
    Ok(output)
}
fn published_hashes(rows: &Publications) -> HashSet<String> {
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

pub fn search(
    store: &Store,
    search: &Search,
    archive: Option<&PromptArchive>,
    needle: Option<&str>,
    lineage: Option<&str>,
) -> Result<SearchMatches> {
    let Some(archive) = archive else {
        return Ok(SearchMatches::default());
    };
    if ![0, 25, 50, 75, 100].contains(&search.limit) {
        return Err(Error::Validation(format!(
            "unsupported search limit {}",
            search.limit
        )));
    }
    let mut unique = HashSet::new();
    for path in &search.paths {
        if !unique.insert(path) {
            return Err(Error::Validation(format!("duplicate search path {path:?}")));
        }
        if !store.config().publicable.contains(path) {
            return Err(Error::Validation(format!(
                "search path {path:?} is not configured as publicable"
            )));
        }
    }
    if search.limit == 0 {
        return Ok(SearchMatches::default());
    }
    let paths = configured_files(&store.paths().data_dir, &search.paths)?;
    let referenced = published_hashes(&store.read()?);
    let needle = needle.filter(|s| !s.is_empty()).map(str::to_lowercase);
    let mut exact = Vec::new();
    let mut maybe = Vec::new();
    let mut skipped = 0;
    for path in paths {
        match archive.resolve_path(&path)? {
            PromptMatch::Exact(version) => {
                if needle
                    .as_ref()
                    .is_some_and(|text| !version.text.to_lowercase().contains(text))
                {
                    continue;
                }
                if lineage.is_some_and(|path| !version.paths.iter().any(|p| p == path)) {
                    continue;
                }
                exact.push(path);
            }
            PromptMatch::Nearest(candidates) => {
                if lineage
                    .is_some_and(|path| candidates.iter().any(|candidate| candidate.path == path))
                {
                    maybe.push(path);
                } else {
                    skipped += 1;
                }
            }
            PromptMatch::Unknown(reason) if reason.starts_with("prompt archive unavailable:") => {
                return Err(Error::PromptArchiveUnavailable(reason));
            }
            PromptMatch::Unknown(_) => {}
        }
    }
    let truncated = exact.len() > SEARCH_GROUP_LIMIT
        || maybe.len() > SEARCH_GROUP_LIMIT
        || exact.len().saturating_add(maybe.len()) > search.limit;
    exact.truncate(SEARCH_GROUP_LIMIT);
    maybe.truncate(SEARCH_GROUP_LIMIT);
    let mut result = SearchMatches {
        skipped,
        truncated,
        ..SearchMatches::default()
    };
    for path in &exact {
        if result.exact.len() >= search.limit {
            break;
        }
        match file_sha512(path) {
            Ok(sha512) if !referenced.contains(&sha512.to_ascii_lowercase()) => {
                result.exact.push(Candidate {
                    path: path.clone(),
                    sha512,
                });
            }
            Ok(_) => {}
            Err(error) => eprintln!("Error hashing {}: {error}", path.display()),
        }
    }
    for path in &maybe {
        if result.exact.len() + result.maybe.len() >= search.limit {
            break;
        }
        match file_sha512(path) {
            Ok(sha512) if !referenced.contains(&sha512.to_ascii_lowercase()) => {
                result.maybe.push(Candidate {
                    path: path.clone(),
                    sha512,
                });
            }
            Ok(_) => {}
            Err(error) => eprintln!("Error hashing {}: {error}", path.display()),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn malformed_blob_framing_skips_bad_headers_and_stops_at_truncation() {
        let root = tempdir().unwrap();
        let config = PromptConfig {
            repo: ".".into(),
            filename: r"(?P<version>.+)".into(),
            version_hash: "sha256".into(),
        };
        let archive = PromptArchive::new(root.path(), &config).unwrap();
        let good = format!("{} blob 5\nhello\n", "a".repeat(40));
        let stream = [
            good.as_bytes(),
            b"malformed\n",
            b"b blob nope\n",
            good.as_bytes(),
        ]
        .concat();
        let (ids, blobs) = archive.read_blobs(&stream).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(blobs.values().next().unwrap(), b"hello");

        let truncated = [
            good.as_bytes(),
            format!("{} blob 50\nshort", "c".repeat(40)).as_bytes(),
        ]
        .concat();
        assert!(archive.read_blobs(&truncated).is_err());
    }
}

pub fn audit(
    data_dir: &Path,
    config: &Config,
    archive: Option<&PromptArchive>,
) -> Result<AuditCounts> {
    let paths = configured_files(data_dir, &config.publicable)?;
    let mut counts = AuditCounts {
        files: paths.len(),
        ..AuditCounts::default()
    };
    let Some(archive) = archive else {
        return Ok(counts);
    };
    let mut exact_versions = HashSet::new();
    let mut nearest_versions = HashSet::new();
    for path in paths {
        let basename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let Some(identity) = archive.grammar.identity(basename)? else {
            continue;
        };
        counts.parsed += 1;
        match archive.resolve(&identity, path.parent().unwrap_or(Path::new("")))? {
            PromptMatch::Exact(_) => {
                counts.exact += 1;
                exact_versions.insert(identity.version);
            }
            PromptMatch::Nearest(_) => {
                counts.nearest += 1;
                nearest_versions.insert(identity.version);
            }
            PromptMatch::Unknown(_) => counts.unknown += 1,
        }
    }
    counts.exact_versions = exact_versions.len();
    counts.nearest_versions = nearest_versions.len();
    Ok(counts)
}
