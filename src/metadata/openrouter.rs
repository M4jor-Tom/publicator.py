use super::{Generated, MetadataClient, metadata_prompt, parse_metadata, title_description_schema};
use crate::{
    error::{Error, Result},
    images::PreparedImage,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogModel {
    pub id: String,
    pub image_input: bool,
    pub free: bool,
    pub schema_supported: Option<bool>,
}
impl CatalogModel {
    pub fn from_api(value: &Value) -> Result<Self> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Protocol("catalogue model missing id".into()))?
            .to_owned();
        let image_input = value
            .pointer("/architecture/input_modalities")
            .and_then(Value::as_array)
            .is_some_and(|xs| xs.iter().any(|v| v.as_str() == Some("image")));
        let pricing = value
            .get("pricing")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Protocol(format!("catalogue model {id} missing pricing")))?;
        let free = ["prompt", "completion"]
            .iter()
            .all(|key| pricing.get(*key).and_then(price).is_some_and(|n| n == 0.0))
            && pricing.values().all(|v| price(v).is_some_and(|n| n == 0.0));
        let schema_supported = value
            .get("supported_parameters")
            .and_then(Value::as_array)
            .map(|xs| xs.iter().any(|v| v.as_str() == Some("response_format")));
        Ok(Self {
            id,
            image_input,
            free,
            schema_supported,
        })
    }
}
fn price(v: &Value) -> Option<f64> {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.as_f64())
}
impl MetadataClient {
    async fn bounded(
        &self,
        request: reqwest::RequestBuilder,
        deadline: tokio::time::Instant,
        secrets: &[&str],
    ) -> Result<Vec<u8>> {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::Provider("OpenRouter deadline exceeded".into()));
        }
        let request = request.timeout(remaining);
        let response = tokio::time::timeout(remaining, async {
            let response = request.send().await.map_err(|e| {
                if e.is_timeout() {
                    Error::Provider("OpenRouter deadline exceeded".into())
                } else {
                    Error::Provider(e.to_string())
                }
            })?;
            let status = response.status();
            let bytes = response.bytes().await.map_err(|e| {
                if e.is_timeout() {
                    Error::Provider("OpenRouter deadline exceeded".into())
                } else {
                    Error::Provider(e.to_string())
                }
            })?;
            Ok::<_, Error>((status, bytes))
        })
        .await
        .map_err(|_| Error::Provider("OpenRouter deadline exceeded".into()))??;
        if !response.0.is_success() {
            return Err(Error::Provider(format!(
                "OpenRouter HTTP {}: {}",
                response.0.as_u16(),
                safe_error_detail(&response.1, secrets)
            )));
        }
        Ok(response.1.to_vec())
    }
    pub async fn catalogue(&self, deadline: tokio::time::Instant) -> Result<Vec<CatalogModel>> {
        if let Some((at, models)) = self.catalogue.lock().unwrap().as_ref()
            && at.elapsed() < Duration::from_secs(300)
        {
            return Ok(models.clone());
        }
        let url = format!("{}/models", self.endpoints.openrouter.trim_end_matches('/'));
        let key = self
            .openrouter_key
            .as_ref()
            .cloned()
            .or_else(|| std::env::var("OPENROUTER_KEY").ok());
        let mut req = self.http.get(url);
        if let Some(key) = &key {
            req = req.bearer_auth(key);
        }
        let secrets = key.as_deref().into_iter().collect::<Vec<_>>();
        let bytes = self.bounded(req, deadline, &secrets).await?;
        let v: Value = serde_json::from_slice(&bytes)?;
        let rows = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Protocol("OpenRouter catalogue missing data".into()))?;
        let models = rows
            .iter()
            .map(CatalogModel::from_api)
            .collect::<Result<Vec<_>>>()?;
        *self.catalogue.lock().unwrap() = Some((Instant::now(), models.clone()));
        Ok(models)
    }
    pub async fn openrouter_attempt(
        &self,
        image: &PreparedImage,
        model: &str,
        schema: bool,
        deadline: tokio::time::Instant,
    ) -> Result<Generated> {
        let key = self
            .openrouter_key
            .as_ref()
            .cloned()
            .or_else(|| std::env::var("OPENROUTER_KEY").ok())
            .ok_or_else(|| Error::Provider("OPENROUTER_KEY is not set".into()))?;
        let model = model.strip_prefix("openrouter/").unwrap_or(model);
        let rejected = self.rejected_schemas.lock().unwrap().contains(model);
        let use_schema = schema && !rejected;
        let data_url = super::image_data_url(image)?;
        let body = completion_body(model, &data_url, use_schema);
        let url = format!(
            "{}/chat/completions",
            self.endpoints.openrouter.trim_end_matches('/')
        );
        let send = |body: Value| self.http.post(&url).bearer_auth(&key).json(&body);
        let bytes = match self
            .bounded(send(body), deadline, &[key.as_str(), data_url.as_str()])
            .await
        {
            Err(error) if use_schema && unsupported_schema(&error.to_string()) => {
                self.rejected_schemas
                    .lock()
                    .unwrap()
                    .insert(model.to_owned());
                let retry = completion_body(model, &data_url, false);
                self.bounded(send(retry), deadline, &[key.as_str(), data_url.as_str()])
                    .await?
            }
            result => result?,
        };
        parse_completion(&bytes, model)
    }
    pub async fn openrouter_generate(
        &self,
        image: &PreparedImage,
        request: &super::GenerationRequest,
        preferred: Option<&str>,
    ) -> Result<Generated> {
        let deadline = tokio::time::Instant::now() + request.timeout;
        let selected = preferred.unwrap_or(&request.model.openrouter_name);
        let selected = selected.strip_prefix("openrouter/").unwrap_or(selected);
        let generated = match self
            .openrouter_attempt(image, selected, true, deadline)
            .await
        {
            Ok(generated) => generated,
            Err(original) => {
                self.openrouter_fallback(image, selected, deadline, original)
                    .await?
            }
        };
        eprintln!("OpenRouter metadata generated by {}", generated.model_used);
        Ok(generated)
    }
    async fn openrouter_fallback(
        &self,
        image: &PreparedImage,
        selected: &str,
        deadline: tokio::time::Instant,
        original: Error,
    ) -> Result<Generated> {
        let models = match self.catalogue(deadline).await {
            Ok(models) => models,
            Err(_) => return Err(original),
        };
        let mut last = original;
        for candidate in fallback_models(models, selected) {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            if candidate.schema_supported == Some(false) {
                self.rejected_schemas
                    .lock()
                    .unwrap()
                    .insert(candidate.id.clone());
            }
            match self
                .openrouter_attempt(
                    image,
                    &candidate.id,
                    candidate.schema_supported.unwrap_or(true),
                    deadline,
                )
                .await
            {
                Ok(generated) => return Ok(generated),
                Err(error) => last = error,
            }
        }
        Err(last)
    }
}
fn completion_body(model: &str, data_url: &str, use_schema: bool) -> Value {
    let mut prompt = metadata_prompt().to_owned();
    if !use_schema {
        prompt.push_str(" Return JSON matching this shape: ");
        prompt.push_str(&title_description_schema().to_string());
    }
    let content =
        json!([{"type":"text","text":prompt},{"type":"image_url","image_url":{"url":data_url}}]);
    let mut body = json!({"model":model,"messages":[{"role":"user","content":content}]});
    if use_schema {
        body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"title_description","strict":true,"schema":title_description_schema()}});
    }
    body
}
fn parse_completion(bytes: &[u8], model: &str) -> Result<Generated> {
    let v: Value = serde_json::from_slice(bytes)?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("OpenRouter response missing message content".into()))?;
    Ok(Generated {
        metadata: parse_metadata(content)?,
        model_used: model.to_owned(),
    })
}
fn fallback_models(models: Vec<CatalogModel>, selected: &str) -> Vec<CatalogModel> {
    let original_vendor = selected.split('/').next().unwrap_or("");
    let mut other = Vec::new();
    let mut sibling = Vec::new();
    let mut seen = HashSet::from([selected.to_owned()]);
    for model in models
        .into_iter()
        .filter(|model| model.image_input && model.free)
    {
        if seen.insert(model.id.clone()) {
            if model.id.split('/').next().unwrap_or("") == original_vendor {
                sibling.push(model);
            } else {
                other.push(model);
            }
        }
    }
    other.extend(sibling);
    other
}
fn safe_error_detail(body: &[u8], secrets: &[&str]) -> String {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let detail = value
        .pointer("/error/message")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
    let mut detail = detail;
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        detail = detail.replace(secret, "[REDACTED]");
    }
    detail = redact_image_data_uris(&detail);
    let mut chars = detail.chars();
    let bounded: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn redact_image_data_uris(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("data:image/") {
        output.push_str(&rest[..start]);
        let uri = &rest[start..];
        let end = uri
            .find([' ', '\t', '\r', '\n', '"', '\'', '<', '>'])
            .unwrap_or(uri.len());
        output.push_str("[REDACTED]");
        rest = &uri[end..];
    }
    output.push_str(rest);
    output
}

fn unsupported_schema(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("response_format")
        && (m.contains("not support")
            || m.contains("unsupported")
            || m.contains("not allowed")
            || m.contains("does not support"))
}
