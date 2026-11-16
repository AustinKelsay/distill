//! Parked for the providers ticket: the check that tells a model whose weights are stored on
//! this machine from one Ollama serves from a cloud. Nothing calls it yet. Moved here unchanged
//! from the previous build's steward, with its tests.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where Ollama keeps its models: `$OLLAMA_MODELS`, else `~/.ollama/models`.
pub fn ollama_models_dir() -> PathBuf {
    std::env::var_os("OLLAMA_MODELS")
        .map(PathBuf::from)
        .or_else(|| {
            crate::home::user_home()
                .ok()
                .map(|h| h.join(".ollama/models"))
        })
        .unwrap_or_default()
}

/// Whether `model`'s weights are stored on this machine, read from Ollama's model store rather
/// than from what the CLI prints. A model named as a cloud source (a `cloud` or `…-cloud` tag)
/// is refused outright. Otherwise every manifest Ollama could resolve the name to (its older
/// `manifests/` store and its newer `manifests-v2/`, which it prefers) must name a weights layer
/// whose file is here at the recorded size and a config with no remote host or model, and at
/// least one must exist. Anything that can't be verified counts as not local.
///
/// This reads the store at `dir`. Ollama's server decides what actually runs; one started with
/// a different model directory than Distill sees is outside what this can check.
pub fn ollama_model_is_local(dir: &Path, model: &str) -> bool {
    let (name, tag) = match model.rsplit_once(':') {
        Some((n, t)) if !t.contains('/') => (n, t),
        _ => (model, "latest"),
    };
    if tag == "cloud" || tag.ends_with("-cloud") || name.ends_with("-cloud") {
        return false;
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
        || tag.is_empty()
        || tag.contains("..")
    {
        return false;
    }
    // Where each store keeps this name. A bare name is in the `library` namespace of the
    // default registry, and Ollama spells that registry two ways (`registry.ollama.ai`,
    // `ollama.com`), treating them as one; every spelling is checked in both stores.
    const DEFAULT_HOSTS: [&str; 2] = ["registry.ollama.ai", "ollama.com"];
    let (host, rest): (Option<&str>, Vec<&str>) = match parts.as_slice() {
        [m] => (None, vec!["library", m]),
        [ns, m] => (None, vec![ns, m]),
        [h, more @ ..] => (Some(*h), more.to_vec()),
        [] => return false,
    };
    let hosts: Vec<&str> = match host {
        Some(h) if !DEFAULT_HOSTS.contains(&h) => vec![h],
        _ => DEFAULT_HOSTS.to_vec(),
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    for store in ["manifests", "manifests-v2"] {
        for h in &hosts {
            let mut path = dir.join(store).join(h);
            for part in &rest {
                path = path.join(part);
            }
            candidates.push(path);
        }
    }
    let blob = |digest: &str| dir.join("blobs").join(digest.replace(':', "-"));
    let local = |manifest: &Value| {
        let weights = manifest["layers"].as_array().is_some_and(|layers| {
            layers.iter().any(|l| {
                l["mediaType"] == "application/vnd.ollama.image.model"
                    && l["digest"].as_str().is_some_and(|d| {
                        std::fs::metadata(blob(d))
                            .is_ok_and(|m| Some(m.len()) == l["size"].as_u64() && m.len() > 0)
                    })
            })
        });
        let remote = manifest["config"]["digest"]
            .as_str()
            .and_then(|d| std::fs::read(blob(d)).ok())
            .and_then(|c| serde_json::from_slice::<Value>(&c).ok())
            .is_none_or(|c| {
                ["remote_host", "remote_model"]
                    .iter()
                    .any(|k| c.get(*k).is_some_and(|v| !v.is_null() && v != ""))
            });
        weights && !remote
    };
    let mut found = false;
    for path in candidates.iter().map(|c| c.join(tag)) {
        if !path.exists() {
            continue;
        }
        found = true;
        let ok = std::fs::read(&path)
            .ok()
            .and_then(|m| serde_json::from_slice::<Value>(&m).ok())
            .is_some_and(|m| local(&m));
        if !ok {
            return false;
        }
    }
    found
}

pub fn parse_ollama_list(out: &str) -> Vec<String> {
    out.lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            let name = *cols.first()?;
            // NAME, ID, SIZE (number and unit), MODIFIED: a stored model has a size.
            let stored = cols
                .get(2)
                .is_some_and(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()));
            let cloud = name.ends_with("-cloud") || name.ends_with(":cloud");
            (stored && !cloud).then(|| name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_model_counts_as_local_only_when_its_weights_are_here() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        let put = |name: &str, tag: &str, config: &str, weights: Option<&[u8]>| {
            let mut layers = Vec::new();
            if let Some(w) = weights {
                let d = format!("sha256:{}", crate::util::sha256_hex(w));
                std::fs::create_dir_all(store.join("blobs")).unwrap();
                std::fs::write(store.join("blobs").join(d.replace(':', "-")), w).unwrap();
                layers.push(json!({"mediaType": "application/vnd.ollama.image.model", "digest": d, "size": w.len()}));
            }
            let c = format!("sha256:{}", crate::util::sha256_hex(config.as_bytes()));
            std::fs::create_dir_all(store.join("blobs")).unwrap();
            std::fs::write(store.join("blobs").join(c.replace(':', "-")), config).unwrap();
            let m = store
                .join("manifests/registry.ollama.ai/library")
                .join(name);
            std::fs::create_dir_all(&m).unwrap();
            std::fs::write(
                m.join(tag),
                json!({"config": {"digest": c}, "layers": layers}).to_string(),
            )
            .unwrap();
        };
        put("qwen-local", "latest", "{}", Some(b"weights"));
        // A cloud model copied to a plain name: no weights, a remote host in its config.
        put(
            "reviewer",
            "latest",
            r#"{"remote_host":"https://ollama.com","remote_model":"big"}"#,
            None,
        );
        put(
            "sneaky",
            "latest",
            r#"{"remote_host":"https://ollama.com"}"#,
            Some(b"w"),
        );
        assert!(ollama_model_is_local(store, "qwen-local"));
        assert!(ollama_model_is_local(store, "qwen-local:latest"));
        assert!(!ollama_model_is_local(store, "reviewer"));
        assert!(!ollama_model_is_local(store, "sneaky"));
        assert!(!ollama_model_is_local(store, "missing:7b"));
        // Names Ollama treats as cloud sources are refused whatever the store holds.
        put("qwen-local", "cloud", "{}", Some(b"weights"));
        put("qwen-local", "8b-cloud", "{}", Some(b"weights"));
        assert!(!ollama_model_is_local(store, "qwen-local:cloud"));
        assert!(!ollama_model_is_local(store, "qwen-local:8b-cloud"));
        // A cloud entry for the same name in the newer store, which Ollama prefers, wins.
        let v2 = store.join("manifests-v2/ollama.com/library/qwen-local");
        std::fs::create_dir_all(&v2).unwrap();
        let cloud_cfg = r#"{"remote_host":"https://ollama.com"}"#;
        let d = format!("sha256:{}", crate::util::sha256_hex(cloud_cfg.as_bytes()));
        std::fs::write(store.join("blobs").join(d.replace(':', "-")), cloud_cfg).unwrap();
        std::fs::write(
            v2.join("latest"),
            json!({"config": {"digest": d}, "layers": []}).to_string(),
        )
        .unwrap();
        assert!(!ollama_model_is_local(store, "qwen-local"));
        // The same holds when the name spells out the registry Ollama treats as `ollama.com`.
        assert!(!ollama_model_is_local(
            store,
            "registry.ollama.ai/library/qwen-local"
        ));
        assert!(!ollama_model_is_local(
            store,
            "ollama.com/library/qwen-local"
        ));
        assert!(!ollama_model_is_local(store, "../../etc/passwd"));
    }

    #[test]
    fn only_models_stored_on_this_machine_are_offered() {
        let list = "NAME                  ID              SIZE      MODIFIED\n\
                    qwen3.6:35b-a3b       0a1b2c3d4e5f    22 GB     2 days ago\n\
                    gpt-oss:120b-cloud    1b2c3d4e5f6a    -         3 days ago\n\
                    kimi-k3:cloud         2c3d4e5f6a7b    -         1 week ago\n";
        assert_eq!(parse_ollama_list(list), ["qwen3.6:35b-a3b"]);
    }
}
