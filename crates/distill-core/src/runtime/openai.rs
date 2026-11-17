//! A remote provider's OpenAI-compatible server reached on this machine, such as a provider's local
//! proxy, standing in for the local runtime while the model steps are evaluated (`distill-eval`).
//! Built only with the `eval` feature.

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{Backend, RuntimeError};

const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct OpenAiCompatible {
    provider: String,
    addr: SocketAddr,
    models: OnceLock<Option<Vec<String>>>,
}

impl OpenAiCompatible {
    pub(crate) fn new(provider: &str, addr: SocketAddr) -> Result<Self> {
        let provider = provider.trim().to_ascii_lowercase();
        if provider.is_empty() || provider == crate::trust::OLLAMA {
            bail!("name the remote provider the server belongs to, other than \"ollama\"");
        }
        if !addr.ip().is_loopback() {
            bail!("{addr} is not on this machine");
        }
        Ok(Self {
            provider,
            addr,
            models: OnceLock::new(),
        })
    }

    fn call(&self, path: &str, body: &Value) -> Result<Value, RuntimeError> {
        let (status, value) =
            super::http::request(&self.addr, "POST", path, Some(body), CALL_TIMEOUT)?;
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

impl Backend for OpenAiCompatible {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn installed(&self) -> bool {
        true
    }

    /// Listed once: the server's list does not change while an evaluation runs.
    fn models(&self) -> Option<Vec<String>> {
        self.models
            .get_or_init(|| {
                let (status, list) =
                    super::http::request(&self.addr, "GET", "/v1/models", None, LIST_TIMEOUT)
                        .ok()?;
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
        let answer = self.call("/v1/chat/completions", &body)?;
        answer["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| RuntimeError::Failed("no message in its answer".into()))
    }

    fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let answer = self.call("/v1/embeddings", &json!({"model": model, "input": texts}))?;
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
