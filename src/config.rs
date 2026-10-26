use crate::error::{Error, Result};
use crate::model::Publications;
use openssl::hash::{Hasher, MessageDigest};
use pcre2::bytes::{Regex, RegexBuilder};
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
        let json = resolve_database_path(json)?;
        Ok(Self { data_dir, json })
    }
}

fn resolve_database_path(json: PathBuf) -> Result<PathBuf> {
    Ok(match std::fs::canonicalize(&json) {
        Ok(target) => target,
        Err(error) => {
            if std::fs::symlink_metadata(&json).is_ok() {
                return Err(error.into());
            }
            let parent = json.parent().unwrap_or_else(|| Path::new("."));
            match std::fs::canonicalize(parent) {
                Ok(parent) => parent.join(json.file_name().ok_or_else(|| {
                    Error::Config("publication database path has no filename".into())
                })?),
                Err(parent_error) if parent_error.kind() == std::io::ErrorKind::NotFound => json,
                Err(parent_error) => return Err(parent_error.into()),
            }
        }
    })
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
            prompts: Option<toml::Table>,
        }
        #[derive(Deserialize, Default)]
        struct Da {
            #[serde(default)]
            tiers: Vec<String>,
            #[serde(default)]
            galleries: Vec<String>,
        }
        let raw: Raw =
            toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        let prompts = raw
            .prompts
            .filter(|table| !table.is_empty())
            .map(|table| {
                let required = |key: &str| {
                    table
                        .get(key)
                        .and_then(toml::Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            Error::Config(format!(
                                "publicator.toml [prompts].{key}: missing or invalid string"
                            ))
                        })
                };
                Ok::<PromptConfig, Error>(PromptConfig {
                    repo: required("repo")?,
                    filename: required("filename")?,
                    version_hash: required("version_hash")?,
                })
            })
            .transpose()?;
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
        let pattern = RegexBuilder::new()
            .utf(true)
            .ucp(true)
            .build(&format!(r"\A(?:{})", config.filename))
            .map_err(|e| Error::Config(format!("publicator.toml [prompts].filename: {e}")))?;
        if !pattern
            .capture_names()
            .iter()
            .any(|n| n.as_deref() == Some("version"))
        {
            return Err(Error::Config("publicator.toml [prompts].filename: missing required (?P<version>...) capture group".into()));
        }
        let digest_name = match config.version_hash.as_str() {
            "blake2b" => "BLAKE2B-512".to_owned(),
            "blake2s" => "BLAKE2S-256".to_owned(),
            name if name.starts_with("sha3_") => name.replace('_', "-"),
            name => name.to_owned(),
        };
        let digest = MessageDigest::from_name(&digest_name).ok_or_else(|| {
            Error::Config(format!(
                "publicator.toml [prompts].version_hash: unsupported digest {:?}",
                config.version_hash
            ))
        })?;
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
                .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
                .filter(|lineage| !lineage.is_empty()),
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
    validate_publications_with(rows, config, &validator)
}

pub(crate) fn validate_publications_with(
    rows: &Publications,
    config: &Config,
    validator: &jsonschema::Validator,
) -> Result<()> {
    for row in rows {
        let uuid = row
            .get("uuid")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("publication");
        for a in row
            .get("apparitions")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            validate_apparition(a, uuid, config)?;
        }
    }
    if let Some(e) = validator
        .iter_errors(&serde_json::Value::Array(rows.clone()))
        .next()
    {
        return Err(Error::Validation(e.to_string()));
    }
    Ok(())
}

fn validate_apparition(a: &serde_json::Value, uuid: &str, config: &Config) -> Result<()> {
    for key in [
        "urlElsePublicationName",
        "apparitionTimestampIfDifferentThanSubmission",
        "tier",
    ] {
        if a.get(key).is_some() {
            return Err(Error::Validation(format!("{uuid}: obsolete {key}")));
        }
    }
    if let Some(url) = a.get("url").and_then(serde_json::Value::as_str)
        && !valid_apparition_url(url)
    {
        return Err(Error::Validation(format!(
            "{uuid}: apparition url must be an HTTP(S) URL with a host"
        )));
    }
    if a.get("platformName").and_then(serde_json::Value::as_str) == Some("deviantart") {
        for (key, allowed) in [("tiers", &config.tiers), ("galleries", &config.galleries)] {
            validate_allowed_options(a, uuid, key, allowed)?;
        }
    }
    Ok(())
}

fn valid_apparition_url(url: &str) -> bool {
    let valid_authority = url
        .split_once("://")
        .is_some_and(|(_, authority)| !authority.starts_with('/') && !authority.starts_with('\\'));
    valid_authority
        && url::Url::parse(url)
            .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

fn validate_allowed_options(
    a: &serde_json::Value,
    uuid: &str,
    key: &str,
    allowed: &[String],
) -> Result<()> {
    for v in a
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        if !allowed.iter().any(|x| x == v) {
            let label = if key == "galleries" {
                "gallery"
            } else {
                "tier"
            };
            return Err(Error::Validation(format!(
                "{uuid}: {label} {v:?} not in publicator.toml"
            )));
        }
    }
    Ok(())
}
