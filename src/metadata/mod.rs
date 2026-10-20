use crate::{
    error::{Error, Result},
    model::validate_metadata,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::LazyLock, time::Duration};

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
    _http: reqwest::Client,
    _endpoints: Endpoints,
    _llmfit_program: PathBuf,
}
impl MetadataClient {
    pub fn new(http: reqwest::Client, endpoints: Endpoints, llmfit_program: PathBuf) -> Self {
        Self {
            _http: http,
            _endpoints: endpoints,
            _llmfit_program: llmfit_program,
        }
    }
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
