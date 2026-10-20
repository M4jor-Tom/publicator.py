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
        let result = self.read_unlocked();
        FileExt::unlock(&lock)?;
        result
    }

    pub fn mutate<R>(&self, f: impl FnOnce(&mut Publications) -> Result<R>) -> Result<R> {
        let _process = MUTATION_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let lock = self.lock_file()?;
        lock.lock_exclusive()?;
        let result = self.mutate_unlocked(f);
        FileExt::unlock(&lock)?;
        result
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
        let original = match fs::read(&self.paths.json) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
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
        let mut backup = if let Some(bytes) = &original {
            let mut backup = NamedTempFile::new_in(parent)?;
            backup.write_all(bytes)?;
            backup.as_file_mut().sync_all()?;
            Some(backup)
        } else {
            None
        };
        temp.persist(&self.paths.json)
            .map_err(|e| Error::Io(e.error))?;
        if let Err(sync_error) = File::open(parent).and_then(|directory| directory.sync_all()) {
            let rollback = match backup.take() {
                Some(backup) => backup
                    .persist(&self.paths.json)
                    .map(|_| ())
                    .map_err(|e| e.error),
                None => fs::remove_file(&self.paths.json),
            };
            rollback?;
            File::open(parent)?.sync_all()?;
            return Err(sync_error.into());
        }
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
            .read(true)
            .write(true)
            .open(path)?)
    }
}
