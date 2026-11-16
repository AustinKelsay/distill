//! Trust: the list of providers, which are local, which are trusted, and what a given reader may
//! read (issue #72, "Trust"; ADR 0003).

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, ProviderConfig, ProviderKind};
use crate::runtime::{LocalRuntime, RuntimeState};

/// The local runtime, built in and always trusted. In this version the only local provider.
pub const OLLAMA: &str = "ollama";
/// Where a reader naming `ollama` with a model Ollama serves from a cloud is placed: a remote
/// provider, untrusted until the owner marks it.
pub const OLLAMA_CLOUD: &str = "ollama-cloud";

/// Provider names match whatever their case: `Ollama` is `ollama`.
fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Whoever asks Distill for session text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reader", rename_all = "snake_case")]
pub enum Reader {
    /// The owner, reading in the GUI.
    Owner,
    /// A model at a named provider, with its model when it knows it. Distill cannot verify
    /// either; a reader that names no provider is refused.
    Model {
        provider: Option<String>,
        model: Option<String>,
    },
    /// Distill itself, for steps that use no model.
    Distill,
}

impl Reader {
    /// A model reader. A blank provider or model counts as not named.
    pub fn model(provider: Option<&str>, model: Option<&str>) -> Self {
        let named = |s: Option<&str>| {
            s.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Self::Model {
            provider: named(provider),
            model: named(model),
        }
    }
}

/// What a reader may do: read, or nothing, with a reason the owner can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "access", content = "reason", rename_all = "snake_case")]
pub enum Access {
    Allowed,
    Refused(String),
}

/// A provider as Distill sees it: built in, or listed in the config's `[[providers]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provider {
    pub name: String,
    pub kind: ProviderKind,
    pub trusted: bool,
}

/// What `distill providers` shows: every provider, the local runtime's state and models under
/// `ollama`, and the config entries that were ignored.
#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub providers: Vec<ListedProvider>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListedProvider {
    #[serde(flatten)]
    pub provider: Provider,
    /// For the local provider: whether its runtime is installed and running, and its models.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeState>,
}

/// The providers Distill knows, built from the config.
#[derive(Debug, Clone)]
pub struct Trust {
    providers: Vec<Provider>,
    warnings: Vec<String>,
}

impl Trust {
    pub fn new(config: &Config) -> Self {
        let mut providers = vec![
            Provider {
                name: OLLAMA.into(),
                kind: ProviderKind::Local,
                trusted: true,
            },
            Provider {
                name: OLLAMA_CLOUD.into(),
                kind: ProviderKind::Remote,
                trusted: false,
            },
        ];
        let mut warnings = Vec::new();
        let mut seen = Vec::new();
        for p in &config.providers {
            let name = normalize(&p.name);
            if name == OLLAMA {
                if p.kind != ProviderKind::Local {
                    warnings.push(format!(
                        "the [[providers]] entry \"{OLLAMA}\" is ignored: it is built in as local"
                    ));
                }
                continue;
            }
            if p.kind == ProviderKind::Local {
                warnings.push(format!(
                    "the [[providers]] entry \"{}\" is ignored: the only local provider is \"{OLLAMA}\"",
                    p.name
                ));
                continue;
            }
            // Listed twice: the first entry counts.
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            match providers.iter_mut().find(|q| q.name == name) {
                Some(builtin) => builtin.trusted = p.trusted,
                None => providers.push(Provider {
                    name,
                    kind: ProviderKind::Remote,
                    trusted: p.trusted,
                }),
            }
        }
        Self {
            providers,
            warnings,
        }
    }

    /// Config entries that were ignored, and why.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub fn provider(&self, name: &str) -> Option<&Provider> {
        let name = normalize(name);
        self.providers.iter().find(|p| p.name == name)
    }

    /// Every provider, asking `runtime` what it is doing for the local one.
    pub fn listing(&self, runtime: &LocalRuntime) -> Listing {
        Listing {
            providers: self
                .providers
                .iter()
                .map(|p| ListedProvider {
                    provider: p.clone(),
                    runtime: (p.name == OLLAMA).then(|| runtime.state()),
                })
                .collect(),
            warnings: self.warnings.clone(),
        }
    }

    /// Whether `reader` may read session text. `runtime` tells a model stored on this machine
    /// from one Ollama serves from a cloud; it need not be running.
    pub fn check(&self, reader: &Reader, runtime: &LocalRuntime) -> Access {
        let (provider, model) = match reader {
            Reader::Owner | Reader::Distill => return Access::Allowed,
            Reader::Model { provider, model } => (
                provider.as_deref().map(normalize).filter(|p| !p.is_empty()),
                model.as_deref().map(str::trim).filter(|m| !m.is_empty()),
            ),
        };
        let Some(provider) = provider else {
            return Access::Refused(
                "this reader names no provider. Name the provider the model runs at with \
                 --provider or DISTILL_PROVIDER; `distill providers` lists the trusted ones."
                    .into(),
            );
        };
        let mut via = String::new();
        let provider = match model {
            Some(m) if provider == OLLAMA && !runtime.stored_here(m) => {
                via = format!(
                    "\"{m}\" is not stored on this machine, so Ollama serves it from a cloud: \
                     it reads as provider \"{OLLAMA_CLOUD}\". "
                );
                OLLAMA_CLOUD
            }
            _ => &provider,
        };
        match self.provider(provider) {
            Some(p) if p.trusted => Access::Allowed,
            Some(_) => Access::Refused(format!(
                "{via}Provider \"{provider}\" is not trusted. The owner can trust it with \
                 `distill providers trust {provider}`."
            )),
            None => Access::Refused(format!(
                "{via}Provider \"{provider}\" is not listed. The owner can add it as a trusted \
                 remote provider with `distill providers add {provider} --trusted`."
            )),
        }
    }
}

/// Add a remote provider to the config's `[[providers]]`, trusted or not.
pub fn add(config: &mut Config, name: &str, trusted: bool) -> Result<()> {
    let name = &normalize(name);
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        bail!("{name:?} is not a provider name: use letters, digits, '-', '_' and '.'");
    }
    if name == OLLAMA {
        bail!("\"{OLLAMA}\" is built in as the local provider and is always trusted");
    }
    if Trust::new(config).provider(name).is_some() {
        bail!(
            "\"{name}\" is already listed; mark it with `distill providers trust {name}` or \
             `distill providers untrust {name}`"
        );
    }
    config.providers.push(ProviderConfig {
        name: name.to_string(),
        kind: ProviderKind::Remote,
        trusted,
    });
    Ok(())
}

/// Mark a listed remote provider trusted, or unmark it. Its models may read, or may not, from
/// the next check on.
pub fn set_trusted(config: &mut Config, name: &str, trusted: bool) -> Result<()> {
    let name = &normalize(name);
    if name == OLLAMA {
        if trusted {
            return Ok(());
        }
        bail!("\"{OLLAMA}\" runs on this machine and is always trusted");
    }
    let trust = Trust::new(config);
    if trust.provider(name).is_none() {
        bail!(
            "\"{name}\" is not listed; add it with `distill providers add {name}`{}",
            if trusted { " --trusted" } else { "" }
        );
    }
    match config
        .providers
        .iter_mut()
        .find(|p| normalize(&p.name) == *name && p.kind == ProviderKind::Remote)
    {
        Some(p) => p.trusted = trusted,
        // Built in and not yet in the file (`ollama-cloud`).
        None => config.providers.push(ProviderConfig {
            name: name.to_string(),
            kind: ProviderKind::Remote,
            trusted,
        }),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{FakeRuntime, Model, RuntimeState, Stored};

    fn provider(name: &str, kind: ProviderKind, trusted: bool) -> Provider {
        Provider {
            name: name.into(),
            kind,
            trusted,
        }
    }

    #[test]
    fn ollama_is_built_in_and_trusted_and_its_cloud_is_not() {
        let t = Trust::new(&Config::default());
        assert_eq!(
            t.providers(),
            [
                provider("ollama", ProviderKind::Local, true),
                provider("ollama-cloud", ProviderKind::Remote, false),
            ]
        );
    }

    fn with_providers(providers: Vec<ProviderConfig>) -> Config {
        Config {
            providers,
            ..Config::default()
        }
    }

    fn listed(name: &str, kind: ProviderKind, trusted: bool) -> ProviderConfig {
        ProviderConfig {
            name: name.into(),
            kind,
            trusted,
        }
    }

    #[test]
    fn remote_providers_come_from_the_config_and_other_local_ones_are_ignored() {
        let c = with_providers(vec![
            listed("example-cloud", ProviderKind::Remote, true),
            listed("other-cloud", ProviderKind::Remote, false),
            listed("llama-box", ProviderKind::Local, true),
            listed("ollama-cloud", ProviderKind::Remote, true),
        ]);
        let t = Trust::new(&c);
        assert_eq!(
            t.providers(),
            [
                provider("ollama", ProviderKind::Local, true),
                provider("ollama-cloud", ProviderKind::Remote, true),
                provider("example-cloud", ProviderKind::Remote, true),
                provider("other-cloud", ProviderKind::Remote, false),
            ]
        );
        assert_eq!(t.warnings().len(), 1, "{:?}", t.warnings());
        assert!(t.warnings()[0].contains("llama-box"), "{:?}", t.warnings());
    }

    fn model(provider: Option<&str>, model: Option<&str>) -> Reader {
        Reader::model(provider, model)
    }

    fn refused(access: Access) -> String {
        match access {
            Access::Refused(why) => why,
            Access::Allowed => panic!("allowed"),
        }
    }

    #[test]
    fn the_owner_and_distill_itself_may_read() {
        let t = Trust::new(&Config::default());
        let rt = FakeRuntime::not_installed().runtime();
        assert_eq!(t.check(&Reader::Owner, &rt), Access::Allowed);
        assert_eq!(t.check(&Reader::Distill, &rt), Access::Allowed);
    }

    #[test]
    fn a_model_at_a_trusted_remote_provider_may_read_and_others_are_refused() {
        let c = with_providers(vec![
            listed("example-cloud", ProviderKind::Remote, true),
            listed("other-cloud", ProviderKind::Remote, false),
        ]);
        let t = Trust::new(&c);
        let rt = FakeRuntime::not_installed().runtime();
        assert_eq!(
            t.check(&model(Some("example-cloud"), Some("any")), &rt),
            Access::Allowed
        );
        assert_eq!(
            t.check(&model(Some("example-cloud"), None), &rt),
            Access::Allowed
        );

        let why = refused(t.check(&model(Some("other-cloud"), None), &rt));
        assert!(why.contains("distill providers trust other-cloud"), "{why}");

        let why = refused(t.check(&model(Some("unheard-of"), None), &rt));
        assert!(why.contains("not listed"), "{why}");
        assert!(
            why.contains("distill providers add unheard-of --trusted"),
            "{why}"
        );

        for nobody in [model(None, Some("any")), model(Some("  "), None)] {
            let why = refused(t.check(&nobody, &rt));
            assert!(
                why.contains("--provider") && why.contains("DISTILL_PROVIDER"),
                "{why}"
            );
        }
    }

    #[test]
    fn a_model_ollama_serves_from_a_cloud_reads_as_ollama_cloud() {
        let rt = FakeRuntime::stopped()
            .with_model("small:8b", Stored::Here)
            .with_model("big:cloud", Stored::Cloud)
            .runtime();
        let mut c = Config::default();
        let t = Trust::new(&c);
        // Trusted whether or not the runtime is running.
        assert_eq!(t.check(&model(Some("ollama"), None), &rt), Access::Allowed);
        assert_eq!(
            t.check(&model(Some("ollama"), Some("small:8b")), &rt),
            Access::Allowed
        );
        for cloud in ["big:cloud", "never-installed"] {
            let why = refused(t.check(&model(Some("ollama"), Some(cloud)), &rt));
            assert!(why.contains("ollama-cloud"), "{why}");
            assert!(
                why.contains("distill providers trust ollama-cloud"),
                "{why}"
            );
        }
        assert!(matches!(
            t.check(&model(Some("ollama-cloud"), None), &rt),
            Access::Refused(_)
        ));

        c.providers
            .push(listed("ollama-cloud", ProviderKind::Remote, true));
        let t = Trust::new(&c);
        assert_eq!(
            t.check(&model(Some("ollama"), Some("big:cloud")), &rt),
            Access::Allowed
        );
    }

    #[test]
    fn the_owner_adds_a_remote_provider_then_trusts_and_unmarks_it() {
        let mut c = Config::default();
        add(&mut c, "example-cloud", false).unwrap();
        let rt = FakeRuntime::not_installed().runtime();
        let reader = model(Some("example-cloud"), None);
        assert!(matches!(
            Trust::new(&c).check(&reader, &rt),
            Access::Refused(_)
        ));

        set_trusted(&mut c, "example-cloud", true).unwrap();
        assert_eq!(Trust::new(&c).check(&reader, &rt), Access::Allowed);
        assert_eq!(
            c.providers,
            [listed("example-cloud", ProviderKind::Remote, true)]
        );

        set_trusted(&mut c, "example-cloud", false).unwrap();
        assert!(matches!(
            Trust::new(&c).check(&reader, &rt),
            Access::Refused(_)
        ));

        // ollama-cloud is built in: it is marked without being added.
        set_trusted(&mut c, "ollama-cloud", true).unwrap();
        assert!(Trust::new(&c).provider("ollama-cloud").unwrap().trusted);
        add(&mut c, "trusted-now", true).unwrap();
        assert!(Trust::new(&c).provider("trusted-now").unwrap().trusted);
    }

    #[test]
    fn what_cannot_be_added_or_marked_says_why() {
        let mut c = Config::default();
        add(&mut c, "example-cloud", false).unwrap();
        for (result, says) in [
            (add(&mut c, "example-cloud", true), "already listed"),
            (add(&mut c, "ollama", true), "built in"),
            (add(&mut c, "ollama-cloud", true), "already listed"),
            (add(&mut c, "", true), "name"),
            (add(&mut c, "two words", true), "name"),
            (
                set_trusted(&mut c, "unheard-of", true),
                "distill providers add unheard-of",
            ),
            (set_trusted(&mut c, "ollama", false), "always trusted"),
        ] {
            let err = result.unwrap_err().to_string();
            assert!(err.contains(says), "{err:?} should say {says:?}");
        }
        assert_eq!(c.providers.len(), 1);
    }

    #[test]
    fn the_listing_shows_the_local_runtimes_models_under_ollama() {
        let c = with_providers(vec![
            listed("example-cloud", ProviderKind::Remote, true),
            listed("llama-box", ProviderKind::Local, true),
        ]);
        let fake = FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .with_model("big:cloud", Stored::Cloud);
        let l = Trust::new(&c).listing(&fake.runtime());
        let names: Vec<_> = l
            .providers
            .iter()
            .map(|p| p.provider.name.as_str())
            .collect();
        assert_eq!(names, ["ollama", "ollama-cloud", "example-cloud"]);
        assert_eq!(
            l.providers[0].runtime,
            Some(RuntimeState::Running(vec![
                Model {
                    name: "small:8b".into(),
                    stored: Stored::Here
                },
                Model {
                    name: "big:cloud".into(),
                    stored: Stored::Cloud
                },
            ]))
        );
        assert!(l.providers[1..].iter().all(|p| p.runtime.is_none()));
        assert_eq!(l.warnings.len(), 1);

        let l = Trust::new(&c).listing(&FakeRuntime::not_installed().runtime());
        assert_eq!(l.providers[0].runtime, Some(RuntimeState::NotInstalled));
        assert!(
            l.providers[0].provider.trusted,
            "trusted with no runtime installed"
        );
    }

    #[test]
    fn provider_names_match_whatever_their_case() {
        let mut c = with_providers(vec![listed("Example-Cloud", ProviderKind::Remote, true)]);
        let rt = FakeRuntime::stopped()
            .with_model("big:cloud", Stored::Cloud)
            .runtime();
        let t = Trust::new(&c);
        assert_eq!(
            t.check(&model(Some("example-cloud"), None), &rt),
            Access::Allowed
        );
        assert_eq!(t.check(&model(Some("OLLAMA"), None), &rt), Access::Allowed);
        assert!(matches!(
            t.check(&model(Some("Ollama"), Some("big:cloud")), &rt),
            Access::Refused(_)
        ));
        let blank_model = Reader::Model {
            provider: Some("ollama".into()),
            model: Some(" ".into()),
        };
        assert_eq!(t.check(&blank_model, &rt), Access::Allowed);
        let err = add(&mut c, "Ollama", true).unwrap_err().to_string();
        assert!(err.contains("built in"), "{err}");
        set_trusted(&mut c, "EXAMPLE-CLOUD", false).unwrap();
        assert!(!Trust::new(&c).provider("example-cloud").unwrap().trusted);
    }
}
