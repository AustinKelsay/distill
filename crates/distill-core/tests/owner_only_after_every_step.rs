//! Everything DISTILL writes is owner-only even with a umask that would allow anything (#90),
//! including what only the model steps and reveals write: cards, vectors, the steps' failure
//! files and reveal requests, which a run of the binary with no model never makes. Its own test
//! binary, since the umask is per process.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use distill_core::archive::Archive;
use distill_core::exposure::{self, Outcome, Rule};
use distill_core::reveal::Filed;
use distill_core::runtime::{FakeRuntime, Stored};
use distill_core::trust::{self, Reader};
use distill_core::{Harness, Home};
use serde_json::Value;

#[test]
fn every_file_the_model_steps_and_a_reveal_write_is_owner_only() {
    // SAFETY: this test binary runs this one test, so nothing else reads the umask meanwhile.
    unsafe { libc::umask(0) };

    let key = ["sk-", "ant-", "api03-", &"Pq7".repeat(9)].concat();
    let said = serde_json::to_string(&format!("deploy with {key} and tell Jane Roe")).unwrap();
    let session = format!(
        r#"{{"type":"user","cwd":"/work/app","timestamp":"2026-11-10T10:00:00Z","message":{{"role":"user","content":{said}}}}}
{{"type":"assistant","timestamp":"2026-11-10T10:00:05Z","message":{{"role":"assistant","model":"m","content":"Deployed."}}}}
"#
    );

    let tmp = tempfile::tempdir().unwrap();
    let nest = tmp.path().join("nest");
    // Checked after every step, so a file a later step rewrites is checked as first written.
    let owner_only = |step: &str| {
        let open = loose(&nest);
        assert!(
            open.is_empty(),
            "after {step}, not owner-only:\n{}",
            open.join("\n")
        );
    };
    // Folders made on the way to the home count too.
    let home = Home::new(tmp.path().join("nest/distill"));
    home.ensure().unwrap();
    Archive::open(&home)
        .unwrap()
        .put(
            Harness::ClaudeCode,
            "/s/proj/s1.jsonl",
            "1",
            session.as_bytes(),
        )
        .unwrap();
    distill_core::normalize::normalize(&home, false).unwrap();
    let mut config = home.load_config().unwrap_or_default();
    distill_core::findings::scan_all(&home, &config).unwrap();
    owner_only("scan");
    trust::add(&mut config, "example-cloud", true).unwrap();
    config.steward.card_model = Some("small:8b".into());
    config.steward.embedding_model = Some("embed:small".into());
    config.save(&home.config_path()).unwrap();

    let model = |reply: &'static str| {
        FakeRuntime::running()
            .with_model("small:8b", Stored::Here)
            .with_model("embed:small", Stored::Here)
            .replying(move |_, _| reply.to_string())
    };
    // A failure first, so each step's failure file is written, then a usable answer.
    let r = distill_core::cards::write(
        &home,
        &config,
        &model("no card").runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.failed, 1, "{r:?}");
    owner_only("a card that failed");
    let card = r#"{"title":"Deploy the app","summary":"It was deployed.","tags":["deploy"],"outcome":"done"}"#;
    let r = distill_core::cards::write(
        &home,
        &config,
        &model(card).runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.written, 1, "{r:?}");
    owner_only("a card");
    let r = distill_core::details::find(
        &home,
        &config,
        &model("no details").runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.failed, 1, "{r:?}");
    owner_only("details that failed");
    let found = r#"{"details":[{"kind":"person_name","text":"Jane Roe"}]}"#;
    let r = distill_core::details::find(
        &home,
        &config,
        &model(found).runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert_eq!(r.found, 1, "{r:?}");
    owner_only("details");
    distill_core::index::build(&home).unwrap();
    distill_core::index::vectors::build(
        &home,
        &config,
        &model(card).runtime(),
        distill_core::tick::Deadline::NONE,
    )
    .unwrap();
    assert!(home.vectors_path().exists());
    owner_only("the index and vectors");

    // A reveal, requested, approved and collected.
    let findings: Value = serde_json::from_str(
        &fs::read_to_string(home.findings_path(Harness::ClaudeCode, "s1")).unwrap(),
    )
    .unwrap();
    let fingerprint = findings["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "anthropic_key")
        .unwrap()["fingerprint"]
        .as_str()
        .unwrap()[..8]
        .to_string();
    let reader = Reader::model(Some("example-cloud"), Some("m"));
    let stopped = FakeRuntime::stopped().runtime();
    let filed = match distill_core::reveal::request(
        &home,
        &stopped,
        &reader,
        "claude-code/s1",
        &[fingerprint],
        "rotate it",
    )
    .unwrap()
    {
        Filed::Filed(r) => r,
        Filed::Refused(why) => panic!("refused: {why}"),
    };
    owner_only("a reveal request");
    distill_core::reveal::approve(&home, &filed.id).unwrap();
    owner_only("approving it");
    distill_core::reveal::collect(&home, &stopped, &reader, &filed.id).unwrap();

    for made in [
        home.card_path(Harness::ClaudeCode, "s1"),
        home.vectors_path(),
        home.card_failures_path(),
        home.detail_failures_path(),
        home.reveal_path(&filed.id),
    ] {
        assert!(made.exists(), "{} was not made", made.display());
    }
    owner_only("collecting it");
    let check = exposure::check(&home);
    let owner_only = check
        .rules
        .iter()
        .find(|r| r.rule == Rule::OwnerOnly)
        .unwrap();
    assert_eq!(owner_only.outcome, Outcome::Pass, "{owner_only:?}");
}

/// Every file and folder under `dir`, `dir` included, with a group or other permission bit.
fn loose(dir: &Path) -> Vec<String> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .map(|e| e.unwrap())
        .filter(|e| !e.path_is_symlink())
        .filter_map(|e| {
            let mode = e.metadata().unwrap().permissions().mode() & 0o777;
            (mode & 0o077 != 0).then(|| format!("{:o} {}", mode, e.path().display()))
        })
        .collect()
}
