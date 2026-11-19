//! Made-up sessions for the Sessions scenes and tests. Every value is synthetic: the secrets are
//! built from pieces at run time and named FAKE, the people and places are examples, and the
//! findings are placed by hand rather than detected.

use distill_core::Harness;
use distill_core::cards::{Card, Outcome, Shown};
use distill_core::detect::Kind;
use distill_core::findings::{CardFinding, Finding};
use distill_core::model::{Event, EventKind, SessionMeta};
use distill_core::read::{Excerpt, Opening, Passage, Read};
use serde_json::json;

use crate::State;
use crate::nav::{self, Screen};
use crate::screens::sessions::{self, ResultRow, Results};

/// A made-up token, in no real credential's shape.
pub fn fake_token() -> String {
    ["FAKE", "-TOKEN-", "0000", "-NOT-REAL"].concat()
}

pub fn fake_bearer() -> String {
    ["fake", "Bearer", "Value", "0000"].concat()
}

pub fn fake_email() -> String {
    ["jo.example", "@", "example.com"].concat()
}

pub fn fake_host() -> String {
    ["build-box", ".tail0000", ".ts.net"].concat()
}

const PERSON: &str = "Avery Example";

fn fingerprint(n: u8) -> distill_core::findings::Fingerprint {
    serde_json::from_value(json!(format!("{:02x}", n).repeat(32))).expect("a fingerprint")
}

/// The byte range of `value` in `text`.
fn at(text: &str, value: &str) -> (usize, usize) {
    let s = text
        .find(value)
        .unwrap_or_else(|| panic!("{value:?} in {text:?}"));
    (s, s + value.len())
}

fn finding(event: u64, text: &str, value: &str, kind: Kind, n: u8) -> Finding {
    let (start, end) = at(text, value);
    serde_json::from_value(json!({
        "event": event, "field": "text", "start": start, "end": end,
        "kind": kind, "fingerprint": fingerprint(n),
    }))
    .expect("a finding")
}

fn input_finding(event: u64, path: &[usize], s: &str, value: &str, kind: Kind, n: u8) -> Finding {
    let (start, end) = at(s, value);
    serde_json::from_value(json!({
        "event": event, "field": "input", "path": path, "start": start, "end": end,
        "kind": kind, "fingerprint": fingerprint(n),
    }))
    .expect("a finding")
}

fn facts(
    harness: Harness,
    id: &str,
    cwd: &str,
    model: &str,
    minutes_ago: i64,
    events: u64,
) -> SessionMeta {
    let t = (super::clock() - chrono::Duration::minutes(minutes_ago))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    serde_json::from_value(json!({
        "harness": harness, "session_id": id, "cwd": cwd, "models": [model],
        "started_at": t, "ended_at": t,
        "counts": { "events": events, "user": 1, "assistant": 1, "tool_calls": 1, "tool_errors": 0 },
        "raw": { "origin": "/example/session.jsonl", "sha256": "0".repeat(64), "size": 1 },
        "normalizer": { "name": harness.id(), "version": 1 },
    }))
    .expect("session facts")
}

fn card(session: &str, title: &str, summary: &str, tags: &[&str], outcome: Outcome) -> Card {
    serde_json::from_value(json!({
        "format": 1, "session": session, "title": title, "summary": summary, "tags": tags,
        "outcome": outcome, "model": "qwen-small:4b", "prompt_version": 1,
        "written_at": "2026-11-15T11:00:00Z",
        "made_from": { "capture_hash": "0".repeat(64), "parser": 1, "secrets": "" },
    }))
    .expect("a card")
}

fn card_finding(c: &Card, field: &str, value: &str, kind: Kind, n: u8) -> CardFinding {
    let text = match field {
        "title" => c.title.as_str(),
        "summary" => c.summary.as_str(),
        f => {
            let i: usize = f.trim_start_matches("tags/").parse().expect("a tag");
            c.tags[i].as_str()
        }
    };
    let (start, end) = at(text, value);
    CardFinding {
        field: field.into(),
        start,
        end,
        kind,
        fingerprint: fingerprint(n),
    }
}

/// The three rows of the list.
fn rows() -> Vec<ResultRow> {
    let c1 = card(
        "claude-code/s-deploy",
        &format!("Deploy the staging app to {}", fake_host()),
        &format!(
            "Rotated the staging deploy token, redeployed, and mailed {} the release notes.",
            fake_email()
        ),
        &["deploy", "staging"],
        Outcome::Done,
    );
    let f1 = vec![
        card_finding(&c1, "title", &fake_host(), Kind::TailnetHost, 3),
        card_finding(&c1, "summary", &fake_email(), Kind::Email, 2),
    ];
    let c3 = card(
        "pi/s-notes",
        &format!("Meeting notes for {PERSON}"),
        "Tidied the notes folder and wrote a summary of the week.",
        &["notes"],
        Outcome::PartlyDone,
    );
    let f3 = vec![card_finding(&c3, "title", PERSON, Kind::PersonName, 5)];
    let opening = format!("set API_TOKEN={} and run the build again", fake_token());
    let (os, oe) = at(&opening, &fake_token());
    let opening_findings = vec![
        serde_json::from_value(json!({
            "event": 0, "field": "text", "start": os, "end": oe,
            "kind": Kind::AssignedSecret, "fingerprint": fingerprint(1),
        }))
        .expect("a finding"),
    ];
    vec![
        ResultRow {
            facts: facts(
                Harness::ClaudeCode,
                "s-deploy",
                "/work/app",
                "claude-opus-5-5",
                40,
                6,
            ),
            card: Some(Shown {
                card: c1,
                findings: f1,
            }),
            opening: Opening::Text("deploy the staging app".into()),
            opening_findings: Vec::new(),
            passage: None,
            started_from: None,
        },
        ResultRow {
            facts: facts(
                Harness::Codex,
                "s-build",
                "/work/tools",
                "gpt-6",
                60 * 26,
                12,
            ),
            card: None,
            opening: Opening::Text(opening),
            opening_findings,
            passage: None,
            started_from: None,
        },
        ResultRow {
            facts: facts(
                Harness::Pi,
                "s-notes",
                "/work/notes",
                "qwen-small:4b",
                60 * 24 * 4,
                5,
            ),
            card: Some(Shown {
                card: c3,
                findings: f3,
            }),
            opening: Opening::Text("tidy the notes".into()),
            opening_findings: Vec::new(),
            passage: None,
            started_from: None,
        },
    ]
}

fn on_sessions() -> State {
    let mut s = super::running();
    s.screen = Screen::Sessions;
    s
}

/// The archive listed newest first.
pub fn sessions_listed() -> State {
    let mut s = on_sessions();
    s.sessions.results = Some(Results {
        total: 3,
        next: None,
        query: None,
        notes: Vec::new(),
        rows: rows(),
    });
    s
}

/// A search for "deploy", with the passage it matched.
pub fn sessions_searched() -> State {
    let mut s = on_sessions();
    s.sessions.query.replace_text("deploy");
    let mut row = rows().remove(0);
    // A sub-agent's session, started from the owner's own session.
    row.started_from = Some(facts(
        Harness::ClaudeCode,
        "s-release",
        "/work/app",
        "claude-opus-5-5",
        90,
        24,
    ));
    let text = format!(
        "ok: deploy finished on {}, notes sent to {}",
        fake_host(),
        fake_email()
    );
    let (hs, he) = at(&text, &fake_host());
    let (es, ee) = at(&text, &fake_email());
    row.passage = Some(Passage {
        event: 4,
        kind: EventKind::ToolResult,
        field: distill_core::findings::Field::Text,
        cut_before: true,
        cut_after: false,
        findings: vec![
            serde_json::from_value(json!({
                "event": 4, "field": "text", "start": hs, "end": he,
                "kind": Kind::TailnetHost, "fingerprint": fingerprint(3),
            }))
            .expect("a finding"),
            serde_json::from_value(json!({
                "event": 4, "field": "text", "start": es, "end": ee,
                "kind": Kind::Email, "fingerprint": fingerprint(2),
            }))
            .expect("a finding"),
        ],
        text,
    });
    s.sessions.results = Some(Results {
        total: 1,
        next: None,
        query: Some("deploy".into()),
        notes: vec![
            "Search by meaning is off (no embedding_model is set): word matches only.".into(),
        ],
        rows: vec![row],
    });
    s
}

/// The deploy session's events: one of each kind, with findings in text and in tool input.
pub fn deploy_excerpt() -> Excerpt {
    let row = rows().remove(0);
    let said = format!(
        "Deploy the staging app. The token is {} and the notes go to {}.",
        fake_token(),
        fake_email()
    );
    let command = format!(
        "curl -H 'Authorization: Bearer {}' https://{}/deploy",
        fake_bearer(),
        fake_host()
    );
    let result = format!(
        "deploying…\nstep 1/3 build ok\nstep 2/3 upload ok\nstep 3/3 restart ok\nok: deploy finished on {}",
        fake_host()
    );
    let reply = format!("Deployed. I mailed the release notes to {}.", fake_email());
    let mut events = vec![
        Event::text(EventKind::User, said.clone()),
        Event::text(
            EventKind::Reasoning,
            "Check the token works before deploying.",
        ),
        Event::text(EventKind::Assistant, "Deploying the staging app now."),
        Event::new(EventKind::ToolCall),
        Event::text(EventKind::ToolResult, result.clone()),
        Event::text(EventKind::Assistant, reply.clone()),
    ];
    events[3].tool = Some("Bash".into());
    events[3].input = Some(json!({ "command": command, "timeout": 120 }));
    events[4].tool = Some("Bash".into());
    for (i, e) in events.iter_mut().enumerate() {
        e.seq = i as u64;
        e.ts = Some(format!("2026-11-15T11:20:{:02}Z", i * 4));
        if e.kind == EventKind::Assistant {
            e.model = Some("claude-opus-5-5".into());
        }
    }
    let findings = vec![
        finding(0, &said, &fake_token(), Kind::AssignedSecret, 1),
        finding(0, &said, &fake_email(), Kind::Email, 2),
        input_finding(3, &[0], &command, &fake_bearer(), Kind::Bearer, 4),
        input_finding(3, &[0], &command, &fake_host(), Kind::TailnetHost, 3),
        finding(4, &result, &fake_host(), Kind::TailnetHost, 3),
        finding(5, &reply, &fake_email(), Kind::Email, 2),
    ];
    Excerpt {
        total: events.len() as u64,
        facts: row.facts,
        events,
        findings,
    }
}

/// `x` open on the Sessions screen, from the first row of the list.
pub fn session_with(x: Excerpt) -> State {
    let mut s = sessions_listed();
    nav::go(&mut s, Screen::Sessions);
    s.outbox.clear();
    sessions::open_row(&mut s, 0);
    s.outbox.clear();
    let key = s.sessions.open.as_ref().expect("open").key.clone();
    sessions::land(
        &mut s,
        &key,
        sessions::loaded(Ok(Read::Events(Box::new(x)))),
    );
    s
}

/// The deploy session open, every finding covered.
pub fn session_covered() -> State {
    session_with(deploy_excerpt())
}

/// The deploy session with the email in the first message shown.
pub fn session_revealed() -> State {
    let mut s = session_covered();
    sessions::toggle(&mut s, "f1");
    s
}

#[cfg(test)]
/// A long made-up session: `n` events, every `long_every`th a tool result of 1,500 lines with a
/// finding every few lines.
pub fn session_long(n: usize, long_every: usize) -> State {
    let mut events = Vec::with_capacity(n);
    let mut findings = Vec::new();
    let kinds = [
        EventKind::User,
        EventKind::Assistant,
        EventKind::Reasoning,
        EventKind::ToolCall,
        EventKind::ToolResult,
    ];
    for i in 0..n {
        let long = long_every > 0 && i % long_every == 0;
        let kind = if long {
            EventKind::ToolResult
        } else {
            kinds[i % kinds.len()]
        };
        let mut text = String::new();
        if long {
            for l in 0..1_500 {
                if l > 0 {
                    text.push('\n');
                }
                text.push_str(&format!(
                    "{:>5} | build step {l} finished in {} ms",
                    l + 1,
                    l * 7 % 300
                ));
                if l % 9 == 0 {
                    text.push_str(" by ");
                    let start = text.len();
                    text.push_str(&fake_email());
                    findings.push(
                        serde_json::from_value::<Finding>(json!({
                            "event": i, "field": "text", "start": start, "end": text.len(),
                            "kind": Kind::Email, "fingerprint": fingerprint(2),
                        }))
                        .expect("a finding"),
                    );
                }
            }
        } else {
            text = format!(
                "Step {i}: looked at the parser and the index, then ran the tests with token "
            );
            let start = text.len();
            text.push_str(&fake_token());
            findings.push(
                serde_json::from_value::<Finding>(json!({
                    "event": i, "field": "text", "start": start, "end": text.len(),
                    "kind": Kind::AssignedSecret, "fingerprint": fingerprint(1),
                }))
                .expect("a finding"),
            );
            text.push_str(". All passed after the fix to the cache layer.");
        }
        let mut e = Event::text(kind, text);
        e.seq = i as u64;
        events.push(e);
    }
    let mut x = deploy_excerpt();
    x.total = events.len() as u64;
    x.events = events;
    x.findings = findings;
    session_with(x)
}
