//! The local runtime in this version: the Ollama server on this machine, reached over loopback
//! only. Which of its models are stored here is read from its model store on disk, not from what
//! the server says.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Backend, RuntimeError};

/// Ollama's own default address.
const DEFAULT_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 11434);

/// The Ollama server on this machine.
pub(crate) struct Ollama {
    addr: SocketAddr,
    models_dir: PathBuf,
}

impl Ollama {
    /// At `OLLAMA_HOST` when that is an address on this machine, else Ollama's default; with
    /// the models in `OLLAMA_MODELS`, else `~/.ollama/models`.
    pub(crate) fn on_this_machine() -> Self {
        Self {
            addr: loopback_addr(std::env::var("OLLAMA_HOST").ok().as_deref()),
            models_dir: models_dir(),
        }
    }
}

/// How long a whole request may take, answer included: a list is quick, a generation on a laptop
/// is not. The tick's time budget (`crate::tick`) relies on a call never outlasting this.
const LIST_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);

impl Backend for Ollama {
    /// Its CLI is on `PATH`, or it has a model store.
    fn installed(&self) -> bool {
        self.models_dir.is_dir()
            || std::env::var_os("PATH").is_some_and(|path| {
                std::env::split_paths(&path).any(|dir| dir.join("ollama").is_file())
            })
    }

    fn models(&self) -> Option<Vec<String>> {
        let tags = self.request("GET", "/api/tags", None, LIST_TIMEOUT).ok()?;
        Some(
            tags["models"]
                .as_array()?
                .iter()
                .filter_map(|m| m["name"].as_str().map(str::to_string))
                .collect(),
        )
    }

    fn stored_here(&self, model: &str) -> bool {
        model_is_local(&self.models_dir, model)
    }

    fn generate(&self, model: &str, prompt: &str) -> Result<String, RuntimeError> {
        // Every prompt asks for one JSON object; Ollama's JSON mode keeps prose out of the reply.
        let body = json!({"model": model, "prompt": prompt, "stream": false, "format": "json"});
        let answer = self.request("POST", "/api/generate", Some(&body), CALL_TIMEOUT)?;
        answer["response"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| RuntimeError::Failed("no response in its answer".into()))
    }

    fn embed(&self, model: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, RuntimeError> {
        let body = json!({"model": model, "input": texts});
        let answer = self.request("POST", "/api/embed", Some(&body), CALL_TIMEOUT)?;
        let vectors: Vec<Vec<f32>> = serde_json::from_value(answer["embeddings"].clone())
            .map_err(|e| RuntimeError::Failed(format!("unreadable embeddings: {e}")))?;
        if vectors.len() != texts.len() {
            return Err(RuntimeError::Failed(format!(
                "{} embeddings for {} texts",
                vectors.len(),
                texts.len()
            )));
        }
        Ok(vectors)
    }
}

impl Ollama {
    /// One HTTP/1.0 request, so the answer is the rest of the stream with no chunking.
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value, RuntimeError> {
        let until = Instant::now() + timeout;
        let mut stream = TcpStream::connect_timeout(&self.addr, Duration::from_secs(2))
            .map_err(|_| RuntimeError::NotRunning)?;
        let failed = |e: std::io::Error| RuntimeError::Failed(e.to_string());
        stream.set_write_timeout(Some(timeout)).map_err(failed)?;
        let body = body.map(Value::to_string).unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            self.addr,
            body.len()
        )
        .map_err(failed)?;
        // Read against the whole request's deadline, so an answer that trickles in can't outlast it.
        let mut answer = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        let late = || {
            RuntimeError::Failed(format!(
                "{path}: no whole answer within {}s",
                timeout.as_secs()
            ))
        };
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(late());
            }
            stream.set_read_timeout(Some(left)).map_err(failed)?;
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => answer.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                // A read timeout: the deadline came with no more of the answer.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(late());
                }
                Err(e) => return Err(failed(e)),
            }
        }
        let text = String::from_utf8_lossy(&answer);
        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or_else(|| RuntimeError::Failed("an answer with no body".into()))?;
        let status = head.split_whitespace().nth(1).unwrap_or("");
        let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if status != "200" {
            let why = value["error"].as_str().unwrap_or(status);
            return Err(RuntimeError::Failed(format!("{path}: {why}")));
        }
        if value.is_null() {
            return Err(RuntimeError::Failed(format!(
                "{path}: an answer that is not JSON"
            )));
        }
        Ok(value)
    }
}

/// Where to reach Ollama: `host` (Ollama's `OLLAMA_HOST`) only when it names this machine, so
/// session text never leaves it. `0.0.0.0` is how a server is told to listen everywhere; a client
/// reaches it at `127.0.0.1`. Anything else falls back to Ollama's default.
fn loopback_addr(host: Option<&str>) -> SocketAddr {
    let default = SocketAddr::from(DEFAULT_ADDR);
    let Some(host) = host.map(str::trim).filter(|h| !h.is_empty()) else {
        return default;
    };
    let host = host
        .strip_prefix("http://")
        .unwrap_or(host)
        .trim_end_matches('/');
    let (name, port) = match host.rsplit_once(':') {
        Some((n, p)) if !n.ends_with(':') => match p.parse::<u16>() {
            Ok(p) => (n, p),
            Err(_) => return default,
        },
        _ => (host, DEFAULT_ADDR.1),
    };
    let name = name.trim_start_matches('[').trim_end_matches(']');
    let ip = match name {
        "" | "localhost" => DEFAULT_ADDR.0.into(),
        n => match n.parse::<std::net::IpAddr>() {
            Ok(ip) if ip.is_unspecified() => DEFAULT_ADDR.0.into(),
            Ok(ip) if ip.is_loopback() => ip,
            _ => return default,
        },
    };
    SocketAddr::new(ip, port)
}

/// Where Ollama keeps its models: `$OLLAMA_MODELS`, else `~/.ollama/models`.
fn models_dir() -> PathBuf {
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
fn model_is_local(dir: &Path, model: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{LocalRuntime, Model, RuntimeState, Stored};
    use serde_json::json;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A canned stand-in for the Ollama server on a loopback port: answers each request by its
    /// path and records what it was sent.
    fn canned(
        answers: &'static [(&'static str, &'static str)],
    ) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let mut conn = conn.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Headers, then a body of Content-Length bytes.
                let request = loop {
                    let n = conn.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .map_or(0, |n| n.parse().unwrap());
                        if body.len() >= len || n == 0 {
                            break text;
                        }
                    }
                };
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(request);
                let body = answers
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map_or("{}", |(_, b)| b);
                write!(
                    conn,
                    "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{body}"
                )
                .unwrap();
            }
        });
        (addr, seen)
    }

    fn store_with(dir: &Path, name: &str) {
        let weights = b"weights";
        let d = format!("sha256:{}", crate::util::sha256_hex(weights));
        let c = format!("sha256:{}", crate::util::sha256_hex(b"{}"));
        std::fs::create_dir_all(dir.join("blobs")).unwrap();
        std::fs::write(dir.join("blobs").join(d.replace(':', "-")), weights).unwrap();
        std::fs::write(dir.join("blobs").join(c.replace(':', "-")), "{}").unwrap();
        let (n, tag) = name.split_once(':').unwrap();
        let m = dir.join("manifests/registry.ollama.ai/library").join(n);
        std::fs::create_dir_all(&m).unwrap();
        let layers = json!([{"mediaType": "application/vnd.ollama.image.model", "digest": d, "size": weights.len()}]);
        std::fs::write(
            m.join(tag),
            json!({"config": {"digest": c}, "layers": layers}).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn the_client_reads_models_generates_and_embeds_through_the_runtimes_api() {
        let store = tempfile::tempdir().unwrap();
        store_with(store.path(), "small:8b");
        let (addr, seen) = canned(&[
            (
                "/api/tags",
                r#"{"models":[{"name":"small:8b"},{"name":"big:cloud"}]}"#,
            ),
            (
                "/api/generate",
                r#"{"model":"small:8b","response":"a reply","done":true}"#,
            ),
            (
                "/api/embed",
                r#"{"model":"small:8b","embeddings":[[0.5,0.25],[1.0,0.0]]}"#,
            ),
        ]);
        let rt = LocalRuntime::new(Ollama {
            addr,
            models_dir: store.path().to_path_buf(),
        });
        assert_eq!(
            rt.state(),
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
        assert_eq!(rt.generate("small:8b", "say \"hi\"").unwrap(), "a reply");
        assert_eq!(
            rt.embed("small:8b", &["one", "two"]).unwrap(),
            [vec![0.5, 0.25], vec![1.0, 0.0]]
        );
        assert!(rt.generate("big:cloud", "x").is_err());

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "the cloud model was never sent: {seen:?}");
        let body = |i: usize| -> Value {
            serde_json::from_str(seen[i].split_once("\r\n\r\n").unwrap().1).unwrap()
        };
        assert_eq!(
            body(1),
            json!({"model": "small:8b", "prompt": "say \"hi\"", "stream": false, "format": "json"})
        );
        assert_eq!(
            body(2),
            json!({"model": "small:8b", "input": ["one", "two"]})
        );
    }

    #[test]
    fn an_answer_that_trickles_in_cannot_outlast_the_requests_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let _ = conn.read(&mut [0u8; 4096]);
            let _ = conn.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
            // A byte every 50 ms, well inside any read timeout, forever.
            while conn.write_all(b" ").is_ok() {
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let ollama = Ollama {
            addr,
            models_dir: PathBuf::from("/nonexistent"),
        };
        let started = Instant::now();
        let r = ollama.request("POST", "/api/generate", None, Duration::from_millis(400));
        assert!(
            matches!(&r, Err(RuntimeError::Failed(why)) if why.contains("no whole answer")),
            "{r:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn nothing_answering_is_not_running_and_no_store_or_binary_is_not_installed() {
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let empty = tempfile::tempdir().unwrap();
        let rt = LocalRuntime::new(Ollama {
            addr: closed,
            models_dir: empty.path().join("missing"),
        });
        assert!(matches!(
            rt.state(),
            RuntimeState::NotInstalled | RuntimeState::NotRunning
        ));
        let rt = LocalRuntime::new(Ollama {
            addr: closed,
            models_dir: empty.path().to_path_buf(),
        });
        assert_eq!(
            rt.state(),
            RuntimeState::NotRunning,
            "a model store means installed"
        );
    }

    #[test]
    fn the_runtime_is_only_ever_reached_on_this_machine() {
        let at = |h: Option<&str>| loopback_addr(h).to_string();
        assert_eq!(at(None), "127.0.0.1:11434");
        assert_eq!(at(Some("127.0.0.1:9999")), "127.0.0.1:9999");
        assert_eq!(at(Some("http://localhost:9999/")), "127.0.0.1:9999");
        assert_eq!(at(Some("0.0.0.0")), "127.0.0.1:11434");
        assert_eq!(at(Some(":9999")), "127.0.0.1:9999");
        assert_eq!(at(Some("[::1]:9999")), "[::1]:9999");
        for elsewhere in [
            "192.168.1.20:11434",
            "https://ollama.example:443",
            "gpu-box",
            "10.0.0.1",
        ] {
            assert_eq!(at(Some(elsewhere)), "127.0.0.1:11434", "{elsewhere}");
        }
    }

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
        assert!(model_is_local(store, "qwen-local"));
        assert!(model_is_local(store, "qwen-local:latest"));
        assert!(!model_is_local(store, "reviewer"));
        assert!(!model_is_local(store, "sneaky"));
        assert!(!model_is_local(store, "missing:7b"));
        // Names Ollama treats as cloud sources are refused whatever the store holds.
        put("qwen-local", "cloud", "{}", Some(b"weights"));
        put("qwen-local", "8b-cloud", "{}", Some(b"weights"));
        assert!(!model_is_local(store, "qwen-local:cloud"));
        assert!(!model_is_local(store, "qwen-local:8b-cloud"));
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
        assert!(!model_is_local(store, "qwen-local"));
        // The same holds when the name spells out the registry Ollama treats as `ollama.com`.
        assert!(!model_is_local(
            store,
            "registry.ollama.ai/library/qwen-local"
        ));
        assert!(!model_is_local(store, "ollama.com/library/qwen-local"));
        assert!(!model_is_local(store, "../../etc/passwd"));
    }
}
