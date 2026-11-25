//! Fixture state for scenes (`--snap --scene`) and the headless tests. Everything here is made
//! up; nothing touches disk, and no path or name is anyone's.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use distill_core::exposure::{Exposure, LeftFolder, Outcome, Rule, RuleResult};
use distill_core::harness::Detection;
use distill_core::runtime::{Model, RuntimeState, Stored};
use distill_core::status::{HarnessStatus, Health, Status, Warning, WarningKind};
use distill_core::{Config, Harness, Home};

use crate::State;

mod reveals;
mod sessions;
pub use reveals::*;
pub use sessions::*;

/// Named fixture states for `--snap --scene`; the snapshot tests render each.
/// A named state for `--snap --scene`.
pub type Scene = (&'static str, fn(&mut State));

pub const SCENES: &[Scene] = &[
    ("guide", |s| *s = base(None)),
    ("setup-harnesses", |s| setup_step(s, 0)),
    ("setup-home", |s| setup_step(s, 1)),
    ("setup-providers", |s| setup_step(s, 2)),
    ("setup-models", |s| setup_step(s, 3)),
    ("setup-models-service", models_at_a_service),
    ("setup-privacy", |s| setup_step(s, 4)),
    ("setup-backups", |s| setup_step(s, 5)),
    ("setup-backups-key", backups_key_shown),
    ("setup-backups-empty", backups_empty),
    ("setup-backups-new-key", backups_new_key),
    ("setup-schedule", |s| setup_step(s, 6)),
    ("overview", |s| *s = running()),
    ("overview-warnings", |s| *s = troubled()),
    ("overview-new", |s| *s = just_set_up()),
    ("sessions", |s| *s = sessions_listed()),
    ("sessions-search", |s| *s = sessions_searched()),
    ("sessions-open", |s| *s = session_covered()),
    ("sessions-open-shown", |s| *s = session_revealed()),
    ("findings", |s| *s = findings_listed()),
    ("findings-where", |s| *s = findings_where()),
    ("approvals", |s| *s = approvals_waiting()),
    ("approvals-confirm", |s| *s = approvals_confirming()),
    ("read-log", |s| *s = read_log_listed()),
];

/// 2026-11-15 12:00 UTC: "now" in every fixture.
pub fn clock() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
        .expect("valid time")
        .with_timezone(&Utc)
}

pub(crate) fn at(minutes_ago: i64) -> String {
    (clock() - chrono::Duration::minutes(minutes_ago))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A config whose harness roots are written as `~/.<id>`, so nothing depends on this machine.
pub fn config() -> Config {
    let mut c = Config::default();
    for h in &mut c.harnesses {
        h.roots = vec![PathBuf::from(format!("~/.{}", h.kind.id()))];
    }
    c
}

fn base(config: Option<Config>) -> State {
    let mut s = State::new(Home::new("~/distill"), config);
    s.fixture = true;
    s.clock = Some(clock());
    s
}

/// Nothing set up yet, on Setup (the window itself opens on the Guide), with harnesses detected
/// and Ollama running.
pub fn fresh() -> State {
    let mut s = base(None);
    s.screen = crate::nav::Screen::Setup;
    s.setup.draft = config();
    s.setup.detected(detections());
    s.setup.runtime = Some(runtime_state());
    s.setup.timer_installed = Some(false);
    s
}

fn setup_step(s: &mut State, step: usize) {
    *s = fresh();
    s.setup.step = step;
    if step == 2 {
        distill_core::trust::add(&mut s.setup.draft, "example-cloud", true).expect("valid name");
    }
    if step == 3 {
        s.setup.draft.steward.card_model = Some("qwen-small:4b".into());
        s.setup.draft.steward.embedding_model = Some("embed-text:v1".into());
    }
    if step == 4 {
        s.setup.draft.privacy.personal_terms = vec!["Northwind".into(), "Jo Example".into()];
        s.setup.draft.privacy.exclude_paths = vec!["~/work/client-x".into()];
    }
    if step == 5 {
        s.setup.draft.backups = backup_targets();
        distill_core::backup::add_folder(&mut s.setup.draft, "Google Drive", GOOGLE_DRIVE)
            .expect("valid target");
        s.setup.places = Some(places());
        s.setup.tests.insert(
            "drive".into(),
            Ok("Ready. Nothing is copied until the first backup.".into()),
        );
        s.setup.tests.insert(
            "share".into(),
            Err("the backup folder /Volumes/Share does not exist (is the drive connected?)".into()),
        );
        s.setup.has_key = Some(false);
    }
}

const GOOGLE_DRIVE: &str = "/Users/you/Library/CloudStorage/GoogleDrive-you@example.com/My Drive";

/// What a Mac with an external drive, Google Drive and iCloud Drive offers as backup folders.
fn places() -> Vec<distill_core::backup::Place> {
    use distill_core::backup::Place;
    vec![
        Place {
            name: "Backup".into(),
            path: "/Volumes/Backup".into(),
            cloud: false,
        },
        Place {
            name: "Google Drive".into(),
            path: GOOGLE_DRIVE.into(),
            cloud: true,
        },
        Place {
            name: "iCloud Drive".into(),
            path: "/Users/you/Library/Mobile Documents/com~apple~CloudDocs".into(),
            cloud: true,
        },
    ]
}

/// The backups step as a new owner first sees it: no folder and no backup key yet, with the
/// places this machine offers.
fn backups_empty(s: &mut State) {
    setup_step(s, 5);
    s.setup.draft.backups.clear();
    s.setup.tests.clear();
}

/// Two folder targets: an external drive and a mounted share.
fn backup_targets() -> Vec<distill_core::config::BackupTarget> {
    let mut c = Config::default();
    distill_core::backup::add_folder(&mut c, "drive", "/Volumes/Backup").expect("valid target");
    distill_core::backup::add_folder(&mut c, "share", "/Volumes/Share").expect("valid target");
    c.backups
}

/// The backups step just after the backup key was made, the key shown. The key is
/// made up and opens nothing.
fn backups_key_shown(s: &mut State) {
    setup_step(s, 5);
    s.setup.has_key = Some(true);
    s.setup.recovery = Some(format!("AGE-SECRET-KEY-1{}", "EXAMPLE".repeat(9)));
}

/// The backups step with a backup key, asking before it makes a new one.
fn backups_new_key(s: &mut State) {
    setup_step(s, 5);
    s.setup.has_key = Some(true);
    s.setup.confirm_new_key = true;
}

/// The models step with cards and detection run at a trusted service that listed its models.
fn models_at_a_service(s: &mut State) {
    setup_step(s, 3);
    distill_core::trust::add(&mut s.setup.draft, "example-cloud", true).expect("valid name");
    s.setup.draft.steward.generation = Some(distill_core::config::GenerationConfig {
        provider: "example-cloud".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key_file: "/keys/example-cloud".into(),
    });
    s.setup.base_url.replace_text("https://api.example.com/v1");
    s.setup.key_file.replace_text("/keys/example-cloud");
    s.setup.remote_models = Some(Ok(vec!["large-model".into(), "small-model".into()]));
    s.setup.draft.steward.card_model = Some("large-model".into());
}

pub fn detections() -> Vec<Detection> {
    let found = [
        (Harness::ClaudeCode, true, 1_840_000_000, 4_212),
        (Harness::Codex, true, 642_000_000, 1_103),
        (Harness::OpenCode, true, 92_400_000, 1),
        (Harness::Droid, false, 0, 0),
        (Harness::Pi, true, 12_300_000, 48),
        (Harness::Cursor, false, 0, 0),
        (Harness::Grok, false, 0, 0),
        (Harness::Antigravity, false, 0, 0),
        (Harness::Hermes, false, 0, 0),
        (Harness::T3Code, true, 31_000_000, 220),
        (Harness::Goose, false, 0, 0),
    ];
    found
        .into_iter()
        .map(|(kind, found, bytes, files)| Detection {
            kind,
            roots: vec![PathBuf::from(format!("~/.{}", kind.id()))],
            found,
            bytes,
            files,
            has_parser: kind.has_parser(),
        })
        .collect()
}

pub fn runtime_state() -> RuntimeState {
    RuntimeState::Running(vec![
        Model {
            name: "qwen-small:4b".into(),
            stored: Stored::Here,
        },
        Model {
            name: "embed-text:v1".into(),
            stored: Stored::Here,
        },
        Model {
            name: "big-model:cloud".into(),
            stored: Stored::Cloud,
        },
    ])
}

#[allow(clippy::too_many_arguments)] // one status row, field by field
fn harness(
    kind: Harness,
    health: Health,
    sessions: u64,
    captures: u64,
    bytes: u64,
    minutes_ago: Option<i64>,
    drift: u64,
    note: Option<&str>,
) -> HarnessStatus {
    HarnessStatus {
        harness: kind,
        label: kind.label().to_string(),
        enabled: health != Health::Disabled,
        health,
        sessions,
        captures,
        archive_bytes: bytes,
        last_capture: minutes_ago.map(at),
        capture_errors: 0,
        parse_failures: drift,
        drift,
        partial_drift: 0,
        unknown_types: Default::default(),
        note: note.map(str::to_string),
    }
}

fn exposure(owner_only: bool) -> Exposure {
    let rule = |rule: Rule, outcome: Outcome, message: &str, paths: &[&str]| RuleResult {
        rule,
        outcome,
        message: message.into(),
        paths: paths.iter().map(|p| p.to_string()).collect(),
    };
    Exposure {
        rules: vec![
            if owner_only {
                rule(
                    Rule::OwnerOnly,
                    Outcome::Pass,
                    "every file and folder in the home is owner-only",
                    &[],
                )
            } else {
                rule(
                    Rule::OwnerOnly,
                    Outcome::Fail,
                    "2 files in the home can be read by others: run `distill exposure --fix`",
                    &["~/distill/config.toml 0644", "~/distill/runs 0755"],
                )
            },
            rule(
                Rule::KeyOwnerOnly,
                Outcome::Pass,
                "the fingerprint key is owner-only",
                &[],
            ),
            rule(
                Rule::NotSynced,
                Outcome::Pass,
                "the home is not in a synced folder",
                &[],
            ),
            rule(
                Rule::NoListener,
                Outcome::Unchecked,
                "lsof is not available",
                &[],
            ),
        ],
        disk_encrypted: Some(true),
        backups: "Backups carry the archive, every secret in it included: keep them encrypted too."
            .into(),
        previous_build: if owner_only {
            vec![]
        } else {
            vec![LeftFolder {
                path: "~/distill/exports".into(),
                mode: "0755".into(),
                owner_only: false,
                holds: "dataset builds: session text".into(),
            }]
        },
    }
}

/// A status report assembled from parts. Fields a later ticket adds take their defaults.
fn status(
    harnesses: Vec<HarnessStatus>,
    warnings: Vec<Warning>,
    last_sweep: Option<String>,
    extra: serde_json::Value,
    exposure: Exposure,
) -> Status {
    let sessions: u64 = harnesses.iter().map(|h| h.sessions).sum();
    let bytes: u64 = harnesses.iter().map(|h| h.archive_bytes).sum();
    let mut v = serde_json::json!({
        "configured": true,
        "sessions": sessions,
        "archive_bytes": bytes,
        "last_sweep": last_sweep,
        "harnesses": harnesses,
        "warnings": warnings,
        "scan": { "sessions": sessions, "current": sessions, "pending": 0, "secrets": 0 },
        "exposure": exposure,
    });
    for (k, x) in extra.as_object().expect("an object") {
        v[k] = x.clone();
    }
    serde_json::from_value(v).expect("a status report")
}

fn all_harnesses(rows: Vec<HarnessStatus>) -> Vec<HarnessStatus> {
    Harness::ALL
        .into_iter()
        .map(|k| {
            rows.iter()
                .find(|h| h.harness == k)
                .cloned()
                .unwrap_or_else(|| harness(k, Health::Disabled, 0, 0, 0, None, 0, None))
        })
        .collect()
}

/// A healthy archive a day into its card backlog, with two reveal requests waiting.
pub fn running() -> State {
    let mut s = base(Some(config()));
    s.overview.status = Some(status(
        all_harnesses(vec![
            harness(
                Harness::ClaudeCode,
                Health::Ok,
                4_118,
                4_212,
                1_840_000_000,
                Some(12),
                0,
                None,
            ),
            harness(
                Harness::Codex,
                Health::Ok,
                1_090,
                1_103,
                642_000_000,
                Some(12),
                0,
                None,
            ),
            harness(
                Harness::OpenCode,
                Health::Ok,
                61,
                1,
                92_400_000,
                Some(75),
                0,
                None,
            ),
            harness(
                Harness::Pi,
                Health::Ok,
                46,
                48,
                12_300_000,
                Some(2_900),
                0,
                None,
            ),
            harness(
                Harness::T3Code,
                Health::Ok,
                214,
                220,
                31_000_000,
                Some(12),
                0,
                None,
            ),
            harness(
                Harness::Droid,
                Health::Missing,
                0,
                0,
                0,
                None,
                0,
                Some("not found on this machine"),
            ),
        ]),
        vec![],
        Some(at(12)),
        serde_json::json!({
            "scan": { "sessions": 5_529, "current": 5_529, "pending": 0, "secrets": 37 },
            "index": { "sessions": 5_529, "current": 5_490, "pending": 39, "segments": 4, "bytes": 212_000_000 },
            "cards": { "model": "qwen-small:4b", "sessions": 5_300, "current": 1_212, "pending": 4_088, "failing": 6 },
            "vectors": { "model": "embed-text:v1", "sessions": 5_300, "current": 900, "pending": 4_400, "vectors": 1_800, "bytes": 5_500_000 },
            "details": { "model": "qwen-small:4b", "sessions": 5_529, "current": 640, "pending": 4_889, "failing": 2, "found": 88 },
            "pending_reveals": 2,
            "runs": runs(),
            "backups": [
                { "name": "drive", "kind": "folder", "last_ok": at(40), "captures_behind": 0 },
                { "name": "share", "kind": "folder", "last_ok": at(1_500), "captures_behind": 212 },
            ],
        }),
        exposure(true),
    ));
    if let Some(c) = s.config.as_mut() {
        c.backups = backup_targets();
    }
    s
}

/// A run history: the cards step reaching its time limit most hours, one tick whose card
/// provider did not answer, and a sweep run on its own.
fn runs() -> serde_json::Value {
    let steps = |odd: &[(&str, &str)]| {
        [
            "sweep", "parse", "scan", "index", "cards", "vectors", "details",
        ]
        .map(|name| {
            let result = odd
                .iter()
                .find(|(n, _)| *n == name)
                .map_or("ok", |(_, r)| r);
            serde_json::json!({ "name": name, "result": result })
        })
        .to_vec()
    };
    serde_json::json!([
        { "newest": at(40), "oldest": at(40), "runs": 1, "step": "tick",
          "steps": steps(&[("cards", "stopped")]) },
        { "newest": at(100), "oldest": at(400), "runs": 6, "step": "tick",
          "steps": steps(&[("cards", "stopped"), ("details", "stopped")]) },
        { "newest": at(460), "oldest": at(460), "runs": 1, "step": "tick",
          "steps": steps(&[("cards", "failed"), ("details", "failed")]),
          "failures": [
            "cards failed: the provider did not answer within 120 seconds",
            "details failed: the provider did not answer within 120 seconds",
          ] },
        { "newest": at(520), "oldest": at(1_900), "runs": 24, "step": "tick", "steps": steps(&[]) },
        { "newest": at(2_000), "oldest": at(2_000), "runs": 1, "step": "sweep",
          "steps": [{ "name": "sweep", "result": "ok" }] },
    ])
}

/// An archive that needs the owner: a sweep overdue, a harness pruning early, drift, and
/// files others can read.
pub fn troubled() -> State {
    let mut s = running();
    let st = s.overview.status.as_mut().expect("a status");
    st.last_sweep = Some(at(60 * 7));
    for h in &mut st.harnesses {
        if h.harness == Harness::Codex {
            h.health = Health::Warn;
            h.parse_failures = 14;
            h.drift = 12;
            h.note = Some("12 look like format drift".into());
        }
        if h.harness == Harness::ClaudeCode {
            h.health = Health::Warn;
            h.note = Some("deletes sessions sooner than the sweep runs".into());
        }
    }
    st.warnings = vec![
        Warning {
            kind: WarningKind::SweepOverdue,
            harness: None,
            message: "Nothing has swept for 7 hours, and the timer runs every 60 minutes: check that it is installed and running (`distill schedule`).".into(),
        },
        Warning {
            kind: WarningKind::PrunesSooner,
            harness: Some(Harness::ClaudeCode),
            message: "Claude Code deletes sessions untouched for 30 minutes (cleanupPeriodDays), sooner than the timer sweeps (every 60 minutes): raise the setting or sweep more often.".into(),
        },
        Warning {
            kind: WarningKind::Exposure,
            harness: None,
            message: "Exposure: 2 files in the home can be read by others: run `distill exposure --fix`".into(),
        },
    ];
    st.warnings.push(Warning {
        kind: WarningKind::Backup,
        harness: None,
        message: "The last backup to share failed, and its last backup was 25 hours ago: the backup folder /Volumes/Share does not exist (is the drive connected?)".into(),
    });
    let share = &mut st.backups[1];
    share.last_failed = Some(at(40));
    share.last_error =
        Some("the backup folder /Volumes/Share does not exist (is the drive connected?)".into());
    st.exposure = exposure(false);
    st.cards.skipped = Some("the local runtime is not running".into());
    s
}

/// Set up a minute ago: nothing swept, scanned, indexed or carded yet.
pub fn just_set_up() -> State {
    let mut s = base(Some(config()));
    let mut c = config();
    c.steward.card_model = None;
    s.overview.status = Some(status(
        all_harnesses(vec![
            harness(Harness::ClaudeCode, Health::Ok, 0, 0, 0, None, 0, None),
            harness(Harness::Codex, Health::Ok, 0, 0, 0, None, 0, None),
        ]),
        vec![],
        None,
        serde_json::json!({
            "cards": { "skipped": "no card_model is set", "sessions": 0, "current": 0, "pending": 0, "failing": 0 },
        }),
        exposure(true),
    ));
    s.config = Some(c);
    s
}
