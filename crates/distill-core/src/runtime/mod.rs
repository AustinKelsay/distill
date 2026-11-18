//! Ollama on this machine, or an explicitly configured remote generation provider. Model steps
//! use the runtime's provider identity when reading sessions through the trust-checked read path.

mod fake;
mod http;
mod ollama;
mod openai;

pub use fake::{Call, FakeRuntime};

use std::sync::Arc;

use serde::Serialize;

/// The one client for the local runtime.
#[derive(Clone)]
pub struct LocalRuntime {
    backend: Arc<dyn Backend>,
}

/// What the runtime on this machine is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "models", rename_all = "snake_case")]
pub enum RuntimeState {
    NotInstalled,
    /// Installed, but nothing answers on this machine.
    NotRunning,
    /// Answering, with the models it has installed.
    Running(Vec<Model>),
}

/// A model the runtime has installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Model {
    pub name: String,
    pub stored: Stored,
}

/// Where a model runs: on this machine, or on a cloud the runtime forwards to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stored {
    Here,
    Cloud,
}

/// How the client reaches a runtime: the real one or the fake.
pub(crate) trait Backend: Send + Sync {
    /// The provider its models run at, which readers name (`ollama/<model>`).
    fn provider(&self) -> &str {
        crate::trust::OLLAMA
    }
    fn installed(&self) -> bool;
    /// The installed models' names, or `None` when nothing answers.
    fn models(&self) -> Option<Vec<String>>;
    fn model_list_error(&self) -> Option<&RuntimeError> {
        None
    }
    /// Whether `model`'s weights are on this machine. Needs no running runtime.
    fn stored_here(&self, model: &str) -> bool;
    fn generate(&self, model: &str, prompt: &str) -> Result<String, RuntimeError>;
    fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError>;
}

/// Why a call to the local runtime gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// The model is served from a cloud, or isn't installed: either way not on this machine.
    NotStoredHere(String),
    /// Nothing answers on this machine.
    NotRunning,
    /// The configured remote provider could not be reached.
    ProviderUnavailable(String),
    /// The runtime answered with an error, or with something Distill can't read.
    Failed(String),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStoredHere(m) => write!(
                f,
                "\"{m}\" is not stored on this machine (a cloud-served or missing model); \
                 Distill only calls models whose weights are here"
            ),
            Self::NotRunning => write!(
                f,
                "the local runtime is not running (start the Ollama app or `ollama serve`)"
            ),
            Self::ProviderUnavailable(e) => write!(f, "could not reach generation provider {e}"),
            Self::Failed(e) => write!(f, "the model runtime failed: {e}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl LocalRuntime {
    pub(crate) fn new(backend: impl Backend + 'static) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }

    /// The Ollama server on this machine. Nothing is contacted until a call needs it.
    pub fn ollama() -> Self {
        Self::new(ollama::Ollama::on_this_machine())
    }

    /// The OpenAI-compatible server of the remote provider `provider` at `addr`, which must be on
    /// this machine (a provider's local proxy). Its models count as stored here, so the model
    /// steps run with them, and they read as `<provider>/<model>`: the home must trust
    /// `provider`, or the read path refuses them. For evaluating the model steps only.
    #[cfg(feature = "eval")]
    pub fn openai_compatible(provider: &str, addr: std::net::SocketAddr) -> anyhow::Result<Self> {
        Ok(Self::new(openai::OpenAiCompatible::new(provider, addr)?))
    }

    /// An HTTPS provider for the scheduled generation steps.
    pub(crate) fn remote_generation(
        config: &crate::config::GenerationConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self::new(openai::OpenAiCompatible::remote(config)?))
    }

    /// The provider the runtime's models run at.
    pub fn provider(&self) -> &str {
        self.backend.provider()
    }

    pub fn state(&self) -> RuntimeState {
        match self.backend.models() {
            Some(names) => RuntimeState::Running(
                names
                    .into_iter()
                    .map(|name| Model {
                        stored: if self.stored_here(&name) {
                            Stored::Here
                        } else {
                            Stored::Cloud
                        },
                        name,
                    })
                    .collect(),
            ),
            None if self.backend.installed() => RuntimeState::NotRunning,
            None => RuntimeState::NotInstalled,
        }
    }

    pub(crate) fn model_list_error(&self) -> Option<&RuntimeError> {
        self.backend.model_list_error()
    }

    /// Whether `model` is stored on this machine. A model that can't be verified, including one
    /// that isn't installed, counts as served from a cloud.
    pub fn stored_here(&self, model: &str) -> bool {
        self.backend.stored_here(model)
    }

    /// Generate text from `prompt` with `model`, which must be stored on this machine.
    pub fn generate(&self, model: &str, prompt: &str) -> Result<String, RuntimeError> {
        self.refuse_unless_here(model)?;
        self.backend.generate(model, prompt)
    }

    /// One vector per text, embedded by `model`, which must be stored on this machine.
    pub fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        self.refuse_unless_here(model)?;
        self.backend.embed(model, texts)
    }

    fn refuse_unless_here(&self, model: &str) -> Result<(), RuntimeError> {
        if self.stored_here(model) {
            Ok(())
        } else {
            Err(RuntimeError::NotStoredHere(model.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_runtime_lists_each_model_as_stored_here_or_served_from_a_cloud() {
        let fake = FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .with_model("big:cloud", Stored::Cloud);
        assert_eq!(
            fake.runtime().state(),
            RuntimeState::Running(vec![
                Model {
                    name: "small:8b".into(),
                    stored: Stored::Here
                },
                Model {
                    name: "big:cloud".into(),
                    stored: Stored::Cloud
                },
            ])
        );
        assert_eq!(
            FakeRuntime::stopped().runtime().state(),
            RuntimeState::NotRunning
        );
        assert_eq!(
            FakeRuntime::not_installed().runtime().state(),
            RuntimeState::NotInstalled
        );
    }

    #[test]
    fn the_client_generates_and_embeds_with_a_model_stored_here() {
        let fake = FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .replying(|model, prompt| format!("{model} read {} bytes", prompt.len()));
        let rt = fake.runtime();
        assert_eq!(
            rt.generate("small:8b", "hello").unwrap(),
            "small:8b read 5 bytes"
        );
        let v = rt.embed("small:8b", &["one", "two", "one"]).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], v[2], "the same text embeds the same");
        assert_ne!(v[0], v[1]);
        assert_eq!(
            fake.calls(),
            [
                Call::Generate {
                    model: "small:8b".into(),
                    prompt: "hello".into()
                },
                Call::Embed {
                    model: "small:8b".into(),
                    texts: vec!["one".into(), "two".into(), "one".into()]
                },
            ]
        );
    }

    #[test]
    fn the_client_refuses_a_model_served_from_a_cloud_before_calling_it() {
        let fake = FakeRuntime::running().with_model("big:cloud", Stored::Cloud);
        let rt = fake.runtime();
        for err in [
            rt.generate("big:cloud", "hello").unwrap_err(),
            rt.embed("big:cloud", &["hello"]).unwrap_err(),
            rt.generate("never-installed", "hello").unwrap_err(),
        ] {
            assert!(matches!(err, RuntimeError::NotStoredHere(_)), "{err}");
            assert!(
                err.to_string().contains("not stored on this machine"),
                "{err}"
            );
        }
        assert_eq!(fake.calls(), [], "a refused model is never called");
    }

    #[test]
    fn a_stopped_runtime_is_reported_as_not_running() {
        let fake = FakeRuntime::stopped().with_model("small:8b", Stored::Here);
        let rt = fake.runtime();
        assert!(rt.stored_here("small:8b"), "its models are still on disk");
        assert!(matches!(
            rt.generate("small:8b", "hello"),
            Err(RuntimeError::NotRunning)
        ));
        assert!(matches!(
            FakeRuntime::not_installed()
                .runtime()
                .embed("small:8b", &["x"]),
            Err(RuntimeError::NotStoredHere(_))
        ));
    }
}
