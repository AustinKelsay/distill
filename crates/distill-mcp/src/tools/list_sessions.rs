//! `list_sessions`: sessions with their facts and their card, or the opening of what the owner
//! first wrote when they have none, filtered and a page at a time, from the read path.

use chrono::NaiveDate;
use distill_core::Harness;
use distill_core::cards::Outcome;
use distill_core::read::{self, Filter, Listed, PAGE_MAX, Page};
use serde_json::{Value, json};

use super::{ARCHIVED_DATA, Tool, flag, number, string, strings};
use crate::Server;

pub struct ListSessions;

impl Tool for ListSessions {
    fn name(&self) -> &'static str {
        "list_sessions"
    }

    fn description(&self) -> String {
        format!(
            "List archived sessions, newest first, with each one's facts (harness, project, \
             branch, models, times, counts) and its card (a title, a summary, tags and an \
             outcome, written by a model), or the opening of what the owner first wrote \
             when it has no card yet. Filter by harness, machine, project (a folder; sessions in it and \
             below), model (part of its name), start date, and the card's tag and outcome. At \
             most {PAGE_MAX} rows per call; the first line says how \
             many match and the offset where the next page starts. Mirrors of sessions already \
             in the archive, and sessions the previous build's steward ran, are left out unless \
             asked for. {ARCHIVED_DATA} The cards and openings come between a notice that opens \
             and one that closes them."
        )
    }

    fn input_schema(&self) -> Value {
        let mut properties = filter_properties();
        properties.insert("offset".into(), json!({"type": "integer", "minimum": 0, "description": "How many matching sessions to skip: where the page starts."}));
        properties.insert("limit".into(), json!({"type": "integer", "minimum": 1, "maximum": PAGE_MAX, "description": format!("How many rows at most (default and most: {PAGE_MAX}).")}));
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let filter = filter(args)?;
        let page = page(args, PAGE_MAX)?;
        match read::list(&server.home, &server.runtime, &server.reader, &filter, page) {
            Ok(Listed::Text(text)) => Ok(text),
            Ok(Listed::Refused(why)) => Err(format!("refused: {why}")),
            Ok(Listed::Rows(_)) => Err("this reader is not served by the MCP server".into()),
            Err(e) => Err(format!("the list failed: {e:#}")),
        }
    }
}

/// The schema of the session filters `list_sessions` and `search_sessions` take.
pub(super) fn filter_properties() -> serde_json::Map<String, Value> {
    let harnesses: Vec<&str> = Harness::ALL.iter().map(|h| h.id()).collect();
    let outcomes: Vec<&str> = Outcome::ALL.iter().map(|o| o.id()).collect();
    let Value::Object(o) = json!({
        "harness": {"type": "string", "enum": harnesses, "description": "Only this harness's sessions."},
        "machine": {"type": "string", "description": "Only sessions captured on this machine: a spoke by its name, or hub for the hub's own. A spoke's session keys start with its name and a colon."},
        "project": {"type": "string", "description": "Only sessions that worked in this folder or one inside it."},
        "model": {"type": "string", "description": "Only sessions that used a model whose name contains this, ignoring case."},
        "since": {"type": "string", "description": "Only sessions that started on or after this day, YYYY-MM-DD (UTC)."},
        "until": {"type": "string", "description": "Only sessions that started on or before this day, YYYY-MM-DD (UTC)."},
        "include_mirrors": {"type": "boolean", "description": "Include mirrors of sessions already in the archive."},
        "include_steward": {"type": "boolean", "description": "Include sessions the previous build's steward ran."},
        "tag": {"type": "array", "items": {"type": "string"}, "description": "Only sessions whose card has every one of these tags, ignoring case."},
        "outcome": {"type": "string", "enum": outcomes, "description": "Only sessions whose card has this outcome."}
    }) else {
        unreachable!("an object literal")
    };
    o
}

/// The `harness` in `args`, by its ID.
pub(super) fn harness(args: &Value) -> Result<Option<Harness>, String> {
    string(args, "harness")?
        .map(|id| {
            Harness::parse(&id).ok_or_else(|| {
                let ids: Vec<&str> = Harness::ALL.iter().map(|h| h.id()).collect();
                format!("unknown harness {id:?}: one of {}", ids.join(", "))
            })
        })
        .transpose()
}

/// The day named `name` in `args`, as YYYY-MM-DD.
pub(super) fn day(args: &Value, name: &str) -> Result<Option<NaiveDate>, String> {
    string(args, name)?
        .map(|d| {
            read::date(&d).map_err(|_| format!("{name} must be a day as YYYY-MM-DD, not {d:?}"))
        })
        .transpose()
}

/// The session filters in `args`.
pub(super) fn filter(args: &Value) -> Result<Filter, String> {
    Ok(Filter {
        harness: harness(args)?,
        machine: string(args, "machine")?,
        project: string(args, "project")?,
        model: string(args, "model")?,
        since: day(args, "since")?,
        until: day(args, "until")?,
        mirrors: flag(args, "include_mirrors")?,
        steward: flag(args, "include_steward")?,
        tags: strings(args, "tag")?,
        outcome: match string(args, "outcome")? {
            Some(o) => Some(Outcome::parse(&o).ok_or_else(|| {
                let ids: Vec<&str> = Outcome::ALL.iter().map(|o| o.id()).collect();
                format!("unknown outcome {o:?}: one of {}", ids.join(", "))
            })?),
            None => None,
        },
        session: None,
    })
}

/// The page in `args`: `offset`, and `limit` from 1 to [`PAGE_MAX`], `default` when not given.
pub(super) fn page(args: &Value, default: usize) -> Result<Page, String> {
    let limit = match number(args, "limit")? {
        Some(0) => return Err(format!("limit must be from 1 to {PAGE_MAX}")),
        Some(n) => usize::try_from(n).unwrap_or(PAGE_MAX).min(PAGE_MAX),
        None => default,
    };
    Ok(Page {
        offset: number(args, "offset")?.map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX)),
        limit,
    })
}
