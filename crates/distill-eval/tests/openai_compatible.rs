//! The evaluation's stand-in runtime, against a canned OpenAI-compatible server on a loopback
//! port: the model steps run with its models, read as `<provider>/<model>`, and only when the
//! home trusts that provider.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};

use distill_core::archive::Archive;
use distill_core::config::Config;
use distill_core::harness::Harness;
use distill_core::home::Home;
use distill_core::runtime::LocalRuntime;
use distill_core::tick::Deadline;
use distill_core::trust::Reader;

const CARD: &str =
    r#"{"title":"Rename the widget","summary":"Renamed it.","tags":["ui"],"outcome":"done"}"#;

/// Answers `/v1/models`, chat completions with [`CARD`] and embeddings with a fixed vector per
/// text.
fn server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut conn = conn.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 65536];
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
            let body = match path.as_str() {
                "/v1/models" => {
                    serde_json::json!({"data": [{"id": "chat-small"}, {"id": "embed-small"}]})
                }
                "/v1/chat/completions" => {
                    serde_json::json!({"choices": [{"message": {"role": "assistant", "content": CARD}}]})
                }
                "/v1/embeddings" => {
                    let sent: serde_json::Value =
                        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                    let n = sent["input"].as_array().unwrap().len();
                    let data: Vec<_> = (0..n)
                        .rev()
                        .map(|i| serde_json::json!({"index": i, "embedding": [1.0, i as f32, 0.5]}))
                        .collect();
                    serde_json::json!({ "data": data })
                }
                _ => serde_json::json!({"error": {"message": "no such path"}}),
            };
            let _ = write!(
                conn,
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{body}"
            );
        }
    });
    addr
}

fn home_with_a_session(trusted: bool) -> (tempfile::TempDir, Home, Config) {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().join("home"));
    home.ensure().unwrap();
    let session = r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-01T10:00:00Z","message":{"role":"user","content":"rename the widget"}}
{"type":"assistant","timestamp":"2026-11-01T10:00:05Z","message":{"role":"assistant","model":"m","content":[{"type":"text","text":"Renamed."}]}}
"#;
    let mut a = Archive::open(&home).unwrap();
    a.put(
        Harness::ClaudeCode,
        "/s/proj/s1.jsonl",
        "1",
        session.as_bytes(),
    )
    .unwrap();
    drop(a);
    distill_core::normalize::normalize(&home, false).unwrap();
    let mut config = Config {
        harnesses: vec![],
        ..Default::default()
    };
    if trusted {
        distill_core::trust::add(&mut config, "maple", true).unwrap();
    }
    config.steward.card_model = Some("chat-small".into());
    config.steward.embedding_model = Some("embed-small".into());
    config.save(&home.config_path()).unwrap();
    distill_core::findings::scan_all(&home, &config).unwrap();
    (dir, home, config)
}

#[test]
fn the_model_steps_run_with_a_trusted_providers_models_and_read_as_that_provider() {
    let (_d, home, config) = home_with_a_session(true);
    let runtime = LocalRuntime::openai_compatible("maple", server()).unwrap();
    assert_eq!(runtime.provider(), "maple");

    let r = distill_core::cards::write(&home, &config, &runtime, Deadline::NONE).unwrap();
    assert_eq!((r.written, r.failed), (1, 0), "{r:?}");
    let r = distill_core::index::vectors::build(&home, &config, &runtime, Deadline::NONE).unwrap();
    assert_eq!((r.embedded, r.failed), (1, 0), "{r:?}");

    let readers: Vec<Reader> = distill_core::readlog::list(&home, None, usize::MAX)
        .unwrap()
        .into_iter()
        .map(|e| e.reader)
        .collect();
    assert!(
        readers.contains(&Reader::model(Some("maple"), Some("chat-small"))),
        "{readers:?}"
    );
    assert!(
        readers.contains(&Reader::model(Some("maple"), Some("embed-small"))),
        "{readers:?}"
    );
    assert!(
        !readers
            .iter()
            .any(|r| matches!(r, Reader::Model { provider: Some(p), .. } if p == "ollama")),
        "{readers:?}"
    );
}

#[test]
fn a_provider_the_home_does_not_trust_is_refused_by_the_read_path() {
    let (_d, home, config) = home_with_a_session(false);
    let runtime = LocalRuntime::openai_compatible("maple", server()).unwrap();
    let r = distill_core::cards::write(&home, &config, &runtime, Deadline::NONE).unwrap();
    assert_eq!((r.written, r.failed), (0, 1), "{r:?}");
    assert!(r.errors[0].contains("the read was refused"), "{r:?}");
    assert!(!home.card_path(Harness::ClaudeCode, "s1").exists());
}

#[test]
fn only_a_named_remote_provider_on_this_machine_can_stand_in() {
    let here: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    assert!(LocalRuntime::openai_compatible("ollama", here).is_err());
    assert!(LocalRuntime::openai_compatible(" ", here).is_err());
    assert!(LocalRuntime::openai_compatible("maple", "192.0.2.1:8080".parse().unwrap()).is_err());
    // The shipped runtime is Ollama's.
    assert_eq!(LocalRuntime::ollama().provider(), "ollama");
}
