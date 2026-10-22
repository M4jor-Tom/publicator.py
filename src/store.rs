use crate::{
    config::{Config, Paths, validate_publications_with},
    error::{Error, Result},
    model::Publications,
};
use fs2::FileExt;
use jsonschema::Validator;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
use tempfile::NamedTempFile;

static MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone)]
pub struct Store {
    paths: Paths,
    config: Config,
    validator: Arc<Validator>,
}
impl Store {
    pub fn new(paths: Paths, config: Config) -> Result<Self> {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../assets/publicationsSchema.json"))?;
        let validator =
            jsonschema::validator_for(&schema).map_err(|e| Error::Config(e.to_string()))?;
        Ok(Self {
            paths,
            config,
            validator: Arc::new(validator),
        })
    }
    pub fn paths(&self) -> &Paths {
        &self.paths
    }
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn read(&self) -> Result<Publications> {
        let _process = MUTATION_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let lock = self.lock_file()?;
        lock.lock_shared()?;
        self.read_unlocked()
    }

    pub fn mutate<R>(&self, f: impl FnOnce(&mut Publications) -> Result<R>) -> Result<R> {
        let _process = MUTATION_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let lock = self.lock_file()?;
        lock.lock_exclusive()?;
        self.mutate_unlocked(f)
    }

    fn read_unlocked(&self) -> Result<Publications> {
        let file = match File::open(&self.paths.json) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let rows: Publications = serde_json::from_reader(BufReader::new(file))?;
        validate_publications_with(&rows, &self.config, &self.validator)?;
        Ok(rows)
    }

    fn mutate_unlocked<R>(&self, f: impl FnOnce(&mut Publications) -> Result<R>) -> Result<R> {
        let mut rows = self.read_unlocked()?;
        let output = f(&mut rows)?;
        validate_publications_with(&rows, &self.config, &self.validator)?;
        let parent = self
            .paths
            .json
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut temp = NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(temp.as_file_mut(), &rows)?;
        temp.as_file_mut().flush()?;
        temp.as_file_mut().sync_all()?;
        commit(temp, &self.paths.json, parent, |parent| {
            File::open(parent)?.sync_all()
        })?;
        Ok(output)
    }

    fn lock_file(&self) -> Result<File> {
        let mut path = PathBuf::from(self.paths.json.as_os_str());
        path.as_mut_os_string().push(".lock");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?)
    }
}

fn commit(
    temp: NamedTempFile,
    target: &std::path::Path,
    parent: &std::path::Path,
    sync_parent: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> Result<()> {
    #[cfg(test)]
    let injected_failure = take_commit_failure_for_test();
    #[cfg(test)]
    if injected_failure == Some(CommitFailureForTest::BeforeRename) {
        return Err(Error::Io(std::io::Error::other(
            "injected pre-rename failure",
        )));
    }
    temp.persist(target).map_err(|e| Error::Io(e.error))?;
    #[cfg(test)]
    if injected_failure == Some(CommitFailureForTest::AfterRename) {
        return Err(Error::DurabilityUncertain {
            path: target.to_owned(),
            source: std::io::Error::other("injected post-rename directory sync failure"),
        });
    }
    sync_parent(parent).map_err(|source| Error::DurabilityUncertain {
        path: target.to_owned(),
        source,
    })
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitFailureForTest {
    BeforeRename,
    AfterRename,
}

#[cfg(test)]
thread_local! {
    static COMMIT_FAILURE_FOR_TEST: std::cell::Cell<Option<CommitFailureForTest>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_commit_for_test(failure: CommitFailureForTest) {
    COMMIT_FAILURE_FOR_TEST.with(|slot| slot.set(Some(failure)));
}

#[cfg(test)]
fn take_commit_failure_for_test() -> Option<CommitFailureForTest> {
    COMMIT_FAILURE_FOR_TEST.with(|slot| slot.take())
}

#[cfg(test)]
mod commit_tests {
    use super::*;

    #[test]
    fn directory_sync_failure_reports_uncertain_commit_and_keeps_new_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("publications.json");
        let mut temp = NamedTempFile::new_in(dir.path()).unwrap();
        temp.write_all(b"new committed bytes").unwrap();
        let result = commit(temp, &target, dir.path(), |_| {
            Err(std::io::Error::other("injected directory sync failure"))
        });
        match result {
            Err(Error::DurabilityUncertain { path, source }) => {
                assert_eq!(path, target);
                assert_eq!(source.to_string(), "injected directory sync failure");
            }
            other => panic!("expected durability-uncertain result, got {other:?}"),
        }
        assert_eq!(fs::read(target).unwrap(), b"new committed bytes");
    }

    #[test]
    fn rename_failure_preserves_original_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("publications.json");
        fs::write(&target, b"original bytes").unwrap();
        let mut temp = NamedTempFile::new_in(dir.path()).unwrap();
        temp.write_all(b"replacement bytes").unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = commit(temp, &target, dir.path(), |_| Ok(()));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(result, Err(Error::Io(_))));
        assert_eq!(fs::read(target).unwrap(), b"original bytes");
    }
}
