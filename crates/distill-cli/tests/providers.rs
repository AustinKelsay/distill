//! `distill providers` through the real binary against a temp home. No local runtime is
//! reachable: `PATH` holds no `ollama`, `HOME` has no model store, and `OLLAMA_HOST` points at a
//! loopback port nothing listens on, so no test contacts a real runtime.

use std::fs;
use std::net::TcpListener;
use std::process::Command;

use serde_json::Value;

struct Machine {
    _tmp: tempfile::TempDir,
    user: std::path::PathBuf,
    home: std::path::PathBuf,
    no_runtime: String,
}

impl Machine {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        fs::create_dir_all(user.join("bin")).unwrap();
        let home = tmp.path().join("home");
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        Self {
            user,
            home,
            no_runtime: closed.to_string(),
            _tmp: tmp,
        }
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_distill"))
            .env("HOME", &self.user)
            .env("PATH", self.user.join("bin"))
            .env("OLLAMA_HOST", &self.no_runtime)
            .env_remove("OLLAMA_MODELS")
            .env_remove("DISTILL_HOME")
            .args(["--home", self.home.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.run(args);
        assert!(ok, "distill {args:?} failed: {err}");
        out
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        serde_json::from_str(&self.ok(&a)).unwrap()
    }

    fn config(&self) -> String {
        fs::read_to_string(self.home.join("config.toml")).unwrap()
    }
}

fn row<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {listing}"))
}

#[test]
fn providers_are_listed_added_trusted_and_unmarked() {
    let m = Machine::new();
    m.ok(&["init"]);

    // With no local runtime installed, it says so; ollama is still trusted.
    let out = m.ok(&["providers"]);
    let ollama = out.lines().find(|l| l.starts_with("ollama ")).unwrap();
    assert!(
        ollama.contains("local") && ollama.contains("trusted"),
        "{out}"
    );
    assert!(out.contains("not installed"), "{out}");
    let l = m.json(&["providers"]);
    assert_eq!(row(&l, "ollama")["kind"], "local");
    assert_eq!(row(&l, "ollama")["trusted"], true);
    assert_eq!(row(&l, "ollama")["runtime"]["state"], "not_installed");
    assert_eq!(row(&l, "ollama-cloud")["kind"], "remote");
    assert_eq!(row(&l, "ollama-cloud")["trusted"], false);

    // Add a remote provider: listed, untrusted.
    m.ok(&["providers", "add", "example-cloud"]);
    let l = m.json(&["providers"]);
    assert_eq!(row(&l, "example-cloud")["kind"], "remote");
    assert_eq!(row(&l, "example-cloud")["trusted"], false);
    let cfg = m.config();
    assert!(
        cfg.contains("[[providers]]") && cfg.contains("name = \"example-cloud\""),
        "{cfg}"
    );

    m.ok(&["providers", "trust", "example-cloud"]);
    assert_eq!(
        row(&m.json(&["providers"]), "example-cloud")["trusted"],
        true
    );
    assert!(m.config().contains("trusted = true"), "{}", m.config());

    m.ok(&["providers", "untrust", "example-cloud"]);
    assert_eq!(
        row(&m.json(&["providers"]), "example-cloud")["trusted"],
        false
    );
    assert!(!m.config().contains("trusted = true"), "{}", m.config());

    m.ok(&["providers", "add", "other-cloud", "--trusted"]);
    assert_eq!(row(&m.json(&["providers"]), "other-cloud")["trusted"], true);

    // What can't be done says why.
    let (ok, _, err) = m.run(&["providers", "trust", "unheard-of"]);
    assert!(
        !ok && err.contains("distill providers add unheard-of"),
        "{err}"
    );
    let (ok, _, err) = m.run(&["providers", "untrust", "ollama"]);
    assert!(!ok && err.contains("always trusted"), "{err}");

    // With no runtime, everything else still works.
    m.ok(&["tick"]);
    m.ok(&["status"]);
}

#[test]
fn an_ignored_local_provider_is_warned_about() {
    let m = Machine::new();
    m.ok(&["init"]);
    let cfg =
        m.config() + "\n[[providers]]\nname = \"llama-box\"\nkind = \"local\"\ntrusted = true\n";
    fs::write(m.home.join("config.toml"), cfg).unwrap();
    let (ok, out, err) = m.run(&["providers"]);
    assert!(ok, "{err}");
    assert!(!out.contains("llama-box"), "{out}");
    assert!(
        err.contains("llama-box") && err.contains("ignored"),
        "{err}"
    );
}
