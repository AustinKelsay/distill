//! Made-up findings, reveal requests and read-log lines for the Findings, Approvals and Read log
//! scenes. Nothing here is a value: a fingerprint is made of a repeated hex pair, the readers
//! are at an example provider, and the sessions are invented.

use distill_core::Harness;
use distill_core::detect::Kind;
use distill_core::findings::Fingerprint;
use distill_core::read::{Appearance, Appearances, Fingerprints, Lookup, Spread};
use distill_core::readlog::{ReadEntry, ReadResult};
use distill_core::reveal::{Request, Secret, State as Asked};
use distill_core::trust::Reader;

use super::{at, clock, running};
use crate::State;
use crate::nav::Screen;

fn fingerprint(pair: &str) -> Fingerprint {
    serde_json::from_value(serde_json::json!(pair.repeat(32))).expect("a fingerprint")
}

fn day(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    Some(
        chrono::DateTime::parse_from_rfc3339(s)
            .expect("a time")
            .with_timezone(&chrono::Utc),
    )
}

fn spread(
    pair: &str,
    kinds: &[Kind],
    sessions: u64,
    harnesses: &[Harness],
    first: &str,
    last: &str,
) -> Spread {
    Spread {
        fingerprint: fingerprint(pair),
        kinds: kinds.to_vec(),
        sessions,
        harnesses: harnesses.to_vec(),
        first_seen: day(first),
        last_seen: day(last),
    }
}

fn rotation_list() -> Fingerprints {
    let rows = vec![
        spread(
            "a1",
            &[Kind::AnthropicKey],
            4,
            &[Harness::ClaudeCode, Harness::Codex],
            "2026-09-02T09:14:00Z",
            "2026-11-14T17:40:00Z",
        ),
        spread(
            "b2",
            &[Kind::GithubToken],
            2,
            &[Harness::ClaudeCode],
            "2026-10-20T08:00:00Z",
            "2026-11-03T12:30:00Z",
        ),
        spread(
            "c3",
            &[Kind::AssignedSecret],
            1,
            &[Harness::Codex],
            "2026-11-12T10:00:00Z",
            "2026-11-12T10:45:00Z",
        ),
        spread(
            "d4",
            &[Kind::Bearer, Kind::CredentialField],
            1,
            &[Harness::T3Code],
            "2026-08-30T16:05:00Z",
            "2026-08-30T16:20:00Z",
        ),
    ];
    Fingerprints {
        total: rows.len() as u64,
        rows,
        next: None,
        pending: 3,
    }
}

fn on(screen: Screen) -> State {
    let mut s = running();
    s.screen = screen;
    s
}

/// The rotation list: secrets by fingerprint, most sessions first.
pub fn findings_listed() -> State {
    let mut s = on(Screen::Findings);
    s.findings.list = Some(rotation_list());
    s
}

/// The rotation list with its first fingerprint pressed: the sessions it appears in.
pub fn findings_where() -> State {
    let mut s = findings_listed();
    let fp = fingerprint("a1");
    let place = |session: &str, harness, project: &str, started: &str, events: &[u64]| Appearance {
        session: session.into(),
        harness,
        project: Some(project.into()),
        started_at: day(started),
        ended_at: day(started),
        events: events.to_vec(),
    };
    s.findings.selected = Some(fp.as_str().to_string());
    s.findings.lookup = Some(Lookup {
        asked: fp.as_str().to_string(),
        found: vec![Appearances {
            fingerprint: fp,
            kinds: vec![Kind::AnthropicKey],
            sessions: vec![
                place(
                    "claude-code/5d1f0c2a",
                    Harness::ClaudeCode,
                    "/work/app",
                    "2026-11-14T17:02:00Z",
                    &[0, 14],
                ),
                place(
                    "codex/rollout-0042",
                    Harness::Codex,
                    "/work/app",
                    "2026-11-02T09:30:00Z",
                    &[3],
                ),
                place(
                    "claude-code/9e8a7b10",
                    Harness::ClaudeCode,
                    "/work/infra",
                    "2026-10-11T13:15:00Z",
                    &[7],
                ),
                place(
                    "claude-code/1a2b3c4d",
                    Harness::ClaudeCode,
                    "/work/app",
                    "2026-09-02T09:14:00Z",
                    &[1, 2, 9],
                ),
            ],
        }],
        pending: 3,
    });
    s
}

fn cloud(model: &str) -> Reader {
    Reader::model(Some("example-cloud"), Some(model))
}

fn request(
    id: &str,
    reader: Reader,
    minutes_ago: i64,
    session: &str,
    secrets: &[(&str, Kind)],
    reason: &str,
) -> Request {
    let expires = clock() + chrono::Duration::minutes(24 * 60 - minutes_ago);
    Request {
        id: id.into(),
        made_at: at(minutes_ago),
        expires_at: expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        reader,
        session: session.into(),
        secrets: secrets
            .iter()
            .map(|(pair, kind)| Secret {
                fingerprint: fingerprint(pair),
                kind: *kind,
            })
            .collect(),
        reason: reason.into(),
        state: Asked::Pending,
    }
}

pub const FIRST_REQUEST: &str = "3f9a1c0e7b2d4a68";

fn requests() -> Vec<Request> {
    vec![
        request(
            FIRST_REQUEST,
            cloud("model-a"),
            5,
            "claude-code/5d1f0c2a",
            &[("a1", Kind::AnthropicKey)],
            "The deploy script fails with 401. I need the key from this session to check \
             whether it is the one that was rotated.",
        ),
        request(
            "9b2e6d40c1f37a85",
            cloud("model-b"),
            190,
            "codex/rollout-0042",
            &[("b2", Kind::GithubToken), ("c3", Kind::AssignedSecret)],
            "Re-run the release job with the token and password set in this session.",
        ),
    ]
}

/// Two reveal requests waiting, as the overview's count says.
pub fn approvals_waiting() -> State {
    let mut s = on(Screen::Approvals);
    s.approvals.requests = Some(requests());
    s
}

/// The first request after one press on Approve: what will be released, waiting for the second.
pub fn approvals_confirming() -> State {
    let mut s = approvals_waiting();
    s.approvals.confirming = Some(FIRST_REQUEST.into());
    s
}

fn entry(minutes_ago: i64, reader: Reader, session: Option<&str>, result: ReadResult) -> ReadEntry {
    ReadEntry {
        at: at(minutes_ago),
        reader,
        session: session.map(str::to_string),
        result,
    }
}

/// The read log: reads, a refusal and the steps of reveal requests, newest first.
pub fn read_log_listed() -> State {
    let mut s = on(Screen::ReadLog);
    let req = |id: &str| id.to_string();
    s.read_log.entries = Some(vec![
        entry(
            1,
            Reader::Owner,
            Some("claude-code/5d1f0c2a"),
            ReadResult::Served { events: 42 },
        ),
        entry(
            2,
            Reader::Owner,
            None,
            ReadResult::Findings {
                fingerprints: 4,
                sessions: 0,
            },
        ),
        entry(
            5,
            cloud("model-a"),
            Some("claude-code/5d1f0c2a"),
            ReadResult::RevealRequested {
                request: req(FIRST_REQUEST),
                secrets: vec!["a1a1a1a1".into()],
            },
        ),
        entry(
            6,
            cloud("model-a"),
            Some("claude-code/5d1f0c2a"),
            ReadResult::Served { events: 42 },
        ),
        entry(
            12,
            Reader::Distill,
            None,
            ReadResult::Indexed {
                sessions: 3,
                events: 214,
            },
        ),
        entry(
            40,
            Reader::model(Some("unknown-cloud"), Some("x")),
            Some("codex/rollout-0042"),
            ReadResult::Refused {
                reason: "unknown-cloud is not a provider Distill knows: add it with `distill providers add unknown-cloud`".into(),
            },
        ),
        entry(
            95,
            cloud("model-b"),
            Some("claude-code/9e8a7b10"),
            ReadResult::RevealCollected {
                request: req("77c0e1d2a3b4c5d6"),
                secrets: 1,
            },
        ),
        entry(
            101,
            cloud("model-b"),
            Some("claude-code/9e8a7b10"),
            ReadResult::RevealApproved {
                request: req("77c0e1d2a3b4c5d6"),
            },
        ),
        entry(
            130,
            cloud("model-b"),
            Some("claude-code/1a2b3c4d"),
            ReadResult::RevealDenied {
                request: req("0e4d5c6b7a8f9012"),
            },
        ),
        entry(
            190,
            cloud("model-b"),
            Some("codex/rollout-0042"),
            ReadResult::RevealRequested {
                request: req("9b2e6d40c1f37a85"),
                secrets: vec!["b2b2b2b2".into(), "c3c3c3c3".into()],
            },
        ),
        entry(
            300,
            Reader::Owner,
            None,
            ReadResult::Listed { sessions: 50 },
        ),
    ]);
    s.read_log.limit = crate::screens::read_log::PAGE;
    s
}
