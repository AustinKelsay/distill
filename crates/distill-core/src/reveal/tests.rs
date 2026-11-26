use super::*;
use crate::archive::Archive;
use crate::harness::Harness;
use crate::runtime::{FakeRuntime, Stored};

/// A fake key and password, built at run time so no secret scanner mistakes them for a leak.
fn fake_key() -> String {
    ["sk-", "ant-", "api03-", &"Rv8".repeat(9)].concat()
}
fn fake_password() -> String {
    ["plaintext", "-reveal-", "pass-31"].concat()
}

/// A synthetic Claude Code session with a key in what the owner said and a password in a tool
/// call, archived and parsed into a temp home.
fn home_with_secrets() -> (tempfile::TempDir, Home) {
    let said = serde_json::to_string(&format!("deploy with {} please", fake_key())).unwrap();
    let input = serde_json::json!({
        "command": format!("export DB_PASSWORD={} && deploy", fake_password())
    });
    let body = r#"{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{"role":"user","content":SAID}}
{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":INPUT}]}}
{"type":"user","timestamp":"2026-11-10T10:00:09Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"moved"}]}}
"#
    .replace("SAID", &said)
    .replace("INPUT", &input.to_string());
    let dir = tempfile::tempdir().unwrap();
    let home = Home::new(dir.path().join("home"));
    home.ensure().unwrap();
    Archive::open(&home)
        .unwrap()
        .put(
            Harness::ClaudeCode,
            "/s/proj/s1.jsonl",
            "1",
            body.as_bytes(),
        )
        .unwrap();
    crate::normalize::normalize(&home, false).unwrap();
    let mut c = home.load_config().unwrap_or_default();
    crate::trust::add(&mut c, "example-cloud", true).unwrap();
    crate::trust::add(&mut c, "other-cloud", false).unwrap();
    c.save(&home.config_path()).unwrap();
    (dir, home)
}

fn runtime() -> LocalRuntime {
    FakeRuntime::stopped()
        .with_model("small:8b", Stored::Here)
        .runtime()
}

fn cloud() -> Reader {
    Reader::model(Some("example-cloud"), Some("m"))
}

/// The short fingerprints a model reads in the session's placeholders, key first.
fn shorts(home: &Home, rt: &LocalRuntime) -> Vec<String> {
    let crate::read::Read::Text(t) = crate::read::read(
        home,
        rt,
        &cloud(),
        "claude-code/s1",
        crate::read::Range::default(),
    )
    .unwrap() else {
        panic!("not served");
    };
    regex::Regex::new(r"\[secret: [a-z_]+ #([0-9a-f]{8})\]")
        .unwrap()
        .captures_iter(&t)
        .map(|c| c[1].to_string())
        .collect()
}

fn filed(f: Filed) -> Request {
    match f {
        Filed::Filed(r) => *r,
        Filed::Refused(why) => panic!("refused: {why}"),
    }
}

fn refused_filing(f: Filed) -> String {
    match f {
        Filed::Refused(why) => why,
        Filed::Filed(r) => panic!("filed: {r:?}"),
    }
}

/// Asserts that, outside the raw archive, only one file holds a value: one in the session's own
/// folder (its events; the leak test names it).
fn assert_no_value_outside_the_archive(home: &Home) {
    let session = home.session_dir(None, Harness::ClaudeCode, "s1");
    let holding: Vec<_> = walkdir::WalkDir::new(&home.root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| !p.starts_with(home.root.join("raw")))
        .filter(|p| {
            let text = String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned();
            [fake_key(), fake_password()]
                .iter()
                .any(|v| text.contains(v))
        })
        .collect();
    assert_eq!(holding.len(), 1, "{holding:?}");
    assert!(holding[0].starts_with(&session), "{holding:?}");
    for not in ["findings.json", "meta.json"] {
        assert_ne!(holding[0], session.join(not));
    }
}

fn log_results(home: &Home) -> Vec<serde_json::Value> {
    let mut log = crate::readlog::list(home, None, usize::MAX).unwrap();
    log.reverse();
    log.iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
}

#[test]
fn a_trusted_model_files_a_request_and_gets_an_id_and_no_value() {
    let (_d, home) = home_with_secrets();
    let rt = runtime();
    let fps = shorts(&home, &rt);
    assert_eq!(fps.len(), 2);
    let r = filed(
        request(
            &home,
            &rt,
            &cloud(),
            "claude-code/s1",
            &[format!("#{}", fps[0])],
            "rotate the deploy key",
        )
        .unwrap(),
    );
    assert_eq!(r.session, "claude-code/s1");
    assert_eq!(r.reader, cloud());
    assert_eq!(r.reason, "rotate the deploy key");
    assert_eq!(r.secrets.len(), 1);
    assert_eq!(r.secrets[0].fingerprint.short(), fps[0]);
    assert_eq!(r.secrets[0].kind, crate::detect::Kind::AnthropicKey);
    assert_eq!(r.state, State::Pending);
    let receipt = r.receipt();
    assert!(receipt.contains(&r.id), "{receipt}");
    assert!(receipt.contains("owner"), "{receipt}");
    assert!(!receipt.contains(&fake_key()), "{receipt}");

    // The request is a file under `reveals/`, and the read log has a line for it.
    assert!(home.reveal_path(&r.id).exists());
    let log = log_results(&home);
    let last = log.last().unwrap();
    assert_eq!(last["result"], "reveal_requested");
    assert_eq!(last["request"], r.id.as_str());
    assert_eq!(last["session"], "claude-code/s1");
    assert_eq!(last["provider"], "example-cloud");
    assert_no_value_outside_the_archive(&home);
}

#[test]
fn an_untrusted_or_unnamed_reader_and_a_vague_request_are_refused_and_logged() {
    let (_d, home) = home_with_secrets();
    let rt = runtime();
    let fp = shorts(&home, &rt)[0].clone();
    let ask = |reader: &Reader, session: &str, fps: &[&str], reason: &str| {
        let fps: Vec<String> = fps.iter().map(|s| s.to_string()).collect();
        refused_filing(request(&home, &rt, reader, session, &fps, reason).unwrap())
    };
    let s = "claude-code/s1";
    for (reader, says) in [
        (
            Reader::model(Some("other-cloud"), None),
            "distill providers trust other-cloud",
        ),
        (
            Reader::model(Some("unheard-of"), Some("m")),
            "distill providers add unheard-of",
        ),
        (Reader::model(None, Some("m")), "names no provider"),
        (
            Reader::model(Some("ollama"), Some("big:cloud")),
            "ollama-cloud",
        ),
        (Reader::Owner, "only a model"),
        (Reader::Distill, "only a model"),
    ] {
        let why = ask(&reader, s, &[&fp], "rotate it");
        assert!(why.contains(says), "{reader:?}: {why}");
    }
    let c = cloud();
    let key_as_reason = format!("I need {}", fake_key());
    for (session, fps, reason, says) in [
        (s, &[][..], "rotate it", "at least one secret"),
        (s, &[fp.as_str()], "  ", "give a reason"),
        (s, &[fp.as_str()], key_as_reason.as_str(), "holds a secret"),
        (s, &["zz"], "rotate it", "fingerprint 1 is not one"),
        (
            s,
            &[fp.as_str(), "abc"],
            "rotate it",
            "fingerprint 2 is not one",
        ),
        (
            s,
            &["0000000000"],
            "rotate it",
            "no secret in claude-code/s1",
        ),
        (
            "nothing-here",
            &[fp.as_str()],
            "rotate it",
            "no such session",
        ),
    ] {
        let why = ask(&c, session, fps, reason);
        assert!(why.contains(says), "{fps:?} {reason:?}: {why}");
    }
    // A key named as the session is not written to the log.
    ask(&c, &fake_key(), &[&fp], "rotate it");

    assert!(!home.reveals_dir().exists() || load_all(&home).is_empty());
    let log = log_results(&home);
    assert_eq!(log.len(), 1 + 6 + 7 + 1, "{log:#?}");
    assert!(
        log[1..].iter().all(|l| l["result"] == "refused"),
        "{log:#?}"
    );
    assert_no_value_outside_the_archive(&home);
}

#[test]
fn a_placeholder_or_a_longer_prefix_names_a_secret_too() {
    let (_d, home) = home_with_secrets();
    let rt = runtime();
    let fps = shorts(&home, &rt);
    let r = filed(
        request(
            &home,
            &rt,
            &cloud(),
            "s1",
            &[
                format!("[secret: anthropic_key #{}]", fps[0]),
                fps[1].to_uppercase(),
                fps[1].clone(),
            ],
            "rotate both",
        )
        .unwrap(),
    );
    let got: Vec<&str> = r.secrets.iter().map(|s| s.fingerprint.short()).collect();
    let mut want = vec![fps[0].as_str(), fps[1].as_str()];
    want.sort();
    let mut got_sorted = got.clone();
    got_sorted.sort();
    assert_eq!(got_sorted, want);
}

fn released(c: Collected) -> Released {
    match c {
        Collected::Released(r) => r,
        Collected::Refused(why) => panic!("refused: {why}"),
    }
}

fn refused_collect(c: Collected) -> String {
    match c {
        Collected::Refused(why) => why,
        Collected::Released(r) => panic!("released: {r:?}"),
    }
}

/// A pending request from [`cloud`] for both secrets.
fn file_both(home: &Home, rt: &LocalRuntime) -> Request {
    let fps = shorts(home, rt);
    filed(request(home, rt, &cloud(), "claude-code/s1", &fps, "rotate both").unwrap())
}

#[test]
fn after_approval_the_same_reader_collects_the_values_once() {
    let (_d, home) = home_with_secrets();
    let rt = runtime();
    let r = file_both(&home, &rt);
    assert_eq!(pending(&home).unwrap(), std::slice::from_ref(&r));
    assert_eq!(pending_count(&home), 1);
    assert_eq!(crate::status::status(&home).unwrap().pending_reveals, 1);

    // Before approval: nothing.
    let why = refused_collect(collect(&home, &rt, &cloud(), &r.id).unwrap());
    assert!(why.contains("not approved yet"), "{why}");

    let approved = approve(&home, &r.id).unwrap();
    assert!(matches!(approved.state, State::Approved { .. }));
    assert!(pending(&home).unwrap().is_empty());
    assert_eq!(pending_count(&home), 0);
    assert_eq!(crate::status::status(&home).unwrap().pending_reveals, 0);
    // An answered request can't be answered again.
    assert!(deny(&home, &r.id).is_err());
    assert!(approve(&home, &r.id).is_err());

    // Another reader, even at the same provider, gets nothing.
    for other in [
        Reader::model(Some("example-cloud"), Some("other-model")),
        Reader::model(Some("example-cloud"), None),
        Reader::model(Some("ollama"), Some("small:8b")),
    ] {
        let why = refused_collect(collect(&home, &rt, &other, &r.id).unwrap());
        assert!(why.contains("another reader"), "{other:?}: {why}");
    }
    let why = refused_collect(collect(&home, &rt, &Reader::model(None, None), &r.id).unwrap());
    assert!(why.contains("names no provider"), "{why}");

    // The reader that filed it, its provider in any case: the values, once.
    let same = Reader::model(Some("Example-Cloud"), Some("m"));
    let got = released(collect(&home, &rt, &same, &r.id).unwrap());
    let mut values: Vec<&str> = got.values.iter().map(|v| v.value.as_str()).collect();
    values.sort();
    let (key, password) = (fake_key(), fake_password());
    let mut want = vec![key.as_str(), password.as_str()];
    want.sort();
    assert_eq!(values, want);
    assert!(got.missing.is_empty());
    let text = got.text();
    assert!(
        text.contains(&format!(
            "[secret: anthropic_key #{}] = {key}",
            r.secrets
                .iter()
                .find(|s| s.kind == crate::detect::Kind::AnthropicKey)
                .unwrap()
                .fingerprint
                .short()
        )),
        "{}",
        r.id
    );
    assert!(!format!("{got:?}").contains(&key));

    let why = refused_collect(collect(&home, &rt, &cloud(), &r.id).unwrap());
    assert!(why.contains("already collected"), "{why}");
    assert!(!why.contains(&key));

    let log = log_results(&home);
    let results: Vec<&str> = log.iter().map(|l| l["result"].as_str().unwrap()).collect();
    assert_eq!(
        results,
        [
            "served",
            "reveal_requested",
            "refused",
            "reveal_approved",
            "refused",
            "refused",
            "refused",
            "refused",
            "reveal_collected",
            "refused"
        ]
    );
    let collected = &log[8];
    assert_eq!(collected["request"], r.id.as_str());
    assert_eq!(collected["secrets"], 2);
    assert_eq!(collected["provider"], "Example-Cloud");
    assert_eq!(log[3]["provider"], "example-cloud");
    assert_eq!(log[3]["session"], "claude-code/s1");
    assert_no_value_outside_the_archive(&home);
}

#[test]
fn a_denied_request_releases_nothing() {
    let (_d, home) = home_with_secrets();
    let rt = runtime();
    let r = file_both(&home, &rt);
    let denied = deny(&home, &r.id).unwrap();
    assert!(matches!(denied.state, State::Denied { .. }));
    assert!(approve(&home, &r.id).is_err());
    let why = refused_collect(collect(&home, &rt, &cloud(), &r.id).unwrap());
    assert!(why.contains("denied"), "{why}");
    let log = log_results(&home);
    assert_eq!(log[2]["result"], "reveal_denied");
    for bad in ["nothing", "0123456789abcdef", "../../keys/fingerprint"] {
        let why = refused_collect(collect(&home, &rt, &cloud(), bad).unwrap());
        assert!(why.contains("no such reveal request"), "{why}");
        assert!(approve(&home, bad).is_err());
    }
}

#[test]
fn a_request_expires_24_hours_after_it_was_made_approved_or_not() {
    let (_d, home) = home_with_secrets();
    let rt = runtime();
    let made = Utc::now();
    let fps = shorts(&home, &rt);
    let file = || filed(request_at(&home, &rt, &cloud(), "s1", &fps, "rotate both", made).unwrap());
    let unanswered = file();
    let approved = file();
    let collected = file();
    answer(&home, &approved.id, true, made).unwrap();
    answer(&home, &collected.id, true, made).unwrap();
    released(collect_at(&home, &rt, &cloud(), &collected.id, made).unwrap());

    // Just before: still pending, still collectable.
    let almost = made + Duration::hours(24) - Duration::seconds(1);
    assert_eq!(pending_at(&home, almost).unwrap().len(), 1);

    let later = made + Duration::hours(24);
    assert!(pending_at(&home, later).unwrap().is_empty());
    let err = answer(&home, &unanswered.id, true, later).unwrap_err();
    assert!(format!("{err}").contains("expired"), "{err}");
    let why = refused_collect(collect_at(&home, &rt, &cloud(), &approved.id, later).unwrap());
    assert!(why.contains("expired"), "{why}");
    // A collected request is done, and does not expire.
    let all = list_at(&home, later).unwrap();
    let state = |id: &str| all.iter().find(|r| r.id == id).unwrap().state.name();
    assert_eq!(state(&unanswered.id), "expired");
    assert_eq!(state(&approved.id), "expired");
    assert_eq!(state(&collected.id), "collected");

    // Each expiry is logged once.
    let log = log_results(&home);
    let expired: Vec<_> = log
        .iter()
        .filter(|l| l["result"] == "reveal_expired")
        .map(|l| l["request"].as_str().unwrap().to_string())
        .collect();
    let mut want = vec![unanswered.id.clone(), approved.id.clone()];
    want.sort();
    let mut got = expired.clone();
    got.sort();
    assert_eq!(got, want);
    assert!(
        log.iter()
            .all(|l| l["provider"] == "example-cloud" || l["result"] == "served")
    );
}
