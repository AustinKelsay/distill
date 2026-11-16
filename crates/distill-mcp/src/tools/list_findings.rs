//! `list_findings`: the archive's findings by fingerprint, secrets by default (the rotation
//! list), or every session one fingerprint appears in, from the read path. Never a value.

use distill_core::Harness;
use distill_core::read::{self, Answer, FindingFilter, Kinds, Order, PAGE_MAX};
use serde_json::{Value, json};

use super::list_sessions::{day, harness, page};
use super::{ARCHIVED_DATA, Tool, string};
use crate::Server;

pub struct ListFindings;

impl Tool for ListFindings {
    fn name(&self) -> &'static str {
        "list_findings"
    }

    fn description(&self) -> String {
        format!(
            "List the sensitive values found in archived sessions, by fingerprint, never the \
             values themselves: each fingerprint's short form (as placeholders show it, \
             `[secret: <kind> #<fingerprint>]`), its kind, how many sessions it appears in, \
             their harnesses, and when it was first and last seen. By default it lists secrets, \
             most widespread first: the rotation list. Filter by kind (`secrets`, `personal`, \
             `all`, or one kind such as `github_token`), harness and session start date; sort \
             by spread or recency. At most {PAGE_MAX} fingerprints per call. Given a \
             `fingerprint` (whole or its short form), it instead lists every session that value \
             appears in, with the events it is in, for every fingerprint that starts with it. \
             Answers from the stored findings: sessions not scanned yet are counted, not \
             searched. {ARCHIVED_DATA}"
        )
    }

    fn input_schema(&self) -> Value {
        let harnesses: Vec<&str> = Harness::ALL.iter().map(|h| h.id()).collect();
        json!({
            "type": "object",
            "properties": {
                "fingerprint": {"type": "string", "description": "A fingerprint or its short form: list every session it appears in. The other arguments are then ignored."},
                "kind": {"type": "string", "description": "secrets (the default), personal, all, or one kind's name as placeholders write it."},
                "harness": {"type": "string", "enum": harnesses, "description": "Only this harness's sessions."},
                "since": {"type": "string", "description": "Only sessions that started on or after this day, YYYY-MM-DD (UTC)."},
                "until": {"type": "string", "description": "Only sessions that started on or before this day, YYYY-MM-DD (UTC)."},
                "sort": {"type": "string", "enum": ["spread", "recent"], "description": "spread (most sessions first, the default) or recent (most recently seen first)."},
                "offset": {"type": "integer", "minimum": 0, "description": "How many fingerprints to skip: where the page starts."},
                "limit": {"type": "integer", "minimum": 1, "maximum": PAGE_MAX, "description": format!("How many fingerprints at most (default and most: {PAGE_MAX}).")}
            },
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let (home, runtime, reader) = (&server.home, &server.runtime, &server.reader);
        if let Some(asked) = string(args, "fingerprint")? {
            return answer(read::where_found(home, runtime, reader, &asked), |l| {
                l.text()
            });
        }
        let kinds = match string(args, "kind")? {
            Some(k) => Kinds::parse(&k).map_err(|e| e.to_string())?,
            None => Kinds::Secrets,
        };
        let filter = FindingFilter {
            kinds,
            harness: harness(args)?,
            since: day(args, "since")?,
            until: day(args, "until")?,
        };
        let order = match string(args, "sort")? {
            Some(s) => Order::parse(&s).map_err(|e| e.to_string())?,
            None => Order::Spread,
        };
        let page = page(args, PAGE_MAX)?;
        answer(
            read::fingerprints(home, runtime, reader, &filter, order, page),
            |f| f.text(),
        )
    }
}

/// The served answer as text, or the refusal or failure as the error.
fn answer<T>(
    a: anyhow::Result<Answer<T>>,
    text: impl FnOnce(&T) -> String,
) -> Result<String, String> {
    match a {
        Ok(Answer::Served(t)) => Ok(text(&t)),
        Ok(Answer::Refused(why)) => Err(format!("refused: {why}")),
        Err(e) => Err(format!("the list failed: {e:#}")),
    }
}
