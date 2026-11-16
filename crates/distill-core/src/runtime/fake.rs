//! A fake local runtime for tests: declared models, canned replies, and a record of every call.
//! No test contacts a real runtime.

use std::sync::{Arc, Mutex};

use super::{Backend, LocalRuntime, RuntimeError, Stored};

/// A fake of the local runtime. Build one, hand [`FakeRuntime::runtime`] to the code under test,
/// then read [`FakeRuntime::calls`].
#[derive(Clone)]
pub struct FakeRuntime {
    state: Arc<Mutex<State>>,
}

/// A call the client passed on to the fake.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Generate { model: String, prompt: String },
    Embed { model: String, texts: Vec<String> },
}

type Reply = Arc<dyn Fn(&str, &str) -> String + Send + Sync>;

struct State {
    installed: bool,
    running: bool,
    models: Vec<(String, Stored)>,
    reply: Reply,
    calls: Vec<Call>,
}

impl FakeRuntime {
    fn with_state(installed: bool, running: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                installed,
                running,
                models: Vec::new(),
                reply: Arc::new(|_, _| "fake reply".to_string()),
                calls: Vec::new(),
            })),
        }
    }

    /// Installed and answering.
    pub fn running() -> Self {
        Self::with_state(true, true)
    }

    /// Installed, but not answering. Its models are still on disk.
    pub fn stopped() -> Self {
        Self::with_state(true, false)
    }

    pub fn not_installed() -> Self {
        Self::with_state(false, false)
    }

    /// Install `name`, stored on this machine or served from a cloud.
    pub fn with_model(self, name: &str, stored: Stored) -> Self {
        self.lock().models.push((name.to_string(), stored));
        self
    }

    /// Answer every generate call with `reply(model, prompt)`. By default it answers
    /// `fake reply`.
    pub fn replying(self, reply: impl Fn(&str, &str) -> String + Send + Sync + 'static) -> Self {
        self.lock().reply = Arc::new(reply);
        self
    }

    /// The client for this fake.
    pub fn runtime(&self) -> LocalRuntime {
        LocalRuntime::new(self.clone())
    }

    /// Every call that reached the fake, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Backend for FakeRuntime {
    fn installed(&self) -> bool {
        self.lock().installed
    }

    fn models(&self) -> Option<Vec<String>> {
        let s = self.lock();
        s.running
            .then(|| s.models.iter().map(|(n, _)| n.clone()).collect())
    }

    fn stored_here(&self, model: &str) -> bool {
        self.lock()
            .models
            .iter()
            .any(|(n, s)| n == model && *s == Stored::Here)
    }

    fn generate(&self, model: &str, prompt: &str) -> Result<String, RuntimeError> {
        let mut s = self.lock();
        if !s.running {
            return Err(RuntimeError::NotRunning);
        }
        s.calls.push(Call::Generate {
            model: model.to_string(),
            prompt: prompt.to_string(),
        });
        let reply = s.reply.clone();
        drop(s);
        Ok(reply(model, prompt))
    }

    /// A vector made from the text's hash: the same text always embeds the same.
    fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let mut s = self.lock();
        if !s.running {
            return Err(RuntimeError::NotRunning);
        }
        s.calls.push(Call::Embed {
            model: model.to_string(),
            texts: texts.iter().map(|t| t.to_string()).collect(),
        });
        Ok(texts
            .iter()
            .map(|t| {
                use sha2::{Digest, Sha256};
                Sha256::digest(t.as_bytes())[..8]
                    .iter()
                    .map(|b| f32::from(*b) / 255.0)
                    .collect()
            })
            .collect())
    }
}
