//! A spoke's raw archive on the hub (ADR 0010, issues #137 and #138), through the real binary: its
//! captures parse into sessions of their own, keyed by machine, beside the hub's, `verify` checks
//! it, and the hub pulls it from the spoke over SSH.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn distill(user: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    distill_with(user, home, args, &[])
}

fn distill_with(user: &Path, home: &Path, args: &[&str], env: &[(&str, &Path)]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_distill"))
        .envs(env.iter().map(|(k, v)| (*k, v.as_os_str())))
        .env("HOME", user)
        .env_remove("DISTILL_HOME")
        .env("OLLAMA_HOST", "127.0.0.1:1")
        .args(["--home", home.to_str().unwrap(), "--json"])
        .args(args)
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

fn json(user: &Path, home: &Path, args: &[&str]) -> Value {
    let (ok, out) = distill(user, home, args);
    assert!(ok, "distill {args:?} failed: {out}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: {e}\n{out}"))
}

/// Copy a folder's files, keeping its layout.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
fn a_spokes_captures_are_sessions_keyed_by_its_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let user = tmp.path().join("user");
    let claude = tmp.path().join("claude/proj");
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        claude.join("s1.jsonl"),
        r#"{"type":"user","cwd":"/work/app","sessionId":"s1","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":"fix the failing test"}}
"#,
    )
    .unwrap();
    let home = tmp.path().join("home");
    json(&user, &home, &["init"]);
    fs::write(
        home.join("config.toml"),
        format!(
            "[[harnesses]]\nkind = \"claude-code\"\nenabled = true\nroots = [\"{}\"]\n",
            claude.parent().unwrap().display()
        ),
    )
    .unwrap();
    json(&user, &home, &["sweep"]);

    // The same captures, as another machine's: same origin paths, same session id.
    copy(&home.join("raw"), &home.join("machines/nas/raw"));
    let parsed = json(&user, &home, &["parse"]);
    assert_eq!(parsed["sessions"], 2, "{parsed}");
    // The spoke's session answers to its key on every face, here the owner's tags.
    let tagged = json(
        &user,
        &home,
        &["tag", "nas:claude-code/s1", "--add", "server"],
    );
    assert!(tagged.to_string().contains("server"), "{tagged}");
    json(&user, &home, &["tag", "claude-code/s1", "--add", "laptop"]);

    let checked = json(&user, &home, &["verify"]);
    assert_eq!(checked["checked"], 2, "{checked}");
    // A spoke's blob that changed is the spoke's, by name.
    let blob = walk(&home.join("machines/nas/raw"))
        .into_iter()
        .find(|p| !p.ends_with("manifest.jsonl"))
        .unwrap();
    fs::write(&blob, "changed").unwrap();
    let (ok, out) = distill(&user, &home, &["verify"]);
    assert!(!ok && out.contains("nas: "), "{out}");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut all = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            all.extend(walk(&p));
        } else {
            all.push(p);
        }
    }
    all
}

/// A stand-in for `ssh` on `PATH`: it runs the command it was given against the spoke's home, as
/// SSH would run it on the spoke, or as a forced command when `FAKE_FORCED` is set, and logs the
/// host. `FAKE_DOWN` makes it fail as an unreachable host does.
fn fake_ssh(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let script = r#"#!/bin/sh
while [ "$1" != "--" ]; do shift; done
shift
host=$1
cmd=$2
echo "$host" >> "$FAKE_LOG"
if [ -n "$FAKE_DOWN" ]; then
  echo "ssh: connect to host $host port 22: Connection refused" >&2
  exit 255
fi
export HOME="$FAKE_SPOKE_USER" DISTILL_HOME="$FAKE_SPOKE_HOME"
if [ -n "$FAKE_FORCED" ]; then
  SSH_ORIGINAL_COMMAND="$cmd" exec "$FAKE_DISTILL" export --ssh
fi
exec sh -c "$cmd"
"#;
    let path = dir.join("ssh");
    fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_hub_pulls_a_spokes_new_captures_over_ssh() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    fake_ssh(&bin);
    let distill_bin = Path::new(env!("CARGO_BIN_EXE_distill"));
    // The spoke: one session, under its user's Claude Code folder, swept.
    let spoke_user = tmp.path().join("spoke-user");
    let spoke = tmp.path().join("spoke");
    let session = spoke_user.join(".claude/projects/proj/s1.jsonl");
    fs::create_dir_all(session.parent().unwrap()).unwrap();
    let line = |said: &str| {
        format!(
            "{{\"type\":\"user\",\"cwd\":\"/work/app\",\"sessionId\":\"s1\",\"timestamp\":\"2026-11-10T10:00:00Z\",\"message\":{{\"role\":\"user\",\"content\":\"{said}\"}}}}\n"
        )
    };
    fs::write(&session, line("fix the failing test")).unwrap();
    json(&spoke_user, &spoke, &["init", "--spoke", "nas"]);
    json(&spoke_user, &spoke, &["tick"]);

    // The hub, with nothing of its own, pulling `nas`.
    let user = tmp.path().join("user");
    let home = tmp.path().join("home");
    json(&user, &home, &["init"]);
    let config = home.join("config.toml");
    let base = fs::read_to_string(&config).unwrap();
    let with = |name: &str| {
        format!(
            "{base}\n[[machines]]\nname = \"{name}\"\nhost = \"me@nas\"\ncommand = \"{}\"\n",
            distill_bin.display()
        )
    };
    fs::write(&config, with("nas")).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let log = tmp.path().join("ssh.log");
    let env = [
        ("PATH", Path::new(&path)),
        ("FAKE_LOG", log.as_path()),
        ("FAKE_SPOKE_USER", spoke_user.as_path()),
        ("FAKE_SPOKE_HOME", spoke.as_path()),
        ("FAKE_DISTILL", distill_bin),
    ];
    let pull = |extra: &[(&str, &Path)]| {
        let all: Vec<_> = env.iter().chain(extra).copied().collect();
        distill_with(&user, &home, &["pull"], &all)
    };

    let (ok, out) = pull(&[]);
    assert!(ok, "{out}");
    let r: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(r["machines"][0]["lines"], 1, "{r}");
    assert_eq!(r["machines"][0]["blobs"], 1, "{r}");
    assert_eq!(fs::read_to_string(&log).unwrap(), "me@nas\n");
    assert_eq!(
        fs::read(spoke.join("raw/manifest.jsonl")).unwrap(),
        fs::read(home.join("machines/nas/raw/manifest.jsonl")).unwrap()
    );

    // The session grows on the spoke; the hub's tick pulls what's new and parses it as nas's.
    fs::write(
        &session,
        line("fix the failing test") + &line("and the flaky one"),
    )
    .unwrap();
    json(&spoke_user, &spoke, &["tick"]);
    let all: Vec<_> = env.to_vec();
    let (ok, out) = distill_with(&user, &home, &["tick"], &all);
    assert!(ok, "{out}");
    let t: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(t["steps"][1]["name"], "pull", "{t}");
    assert_eq!(t["steps"][1]["report"]["machines"][0]["lines"], 1, "{t}");
    assert_eq!(t["steps"][2]["report"]["sessions"], 1, "{t}");
    assert!(
        home.join("sessions/machines/nas/claude-code/s1/meta.json")
            .exists()
    );
    let tagged = json(
        &user,
        &home,
        &["tag", "nas:claude-code/s1", "--add", "server"],
    );
    assert!(tagged.to_string().contains("server"), "{tagged}");

    // Through a forced command the same; and the spoke logged every export.
    let forced = tmp.path().join("forced");
    let (ok, out) = pull(&[("FAKE_FORCED", forced.as_path())]);
    assert!(ok, "{out}");
    let exports = fs::read_to_string(spoke.join("log/exports.jsonl")).unwrap();
    assert_eq!(exports.lines().count(), 3, "{exports}");

    // A host that can't be reached fails with what ssh said.
    let (ok, out) = pull(&[("FAKE_DOWN", forced.as_path())]);
    assert!(!ok && out.contains("Connection refused"), "{out}");

    // A machine that calls itself something else is refused before anything is taken.
    fs::write(&config, with("box")).unwrap();
    let (ok, out) = pull(&[]);
    assert!(!ok && out.contains(r#"is \"nas\", not \"box\""#), "{out}");
    assert!(!home.join("machines/box/raw/manifest.jsonl").exists());
}
