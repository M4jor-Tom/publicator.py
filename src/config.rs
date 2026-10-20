use crate::error::{Error, Result};
use crate::model::Publications;
use openssl::hash::{Hasher, MessageDigest};
use pcre2::bytes::Regex;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub json: PathBuf,
}
impl Paths {
    pub fn resolve(data_dir: Option<&Path>, json: Option<&Path>) -> Result<Self> {
        let data_dir = match data_dir {
            Some(p) => p.to_path_buf(),
            None => std::env::current_dir()?,
        };
        let data_dir = if data_dir.is_absolute() {
            data_dir
        } else {
            std::env::current_dir()?.join(data_dir)
        };
        let json = json.unwrap_or(Path::new("publications.json"));
        let json = if json.is_absolute() {
            json.to_path_buf()
        } else {
            data_dir.join(json)
        };
        Ok(Self { data_dir, json })
    }
}
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub tiers: Vec<String>,
    pub galleries: Vec<String>,
    pub publicable: Vec<String>,
    pub tags: Option<String>,
    pub schedule: toml::Table,
    pub prompts: Option<PromptConfig>,
}
#[derive(Clone, Debug)]
pub struct PromptConfig {
    pub repo: String,
    pub filename: String,
    pub version_hash: String,
}
pub struct PromptGrammar {
    pattern: Regex,
    digest: MessageDigest,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageIdentity {
    pub version: String,
    pub lineage: Option<String>,
}

impl Config {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join("publicator.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        #[derive(Deserialize, Default)]
        struct Raw {
            #[serde(default)]
            publicable: Vec<String>,
            tags: Option<String>,
            #[serde(default)]
            schedule: toml::Table,
            #[serde(default)]
            deviantart: Da,
            prompts: Option<Prompts>,
        }
        #[derive(Deserialize, Default)]
        struct Da {
            #[serde(default)]
            tiers: Vec<String>,
            #[serde(default)]
            galleries: Vec<String>,
        }
        #[derive(Deserialize)]
        struct Prompts {
            repo: String,
            filename: String,
            version_hash: String,
        }
        let raw: Raw =
            toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        let prompts = raw.prompts.map(|p| PromptConfig {
            repo: p.repo,
            filename: p.filename,
            version_hash: p.version_hash,
        });
        if let Some(ref p) = prompts {
            PromptGrammar::compile(p)?;
        }
        Ok(Self {
            tiers: raw.deviantart.tiers,
            galleries: raw.deviantart.galleries,
            publicable: raw.publicable,
            tags: raw.tags,
            schedule: raw.schedule,
            prompts,
        })
    }
}
impl PromptGrammar {
    pub fn compile(config: &PromptConfig) -> Result<Self> {
        let pattern = Regex::new(&format!(r"\A(?:{})", config.filename))
            .map_err(|e| Error::Config(format!("publicator.toml [prompts].filename: {e}")))?;
        if !pattern
            .capture_names()
            .iter()
            .any(|n| n.as_deref() == Some("version"))
        {
            return Err(Error::Config("publicator.toml [prompts].filename: missing required (?P<version>...) capture group".into()));
        }
        let digest = match config.version_hash.as_str() {
            "sha1" => MessageDigest::sha1(),
            "sha256" => MessageDigest::sha256(),
            _ => {
                return Err(Error::Config(format!(
                    "publicator.toml [prompts].version_hash: unknown digest {:?}",
                    config.version_hash
                )));
            }
        };
        Ok(Self { pattern, digest })
    }
    pub fn identity(&self, basename: &str) -> Result<Option<ImageIdentity>> {
        let Some(c) = self
            .pattern
            .captures(basename.as_bytes())
            .map_err(|e| Error::Config(e.to_string()))?
        else {
            return Ok(None);
        };
        let Some(v) = c.name("version") else {
            return Ok(None);
        };
        Ok(Some(ImageIdentity {
            version: String::from_utf8_lossy(v.as_bytes()).into_owned(),
            lineage: c
                .name("lineage")
                .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned()),
        }))
    }
    pub fn digest(&self, bytes: &[u8]) -> Result<String> {
        let mut h = Hasher::new(self.digest).map_err(|e| Error::Config(e.to_string()))?;
        h.update(bytes).map_err(|e| Error::Config(e.to_string()))?;
        Ok(h.finish()
            .map_err(|e| Error::Config(e.to_string()))?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect())
    }
}
pub fn validate_publications(rows: &Publications, config: &Config) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../assets/publicationsSchema.json"))?;
    let validator = jsonschema::validator_for(&schema).map_err(|e| Error::Config(e.to_string()))?;
    if let Some(e) = validator
        .iter_errors(&serde_json::Value::Array(rows.clone()))
        .next()
    {
        return Err(Error::Validation(e.to_string()));
    }
    for row in rows {
        let uuid = row
            .get("uuid")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("publication");
        if let Some(apparitions) = row.get("apparitions").and_then(serde_json::Value::as_array) {
            for a in apparitions {
                for key in [
                    "urlElsePublicationName",
                    "apparitionTimestampIfDifferentThanSubmission",
                    "tier",
                ] {
                    if a.get(key).is_some() {
                        return Err(Error::Validation(format!("{uuid}: obsolete {key}")));
                    }
                }
                if let Some(url) = a.get("url").and_then(serde_json::Value::as_str) {
                    let valid = url::Url::parse(url)
                        .map(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
                        .unwrap_or(false);
                    if !valid {
                        return Err(Error::Validation(format!(
                            "{uuid}: apparition url must be an HTTP(S) URL with a host"
                        )));
                    }
                }
                if a.get("platformName").and_then(serde_json::Value::as_str) == Some("deviantart") {
                    for (key, allowed) in
                        [("tiers", &config.tiers), ("galleries", &config.galleries)]
                    {
                        if let Some(vs) = a.get(key).and_then(serde_json::Value::as_array) {
                            for v in vs {
                                if let Some(v) = v.as_str()
                                    && !allowed.iter().any(|x| x == v)
                                {
                                    return Err(Error::Validation(format!(
                                        "{uuid}: {key} {v:?} not in publicator.toml"
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
