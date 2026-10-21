use crate::{
    error::{Error, Result},
    images::PreparedImage,
    model::validate_metadata,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::LazyLock, time::Duration};

mod local;
mod openrouter;
pub use local::{Fit, LocalStatus, consent_required};
pub use openrouter::CatalogModel;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    OpenRouter,
    Local,
}

pub fn available_sources() -> &'static [Source] {
    &[Source::OpenRouter, Source::Local]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelOption {
    pub ollama_name: String,
    pub openrouter_name: String,
    pub llmfit_name: String,
}

pub static VISION_MODELS: LazyLock<[ModelOption; 2]> = LazyLock::new(|| {
    [
        ModelOption {
            ollama_name: "gemma4:26b-a4b".into(),
            openrouter_name: "openrouter/google/gemma-4-26b-a4b-it:free".into(),
            llmfit_name: "google/gemma-4-26B-A4B-it".into(),
        },
        ModelOption {
            ollama_name: "gemma4:31b".into(),
            openrouter_name: "openrouter/google/gemma-4-31b-it:free".into(),
            llmfit_name: "google/gemma-4-31B-it".into(),
        },
    ]
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationRequest {
    pub source: Source,
    pub model: ModelOption,
    pub timeout: Duration,
    pub install_missing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Metadata {
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generated {
    pub metadata: Metadata,
    pub model_used: String,
}

pub struct Endpoints {
    pub openrouter: String,
    pub ollama: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            openrouter: "https://openrouter.ai/api/v1".into(),
            ollama: "http://localhost:11434".into(),
        }
    }
}

pub struct MetadataClient {
    pub(crate) http: reqwest::Client,
    pub(crate) endpoints: Endpoints,
    _llmfit_program: PathBuf,
    pub(crate) rejected_schemas: std::sync::Mutex<std::collections::HashSet<String>>,
    pub(crate) catalogue: std::sync::Mutex<Option<(std::time::Instant, Vec<CatalogModel>)>>,
    pub(crate) openrouter_key: Option<String>,
}
impl MetadataClient {
    pub fn new(http: reqwest::Client, endpoints: Endpoints, llmfit_program: PathBuf) -> Self {
        Self {
            http,
            endpoints,
            _llmfit_program: llmfit_program,
            rejected_schemas: std::sync::Mutex::new(std::collections::HashSet::new()),
            catalogue: std::sync::Mutex::new(None),
            openrouter_key: None,
        }
    }
    pub fn with_openrouter_key(mut self, key: impl Into<String>) -> Self {
        self.openrouter_key = Some(key.into());
        self
    }

    pub async fn generate(
        &self,
        image: &PreparedImage,
        request: &GenerationRequest,
    ) -> Result<Generated> {
        match request.source {
            Source::OpenRouter => self.openrouter_generate(image, request, None).await,
            Source::Local => {
                let deadline = tokio::time::Instant::now() + request.timeout;
                self.prepare_local(&request.model, request.install_missing, deadline)
                    .await?;
                self.local_generate_until(image, request, deadline).await
            }
        }
    }
}

pub fn image_data_url(image: &PreparedImage) -> Result<String> {
    if !matches!(
        image.mime.as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/bmp" | "image/tiff"
    ) {
        return Err(Error::InvalidState(format!(
            "unsupported AI image MIME type: {}",
            image.mime
        )));
    }
    let bytes = std::fs::read(&image.path)?;
    Ok(format!(
        "data:{};base64,{}",
        image.mime,
        openssl::base64::encode_block(&bytes)
    ))
}

pub fn parse_metadata(content: &str) -> Result<Metadata> {
    let begin = content
        .find('{')
        .ok_or_else(|| Error::Provider("no JSON object".into()))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| Error::Provider("no JSON object".into()))?;
    if end < begin {
        return Err(Error::Provider("no JSON object".into()));
    }
    let pair: Metadata = serde_json::from_str(&content[begin..=end])?;
    validate_metadata(&pair.title, &pair.description)?;
    Ok(pair)
}

pub fn title_description_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string", "description": "<= 50 chars, evocative"},
            "description": {"type": "string", "description": "2-3 sentences, invented story of what could be happening in the scene"}
        },
        "required": ["title", "description"]
    })
}

pub fn metadata_prompt() -> &'static str {
    "Look at the artwork. Reply with a title (<= 50 characters, evocative) and a description (2-3 sentences) that invents a short backstory for the scene — imagine what could be happening in it and narrate that. No hashtags, no emojis."
}
