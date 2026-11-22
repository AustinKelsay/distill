use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::*;

/// What the machine says when nothing of DISTILL listens and the disk is encrypted.
fn quiet() -> Probes {
    Probes {
        listeners: || Some(Vec::new()),
        disk_encrypted: || Some(true),
    }
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

/// A user home in a temp dir, and a DISTILL home in it built by DISTILL's own writers.
fn machine() -> (tempfile::TempDir, std::path::PathBuf, Home) {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user");
    fs::create_dir_all(&user).unwrap();
    let home = Home::new(user.join("distill"));
    home.ensure().unwrap();
    crate::Config::default().save(&home.config_path()).unwrap();
    (dir, user, home)
}

fn rule(e: &Exposure, rule: Rule) -> &RuleResult {
    e.rules.iter().find(|r| r.rule == rule).unwrap()
}

#[test]
fn a_home_distill_built_passes_every_rule() {
    let (_dir, user, home) = machine();
    crate::findings::FingerprintKey::open(&home).unwrap();
    let e = check_at(&home, &user, &quiet());
    assert_eq!(e.rules.len(), 4);
    for r in &e.rules {
        assert_eq!(r.outcome, Outcome::Pass, "{r:?}");
    }
    assert!(e.passes());
    assert_eq!(e.disk_encrypted, Some(true));
    assert!(
        e.backups.starts_with("No backup target is set up"),
        "{}",
        e.backups
    );
}

#[test]
fn a_file_or_folder_others_can_read_fails_owner_only_and_is_named() {
    let (_dir, user, home) = machine();
    chmod(&home.config_path(), 0o644);
    chmod(&home.sessions_dir(), 0o755);
    let e = check_at(&home, &user, &quiet());
    let r = rule(&e, Rule::OwnerOnly);
    assert_eq!(r.outcome, Outcome::Fail);
    assert!(!e.passes());
    let named = r.paths.join("\n");
    assert!(named.contains("config.toml"), "{named}");
    assert!(named.contains("sessions"), "{named}");
    assert!(r.message.contains('2'), "{}", r.message);

    // The home itself counts too.
    let (_dir, user, home) = machine();
    chmod(&home.root, 0o750);
    assert_eq!(
        rule(&check_at(&home, &user, &quiet()), Rule::OwnerOnly).outcome,
        Outcome::Fail
    );
}

#[test]
fn a_fingerprint_key_others_can_read_fails_its_rule() {
    let (_dir, user, home) = machine();
    // No key yet: nothing to expose.
    assert_eq!(
        rule(&check_at(&home, &user, &quiet()), Rule::KeyOwnerOnly).outcome,
        Outcome::Pass
    );
    crate::findings::FingerprintKey::open(&home).unwrap();
    chmod(&home.fingerprint_key_path(), 0o640);
    let e = check_at(&home, &user, &quiet());
    assert_eq!(rule(&e, Rule::KeyOwnerOnly).outcome, Outcome::Fail);
    assert_eq!(rule(&e, Rule::OwnerOnly).outcome, Outcome::Fail);
}

#[test]
fn a_home_in_a_synced_folder_fails() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user");
    for synced in [
        "Library/Mobile Documents/com~apple~CloudDocs/distill",
        "Library/CloudStorage/GoogleDrive-someone/My Drive/distill",
        "Dropbox/distill",
        "Dropbox (Personal)/archive/distill",
        "Google Drive/distill",
        "OneDrive - Example/distill",
    ] {
        let home = Home::new(user.join(synced));
        home.ensure().unwrap();
        let r = check_at(&home, &user, &quiet());
        let r = rule(&r, Rule::NotSynced);
        assert_eq!(r.outcome, Outcome::Fail, "{synced}: {r:?}");
    }
    for not_synced in ["distill", "Documents/distill", "dropbox-notes/distill"] {
        let home = Home::new(user.join(not_synced));
        home.ensure().unwrap();
        let r = check_at(&home, &user, &quiet());
        assert_eq!(
            rule(&r, Rule::NotSynced).outcome,
            Outcome::Pass,
            "{not_synced}"
        );
    }
    // A link into a synced folder is still in it.
    fs::create_dir_all(user.join("Dropbox/real")).unwrap();
    std::os::unix::fs::symlink(user.join("Dropbox/real"), user.join("linked")).unwrap();
    let home = Home::new(user.join("linked/distill"));
    let r = check_at(&home, &user, &quiet());
    assert_eq!(rule(&r, Rule::NotSynced).outcome, Outcome::Fail);
}

#[test]
fn a_distill_process_listening_on_a_port_fails() {
    let (_dir, user, home) = machine();
    let listening = Probes {
        listeners: || Some(vec!["distill-mcp (pid 41) on 127.0.0.1:7000".into()]),
        disk_encrypted: || None,
    };
    let e = check_at(&home, &user, &listening);
    let r = rule(&e, Rule::NoListener);
    assert_eq!(r.outcome, Outcome::Fail);
    assert!(r.paths[0].contains("127.0.0.1:7000"), "{r:?}");
    assert_eq!(e.disk_encrypted, None);

    let unknown = Probes {
        listeners: || None,
        disk_encrypted: || Some(false),
    };
    let e = check_at(&home, &user, &unknown);
    assert_eq!(rule(&e, Rule::NoListener).outcome, Outcome::Unchecked);
    assert!(
        e.passes(),
        "a rule that could not be checked is not a failure"
    );
}

#[test]
fn listeners_are_read_from_lsof() {
    let out = "COMMAND      PID  USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
distill      101 owner    9u  IPv4 0x1      0t0  TCP 127.0.0.1:52100->127.0.0.1:11434 (ESTABLISHED)
distill-mcp  102 owner   10u  IPv4 0x2      0t0  TCP 127.0.0.1:7000 (LISTEN)
distill-gui  103 owner   11u  IPv6 0x3      0t0  UDP *:5353
distill      104 owner   12u  IPv4 0x4      0t0  UDP 10.0.0.2:5000->10.0.0.1:53
distillery   105 owner   13u  IPv4 0x5      0t0  TCP *:8080 (LISTEN)
DISTILL      106 owner   14u  IPv4 0x6      0t0  TCP 127.0.0.1:7001 (LISTEN)
Distiller    107 owner   15u  IPv4 0x7      0t0  TCP *:8081 (LISTEN)
";
    assert_eq!(
        listeners_in(out),
        vec![
            "distill-mcp (pid 102) on 127.0.0.1:7000".to_string(),
            "distill-gui (pid 103) on *:5353".to_string(),
            "DISTILL (pid 106) on 127.0.0.1:7001".to_string(),
        ]
    );
    assert!(listeners_in("").is_empty());
}

#[test]
fn folders_the_previous_build_left_are_named_with_their_modes() {
    let (_dir, user, home) = machine();
    assert!(check_at(&home, &user, &quiet()).previous_build.is_empty());
    fs::create_dir_all(home.root.join("exports/recipe/b1")).unwrap();
    chmod(&home.root.join("exports"), 0o755);
    fs::create_dir_all(home.root.join("steward")).unwrap();
    chmod(&home.root.join("steward"), 0o700);
    fs::create_dir_all(user.join(".distill")).unwrap();
    chmod(&user.join(".distill"), 0o755);
    let e = check_at(&home, &user, &quiet());
    let left: Vec<_> = e
        .previous_build
        .iter()
        .map(|f| {
            (
                f.path.rsplit('/').next().unwrap().to_string(),
                f.mode.clone(),
                f.owner_only,
            )
        })
        .collect();
    assert_eq!(
        left,
        vec![
            ("exports".to_string(), "0755".to_string(), false),
            ("steward".to_string(), "0700".to_string(), true),
            (".distill".to_string(), "0755".to_string(), false),
        ]
    );
    assert!(e.previous_build.iter().all(|f| !f.holds.is_empty()));
}

#[test]
fn fix_tightens_every_mode_in_an_existing_home() {
    let (_dir, user, home) = machine();
    crate::findings::FingerprintKey::open(&home).unwrap();
    fs::create_dir_all(home.root.join("exports/r")).unwrap();
    fs::write(home.root.join("exports/r/data.jsonl"), "{}\n").unwrap();
    for (p, m) in [
        (home.root.clone(), 0o755),
        (home.config_path(), 0o644),
        (home.root.join("exports"), 0o775),
        (home.root.join("exports/r/data.jsonl"), 0o664),
        (home.fingerprint_key_path(), 0o644),
    ] {
        chmod(&p, m);
    }
    // A link out of the home is left alone, and so is what it points to.
    let outside = user.join("outside.txt");
    fs::write(&outside, "x").unwrap();
    chmod(&outside, 0o644);
    std::os::unix::fs::symlink(&outside, home.root.join("link")).unwrap();

    // Those five, and `exports/r`, made with the test's own loose umask.
    let fixed = fix(&home).unwrap();
    assert_eq!(fixed, 6);
    assert_eq!(mode(&home.root), 0o700);
    assert_eq!(mode(&home.config_path()), 0o600);
    assert_eq!(mode(&home.root.join("exports")), 0o700);
    assert_eq!(mode(&home.root.join("exports/r/data.jsonl")), 0o600);
    assert_eq!(mode(&outside), 0o644);
    let e = check_at(&home, &user, &quiet());
    assert!(e.passes(), "{e:?}");
    assert_eq!(fix(&home).unwrap(), 0);
}
