use super::*;
use crate::archive::Archive;
use crate::harness::Harness;

/// A fake key, built at run time so no secret scanner mistakes it for a leak.
fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Zz9".repeat(9)].concat()
}

/// A home with synthetic Claude Code sessions, archived and parsed. Each session `s<n>` opens
/// with `said` and runs one tool call whose input holds two secrets.
fn home_with_sessions(said: &[&str]) -> (tempfile::TempDir, Home) {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().join("home"));
    home.ensure().unwrap();
    let mut a = Archive::open(&home).unwrap();
    for (n, said) in said.iter().enumerate() {
        let said = serde_json::to_string(said).unwrap();
        let body = r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":SAID}}
{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"export DB_PASSWORD=plaintext-value-1","password":"plaintext-pass-2"}}]}}
{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}
"#
        .replace("SAID", &said);
        a.put(
            Harness::ClaudeCode,
            &format!("/s/proj/s{n}.jsonl"),
            "1",
            body.as_bytes(),
        )
        .unwrap();
    }
    crate::normalize::normalize(&home, false).unwrap();
    (dir, home)
}

fn facts(home: &Home, id: &str) -> SessionMeta {
    normalize::read_session_list(home)
        .unwrap()
        .into_iter()
        .find(|m| m.session_id == id)
        .unwrap()
}

#[test]
fn findings_place_each_secret_and_never_hold_its_value() {
    let key = fake_key();
    let (_d, home) = home_with_sessions(&[&format!("use {key} for the deploy")]);
    let r = scan_all(&home, &Config::default()).unwrap();
    assert_eq!((r.sessions, r.scanned, r.current, r.failed), (1, 1, 0, 0));
    assert_eq!((r.findings, r.secrets), (3, 3));

    let s0 = facts(&home, "s0");
    let f = load(&home, &s0).unwrap();
    assert_eq!(f.session, "claude-code/s0");
    let events = normalize::read_events(&home, &s0).unwrap();
    let mut seen = Vec::new();
    for x in &f.findings {
        let e = events.iter().find(|e| e.seq == x.event).unwrap();
        let place = match &x.field {
            Field::Text => e.text.clone().unwrap(),
            Field::Input { path, key } => {
                assert!(!key);
                at(e.input.as_ref().unwrap(), path)
                    .unwrap()
                    .1
                    .as_str()
                    .unwrap()
                    .to_string()
            }
        };
        seen.push((x.kind, x.field.clone(), place[x.start..x.end].to_string()));
    }
    assert_eq!(
        seen,
        [
            (Kind::AnthropicKey, Field::Text, key.clone()),
            (
                Kind::AssignedSecret,
                Field::Input {
                    path: vec![0],
                    key: false
                },
                "plaintext-value-1".into()
            ),
            // Under a credential-named key the whole value is one finding.
            (
                Kind::CredentialField,
                Field::Input {
                    path: vec![1],
                    key: false
                },
                "plaintext-pass-2".into()
            ),
        ]
    );

    let file = fs::read_to_string(home.findings_path(Harness::ClaudeCode, "s0")).unwrap();
    for value in [key.as_str(), "plaintext-value-1", "plaintext-pass-2"] {
        assert!(!file.contains(value), "{file}");
    }
    let stored: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(stored["made_from"]["detector"], DETECTOR_VERSION);
    assert_eq!(stored["made_from"]["capture_hash"], s0.raw.sha256);
    assert_eq!(stored["made_from"]["parser"], s0.normalizer.version);
    assert_eq!(stored["findings"][0]["field"], "text");
    assert_eq!(stored["findings"][1]["field"], "input");
    assert_eq!(stored["findings"][1]["path"], serde_json::json!([0]));
    assert_eq!(stored["findings"][0]["kind"], "anthropic_key");
    let fp = f.findings[0].fingerprint.as_str();
    assert!(
        fp.len() == 64 && fp.bytes().all(|b| b.is_ascii_hexdigit()),
        "{fp}"
    );
    assert_eq!(
        f.findings[0].placeholder(),
        format!("[secret: anthropic_key #{}]", &fp[..8])
    );
}

#[cfg(unix)]
fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_key_is_made_on_first_use_owner_only_and_the_same_value_has_one_fingerprint() {
    let key = fake_key();
    let said = format!("use {key} for the deploy");
    let (_d, home) = home_with_sessions(&[&said, &format!("again: {key}")]);
    assert!(FingerprintKey::existing(&home).unwrap().is_none());
    assert!(!home.fingerprint_key_path().exists());

    scan_all(&home, &Config::default()).unwrap();
    assert!(home.fingerprint_key_path().exists());
    #[cfg(unix)]
    {
        assert_eq!(mode(&home.fingerprint_key_path()), 0o600);
        assert_eq!(mode(&home.keys_dir()), 0o700);
    }
    let stored = fs::read_to_string(home.fingerprint_key_path()).unwrap();
    assert!(!stored.contains(&key));

    // The same value has the same fingerprint in every session, and the key stays.
    let fp = |id: &str| {
        load(&home, &facts(&home, id)).unwrap().findings[0]
            .fingerprint
            .clone()
    };
    assert_eq!(fp("s0"), fp("s1"));
    let k = FingerprintKey::open(&home).unwrap();
    assert_eq!(k.fingerprint(&key), fp("s0"));
    assert_ne!(k.fingerprint("plaintext-other"), fp("s0"));
    assert_eq!(
        fs::read_to_string(home.fingerprint_key_path()).unwrap(),
        stored
    );

    // Another home has another key.
    let (_d2, other) = home_with_sessions(&[&said]);
    assert_ne!(
        FingerprintKey::open(&other).unwrap().fingerprint(&key),
        fp("s0")
    );

    // A key file that is not a key fails, rather than being replaced.
    fs::write(home.fingerprint_key_path(), "not a key").unwrap();
    assert!(FingerprintKey::open(&home).is_err());
    assert!(scan_all(&home, &Config::default()).is_err());
}

#[test]
fn findings_go_out_of_date_when_what_they_were_made_from_changes() {
    let (_d, home) = home_with_sessions(&["nothing secret here"]);
    let before = progress(&home).unwrap();
    assert_eq!((before.sessions, before.current, before.pending), (1, 0, 1));
    assert!(
        !home.fingerprint_key_path().exists(),
        "status creates nothing"
    );

    assert_eq!(scan_all(&home, &Config::default()).unwrap().scanned, 1);
    let again = scan_all(&home, &Config::default()).unwrap();
    assert_eq!((again.scanned, again.current), (0, 1));
    let p = progress(&home).unwrap();
    assert_eq!((p.current, p.pending, p.secrets), (1, 0, 2));

    let path = home.findings_path(Harness::ClaudeCode, "s0");
    for (field, other) in [
        ("detector", serde_json::json!(DETECTOR_VERSION + 1)),
        ("capture_hash", serde_json::json!("0".repeat(64))),
        ("parser", serde_json::json!(999)),
        ("key", serde_json::json!("0123456789abcdef")),
        ("personal_terms", serde_json::json!("0123456789abcdef")),
    ] {
        let mut f: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        f["made_from"][field] = other;
        fs::write(&path, f.to_string()).unwrap();
        assert_eq!(progress(&home).unwrap().pending, 1, "{field}");
        assert_eq!(
            scan_all(&home, &Config::default()).unwrap().scanned,
            1,
            "{field}"
        );
        assert_eq!(
            scan_all(&home, &Config::default()).unwrap().current,
            1,
            "{field}"
        );
    }

    // A new key changes every fingerprint at the next scan.
    let old = load(&home, &facts(&home, "s0")).unwrap();
    fs::remove_file(home.fingerprint_key_path()).unwrap();
    assert_eq!(scan_all(&home, &Config::default()).unwrap().scanned, 1);
    let new = load(&home, &facts(&home, "s0")).unwrap();
    assert_ne!(old.findings[0].fingerprint, new.findings[0].fingerprint);

    // A file that can't be read is missing.
    fs::write(&path, "{").unwrap();
    assert_eq!(scan_all(&home, &Config::default()).unwrap().scanned, 1);
}

#[test]
fn a_short_form_finds_every_fingerprint_it_starts() {
    let key = fake_key();
    let (_d, home) = home_with_sessions(&[&format!("use {key}")]);
    scan_all(&home, &Config::default()).unwrap();
    let f = load(&home, &facts(&home, "s0")).unwrap();
    let fp = f.findings[0].fingerprint.clone();
    let short = fp.short().to_string();
    assert_eq!(short.len(), 8);
    let found = lookup(&home, &short).unwrap();
    assert!(found.contains(&fp), "{found:?}");
    assert!(found.iter().all(|x| x.as_str().starts_with(&short)));
    assert!(
        lookup(&home, &short.to_ascii_uppercase())
            .unwrap()
            .contains(&fp)
    );
    // Two characters match more of them; an empty prefix matches none.
    assert!(lookup(&home, &short[..2]).unwrap().contains(&fp));
    assert!(lookup(&home, "").unwrap().is_empty());
    assert!(lookup(&home, "zz").unwrap().is_empty());
}

#[test]
fn personal_details_are_stored_beside_the_secrets_with_their_own_kinds() {
    let key = fake_key();
    let (_d, home) = home_with_sessions(&[&format!(
        "mail jane@example.com about Project Falcon, then use {key}"
    )]);
    let mut config = Config::default();
    config.privacy.personal_terms = vec!["project falcon".into()];
    config.save(&home.config_path()).unwrap();
    let r = scan_all(&home, &config).unwrap();
    assert_eq!((r.findings, r.secrets), (5, 3));

    let s0 = facts(&home, "s0");
    let f = load(&home, &s0).unwrap();
    let text = normalize::read_events(&home, &s0).unwrap()[0]
        .text
        .clone()
        .unwrap();
    let in_text: Vec<_> = f
        .in_event(0)
        .map(|x| (x.kind, x.is_secret(), &text[x.start..x.end]))
        .collect();
    assert_eq!(
        in_text,
        [
            (Kind::Email, false, "jane@example.com"),
            (Kind::PersonalTerm, false, "Project Falcon"),
            (Kind::AnthropicKey, true, key.as_str()),
        ]
    );
    let file = fs::read_to_string(home.findings_path(Harness::ClaudeCode, "s0")).unwrap();
    for value in ["jane@example.com", "Falcon", "falcon"] {
        assert!(!file.contains(value), "{file}");
    }
    let stored: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(stored["findings"][0]["kind"], "email");
    assert_eq!(stored["findings"][1]["kind"], "personal_term");
    // The same address has one fingerprint wherever it appears.
    let email = &f.findings[0].fingerprint;
    assert_eq!(
        *email,
        FingerprintKey::open(&home)
            .unwrap()
            .fingerprint("jane@example.com")
    );

    // Changing the list makes every session's findings out of date.
    assert_eq!(progress(&home).unwrap().pending, 0);
    config.privacy.personal_terms.push("Acme".into());
    config.save(&home.config_path()).unwrap();
    assert_eq!(progress(&home).unwrap().pending, 1);
    assert_eq!(scan_all(&home, &config).unwrap().scanned, 1);
    assert_eq!(scan_all(&home, &config).unwrap().current, 1);
    // Back to no terms: the term is no longer found.
    assert_eq!(scan_all(&home, &Config::default()).unwrap().scanned, 1);
    let f = load(&home, &s0).unwrap();
    assert!(f.findings.iter().all(|x| x.kind != Kind::PersonalTerm));
}

#[test]
fn the_detector_is_told_the_personal_terms_and_the_owners_username() {
    let mut config = Config::default();
    config.privacy.personal_terms = vec!["Acme".into()];
    let owner = owner(&config);
    assert_eq!(owner.terms, ["Acme"]);
    let home = crate::home::user_home().unwrap();
    assert_eq!(
        owner.username.as_deref(),
        home.file_name().and_then(|n| n.to_str())
    );
}
