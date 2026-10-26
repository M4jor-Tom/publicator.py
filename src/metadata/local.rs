use super::{
    Generated, GenerationRequest, MetadataClient, ModelOption, image_data_url, metadata_prompt,
    parse_metadata, title_description_schema,
};
use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fit {
    pub fit: bool,
    pub level: String,
    pub reason: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalStatus {
    pub fit: Fit,
    pub installed: bool,
    pub available: bool,
}
pub fn consent_required(status: &LocalStatus) -> bool {
    status.fit.fit && status.available && !status.installed
}

impl MetadataClient {
    pub async fn local_fit(&self, model: &ModelOption) -> Result<Fit> {
        let output = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::process::Command::new(&self._llmfit_program)
                .args(["info", &model.llmfit_name, "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(Error::from)
        })
        .await;
        let output = match output {
            Ok(Ok(o)) if o.status.success() => o,
            Ok(Ok(o)) => {
                return Ok(unavailable(
                    String::from_utf8_lossy(&o.stderr).trim().to_owned(),
                ));
            }
            Ok(Err(e)) => return Ok(unavailable(e.to_string())),
            Err(_) => return Ok(unavailable("llmfit timed out".into())),
        };
        Ok(parse_fit_report(&output.stdout, model))
    }
    pub async fn local_status(
        &self,
        model: &ModelOption,
        deadline: tokio::time::Instant,
    ) -> Result<LocalStatus> {
        let fit = self.bounded_fit(model, deadline).await?;
        if !fit.fit {
            return Ok(LocalStatus {
                fit,
                installed: false,
                available: false,
            });
        }
        let value = self
            .local_request(
                self.http.get(format!(
                    "{}/api/tags",
                    self.endpoints.ollama.trim_end_matches('/')
                )),
                deadline,
            )
            .await;
        match value {
            Ok(v) => {
                let models = v
                    .get("models")
                    .and_then(Value::as_array)
                    .ok_or_else(|| Error::Protocol("Ollama tags missing models".into()))?;
                let installed = models
                    .iter()
                    .any(|m| m.get("name").and_then(Value::as_str) == Some(&model.ollama_name));
                Ok(LocalStatus {
                    fit,
                    installed,
                    available: true,
                })
            }
            Err(_) => Ok(LocalStatus {
                fit,
                installed: false,
                available: false,
            }),
        }
    }
    async fn bounded_fit(
        &self,
        model: &ModelOption,
        deadline: tokio::time::Instant,
    ) -> Result<Fit> {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::Provider("local metadata deadline exceeded".into()));
        }
        tokio::time::timeout(remaining, self.local_fit(model))
            .await
            .map_err(|_| Error::Provider("local metadata deadline exceeded".into()))?
    }
    pub async fn prepare_local(
        &self,
        model: &ModelOption,
        install: bool,
        deadline: tokio::time::Instant,
    ) -> Result<()> {
        let status = self.local_status(model, deadline).await?;
        if !status.fit.fit {
            return Err(Error::Provider(format!(
                "Local model does not fit: {}",
                status.fit.reason
            )));
        }
        if !status.available {
            return Err(Error::Provider("Ollama is unavailable".into()));
        }
        if !status.installed {
            if !install {
                return Err(Error::Provider(format!(
                    "Local model {} is not installed",
                    model.ollama_name
                )));
            }
            let value = self
                .local_request(
                    self.http
                        .post(format!(
                            "{}/api/pull",
                            self.endpoints.ollama.trim_end_matches('/')
                        ))
                        .json(&json!({"model":model.ollama_name,"stream":false})),
                    deadline,
                )
                .await?;
            if value.get("status").and_then(Value::as_str) != Some("success") {
                return Err(Error::Provider(format!(
                    "Ollama failed to pull {}",
                    model.ollama_name
                )));
            }
        }
        Ok(())
    }
    pub async fn local_generate(
        &self,
        image: &crate::images::PreparedImage,
        request: &GenerationRequest,
    ) -> Result<Generated> {
        let deadline = tokio::time::Instant::now() + request.timeout;
        self.local_generate_until(image, request, deadline).await
    }
    pub(crate) async fn local_generate_until(
        &self,
        image: &crate::images::PreparedImage,
        request: &GenerationRequest,
        deadline: tokio::time::Instant,
    ) -> Result<Generated> {
        let encoded = image_data_url(image)?
            .split_once(',')
            .ok_or_else(|| Error::Protocol("invalid image data URL".into()))?
            .1
            .to_owned();
        let body = json!({"model":request.model.ollama_name,"stream":false,"format":title_description_schema(),"messages":[{"role":"user","content":metadata_prompt(),"images":[encoded]}]});
        let response = self
            .local_request(
                self.http
                    .post(format!(
                        "{}/api/chat",
                        self.endpoints.ollama.trim_end_matches('/')
                    ))
                    .json(&body),
                deadline,
            )
            .await?;
        let content = response
            .pointer("/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("Ollama response missing message content".into()))?;
        Ok(Generated {
            metadata: parse_metadata(content)?,
            model_used: request.model.ollama_name.clone(),
        })
    }
    async fn local_request(
        &self,
        request: reqwest::RequestBuilder,
        deadline: tokio::time::Instant,
    ) -> Result<Value> {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::Provider("local metadata deadline exceeded".into()));
        }
        let response = tokio::time::timeout(remaining, async {
            let response = request.timeout(remaining).send().await?;
            let status = response.status();
            let bytes = response.bytes().await?;
            Ok::<_, reqwest::Error>((status, bytes))
        })
        .await
        .map_err(|_| Error::Provider("local metadata deadline exceeded".into()))?
        .map_err(|e| {
            if e.is_timeout() {
                Error::Provider("local metadata deadline exceeded".into())
            } else {
                Error::Provider(e.to_string())
            }
        })?;
        let status = response.0;
        let bytes = response.1;
        if !status.is_success() {
            return Err(Error::Provider(format!("Ollama HTTP {}", status.as_u16())));
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::Protocol(format!("invalid Ollama JSON: {e}")))
    }
}
fn unavailable(reason: String) -> Fit {
    Fit {
        fit: false,
        level: "Unavailable".into(),
        reason,
    }
}

fn parse_fit_report(stdout: &[u8], model: &ModelOption) -> Fit {
    let parsed: Value = match serde_json::from_slice(stdout) {
        Ok(v) => v,
        Err(e) => return unavailable(e.to_string()),
    };
    let row = parsed
        .get("models")
        .and_then(Value::as_array)
        .and_then(|xs| {
            xs.iter()
                .find(|x| x.get("name").and_then(Value::as_str) == Some(&model.llmfit_name))
        });
    let Some(row) = row else {
        return unavailable("llmfit model missing from report".into());
    };
    let Some(level) = row.get("fit_level").and_then(Value::as_str) else {
        return unavailable("llmfit fit_level missing".into());
    };
    let reason = match row.get("notes") {
        Some(Value::Array(xs)) => xs
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("; "),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    Fit {
        fit: level == "Good",
        level: level.into(),
        reason: if level != "Good" && reason.is_empty() {
            format!("llmfit fit_level: {level}")
        } else {
            reason
        },
    }
}
