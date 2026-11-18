//! OpenAI-compatible generation over HTTPS, and loopback generation and embeddings for evaluation.

#[cfg(feature = "eval")]
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::GenerationConfig;

use super::{Backend, RuntimeError};

const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct OpenAiCompatible {
    provider: String,
    endpoint: Endpoint,
    models: OnceLock<Option<Vec<String>>>,
}

enum Endpoint {
    #[cfg(feature = "eval")]
    Loopback(SocketAddr),
    Https {
        base_url: String,
        key_file: PathBuf,
        agent: ureq::Agent,
    },
}

impl OpenAiCompatible {
    pub(crate) fn remote(config: &GenerationConfig) -> Result<Self> {
        let provider = provider_name(&config.provider)?;
        let base_url = config.base_url.trim_end_matches('/');
        if !base_url.starts_with("https://") {
            bail!("the generation provider needs an HTTPS base URL");
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(LIST_TIMEOUT)
            .https_only(true)
            .redirects(0)
            .build();
        Ok(Self {
            provider,
            endpoint: Endpoint::Https {
                base_url: base_url.into(),
                key_file: config.api_key_file.clone(),
                agent,
            },
            models: OnceLock::new(),
        })
    }

    #[cfg(feature = "eval")]
    pub(crate) fn new(provider: &str, addr: SocketAddr) -> Result<Self> {
        let provider = provider_name(provider)?;
        if !addr.ip().is_loopback() {
            bail!("{addr} is not on this machine");
        }
        Ok(Self {
            provider,
            endpoint: Endpoint::Loopback(addr),
            models: OnceLock::new(),
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(String, Value), RuntimeError> {
        match &self.endpoint {
            #[cfg(feature = "eval")]
            Endpoint::Loopback(addr) => {
                super::http::request(addr, method, &format!("/v1{path}"), body, timeout)
            }
            Endpoint::Https {
                base_url,
                key_file,
                agent,
            } => {
                let key = std::fs::read_to_string(key_file).map_err(|_| {
                    RuntimeError::Failed("cannot read the generation API key file".into())
                })?;
                let key = key.lines().next().unwrap_or("").trim();
                if key.is_empty() || !key.bytes().all(|b| b.is_ascii_graphic()) {
                    return Err(RuntimeError::Failed("invalid generation API key".into()));
                }
                let request = agent
                    .request(method, &format!("{base_url}{path}"))
                    .set("Authorization", &format!("Bearer {key}"))
                    .timeout(timeout);
                let result = match body {
                    Some(body) => request.send_json(body),
                    None => request.call(),
                };
                let response = match result {
                    Ok(response) => response,
                    Err(ureq::Error::Status(status, _)) => {
                        return Err(RuntimeError::Failed(format!("{path}: HTTP {status}")));
                    }
                    Err(ureq::Error::Transport(_)) => return Err(RuntimeError::NotRunning),
                };
                let status = response.status().to_string();
                let value = response
                    .into_json()
                    .map_err(|_| RuntimeError::Failed(format!("{path}: invalid JSON response")))?;
                Ok((status, value))
            }
        }
    }

    fn call(&self, path: &str, body: &Value) -> Result<Value, RuntimeError> {
        let (status, value) = self.request("POST", path, Some(body), CALL_TIMEOUT)?;
        if status != "200" {
            let why = value["error"]["message"]
                .as_str()
                .or(value["error"].as_str())
                .unwrap_or(&status);
            return Err(RuntimeError::Failed(format!("{path}: {why}")));
        }
        Ok(value)
    }
}

fn provider_name(provider: &str) -> Result<String> {
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() || provider == crate::trust::OLLAMA {
        bail!("name the remote provider the server belongs to, other than \"ollama\"");
    }
    Ok(provider)
}

impl Backend for OpenAiCompatible {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn installed(&self) -> bool {
        true
    }

    /// Listed once per tick or evaluation.
    fn models(&self) -> Option<Vec<String>> {
        self.models
            .get_or_init(|| {
                let (status, list) = self.request("GET", "/models", None, LIST_TIMEOUT).ok()?;
                (status == "200").then_some(())?;
                Some(
                    list["data"]
                        .as_array()?
                        .iter()
                        .filter_map(|m| m["id"].as_str().map(str::to_string))
                        .collect(),
                )
            })
            .clone()
    }

    /// The provider serves it.
    fn stored_here(&self, model: &str) -> bool {
        self.models().is_some_and(|m| m.iter().any(|n| n == model))
    }

    fn generate(&self, model: &str, prompt: &str) -> Result<String, RuntimeError> {
        // Every prompt asks for one JSON object, as Ollama's JSON mode does for the local runtime.
        let body = json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "response_format": {"type": "json_object"},
            "stream": false,
        });
        let answer = self.call("/chat/completions", &body)?;
        answer["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| RuntimeError::Failed("no message in its answer".into()))
    }

    fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        #[cfg(feature = "eval")]
        if matches!(self.endpoint, Endpoint::Loopback(_)) {
            return self.eval_embed(model, texts);
        }
        let _ = (model, texts);
        Err(RuntimeError::Failed(
            "the generation provider does not handle embeddings".into(),
        ))
    }
}

#[cfg(feature = "eval")]
impl OpenAiCompatible {
    fn eval_embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let answer = self.call("/embeddings", &json!({"model": model, "input": texts}))?;
        let mut data: Vec<(u64, Vec<f32>)> = answer["data"]
            .as_array()
            .ok_or_else(|| RuntimeError::Failed("no data in its answer".into()))?
            .iter()
            .map(|d| {
                let v = serde_json::from_value(d["embedding"].clone())
                    .map_err(|e| RuntimeError::Failed(format!("unreadable embedding: {e}")))?;
                Ok((d["index"].as_u64().unwrap_or(0), v))
            })
            .collect::<Result<_, RuntimeError>>()?;
        data.sort_by_key(|(i, _)| *i);
        if data.len() != texts.len() {
            return Err(RuntimeError::Failed(format!(
                "{} embeddings for {} texts",
                data.len(),
                texts.len()
            )));
        }
        Ok(data.into_iter().map(|(_, v)| v).collect())
    }
}
