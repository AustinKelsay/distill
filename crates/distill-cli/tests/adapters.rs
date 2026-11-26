//! The backup adapters DISTILL ships (issue #179, `adapters/`), through the real binary: a home
//! backs up through each, verifies, backs up again with nothing new to send, and comes back from
//! it with `restore --kind`.
//!
//! - `distill-backup-ssh` runs against a stand-in for `ssh` that runs each command on this
//!   machine, as the other machine's shell would, so its commands run in a real shell.
//! - `distill-backup-rclone` runs against rclone's `local` backend. Without `rclone` on PATH that
//!   case says so and is skipped.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Machine {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    user: PathBuf,
    bin: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let user = root.join("user");
        let bin = user.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let adapters = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters");
        for e in fs::read_dir(adapters).unwrap() {
            let e = e.unwrap();
            let to = bin.join(e.file_name());
            fs::copy(e.path(), &to).unwrap();
            fs::set_permissions(&to, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let claude = root.join("claude/proj");
        fs::create_dir_all(&claude).unwrap();
        fs::write(
            claude.join("s1.jsonl"),
            r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"fix the failing test"}}
"#,
        )
        .unwrap();
        Machine {
            _tmp: tmp,
            root,
            user,
            bin,
        }
    }

    fn run(&self, home: &Path, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_distill"))
            .args(["--home", home.to_str().unwrap()])
            .args(args)
            .env("HOME", &self.user)
            .env("TMPDIR", &self.root)
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("OLLAMA_HOST", "127.0.0.1:1")
            .env_remove("OLLAMA_MODELS")
            .env_remove("DISTILL_HOME")
            .output()
            .unwrap();
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    fn ok(&self, home: &Path, args: &[&str]) -> String {
        let (ok, out) = self.run(home, args);
        assert!(ok, "distill {args:?} failed: {out}");
        out
    }

    /// Back up through the adapter `kind` with these settings, check, back up again, and
    /// restore. `objects` is where the adapter keeps the objects, to look at.
    fn journey(&self, kind: &str, settings: &[(&str, String)], objects: &Path) {
        let home = self.root.join("home");
        self.ok(&home, &["init"]);
        let table: String = settings
            .iter()
            .map(|(k, v)| format!("{k} = \"{v}\"\n"))
            .collect();
        fs::write(
            home.join("config.toml"),
            format!(
                "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n\n\
                 [[backup]]\nname = \"offsite\"\nkind = \"{kind}\"\n{table}",
                self.root.join("claude").display(),
            ),
        )
        .unwrap();
        self.ok(&home, &["sweep"]);
        let key = self.ok(&home, &["--json", "backup", "init"]);
        let key: serde_json::Value = serde_json::from_str(&key).unwrap();
        let key_file = self.root.join("key.txt");
        fs::write(&key_file, key["backup_key"].as_str().unwrap()).unwrap();

        let first = self.ok(&home, &["--json", "backup", "run"]);
        assert!(first.contains("\"ok\": true"), "{first}");
        let stored = fs::read_dir(objects).unwrap().count();
        assert!(stored > 0, "{kind} stored nothing");
        let check = self.ok(&home, &["backup", "verify", "--full"]);
        assert!(check.contains("fetched and decrypted"), "{check}");
        // Nothing new: the second run sends no blob again, only its record and logs.
        let second = self.ok(&home, &["--json", "backup", "run"]);
        assert!(second.contains("\"blobs_sent\": 0"), "{second}");

        let restored = self.root.join("restored");
        let mut args = vec!["restore", "--kind", kind];
        let sets: Vec<String> = settings.iter().map(|(k, v)| format!("{k}={v}")).collect();
        for s in &sets {
            args.extend(["--set", s]);
        }
        args.extend(["--key-file", key_file.to_str().unwrap()]);
        let back = self.ok(&restored, &args);
        assert!(back.contains("Restored into"), "{back}");
        assert!(restored.join("raw").is_dir());
    }
}

#[test]
fn a_home_backs_up_over_ssh_and_comes_back() {
    let m = Machine::new();
    // Stands in for ssh: drops its options and the host, and runs the command here.
    let log = m.root.join("ssh.log");
    fs::write(
        m.bin.join("ssh"),
        format!(
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    -O) exit 0 ;;\n    \
             -o) shift 2 ;;\n    -*) shift ;;\n    *) break ;;\n  esac\ndone\n\
             echo \"$1\" >> '{}'\nshift\nexec sh -c \"$*\"\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(m.bin.join("ssh"), fs::Permissions::from_mode(0o755)).unwrap();
    let dir = m.root.join("far away");
    fs::create_dir(&dir).unwrap();
    m.journey(
        "ssh",
        &[
            ("host", "me@nas".into()),
            ("dir", dir.display().to_string()),
        ],
        &dir.join("objects"),
    );
    let hosts = fs::read_to_string(&log).unwrap();
    assert!(hosts.lines().all(|h| h == "me@nas"), "{hosts}");

    // A folder that isn't there is the adapter's error, with the host.
    let (ok, out) = m.run(
        &m.root.join("other"),
        &[
            "restore",
            "--kind",
            "ssh",
            "--set",
            "host=me@nas",
            "--set",
            "dir=/nowhere",
            "--key-file",
            m.root.join("key.txt").to_str().unwrap(),
        ],
    );
    assert!(
        !ok && out.contains("distill-backup-ssh: me@nas:/nowhere is not there"),
        "{out}"
    );
}

#[test]
fn a_home_backs_up_through_rclone_and_comes_back() {
    let found = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("rclone").is_file()));
    if !found {
        eprintln!("skipped: no rclone on PATH");
        return;
    }
    let m = Machine::new();
    let dir = m.root.join("bucket");
    fs::create_dir(&dir).unwrap();
    m.journey(
        "rclone",
        &[("remote", format!(":local:{}", dir.display()))],
        &dir.join("objects"),
    );
}
