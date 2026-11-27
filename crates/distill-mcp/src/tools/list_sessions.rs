//! `list_sessions`: sessions with their facts and their card, or the opening of what the owner
//! first wrote when they have none, filtered and a page at a time, from the read path.

use chrono::NaiveDate;
use distill_core::Harness;
use distill_core::cards::Outcome;
use distill_core::read::{self, Filter, Listed, PAGE_MAX, Page};
use serde_json::{Value, json};

use super::{ARCHIVED_DATA, Tool, flag, number, opt_flag, string, strings};
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
             below), model (part of its name), start date, the card's tag and outcome, exclusions \
             (exclude_tag, exclude_project, exclude_harness), and session shape (has_reasoning, \
             min_turns, max_turns, min_tool_calls, max_bytes, tool). At \
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
        "exclude_tag": {"type": "array", "items": {"type": "string"}, "description": "Leave out sessions whose card has any of these tags, ignoring case."},
        "exclude_project": {"type": "array", "items": {"type": "string"}, "description": "Leave out sessions that worked in any of these folders or one inside them."},
        "exclude_harness": {"type": "array", "items": {"type": "string", "enum": harnesses}, "description": "Leave out sessions from any of these harnesses."},
        "has_reasoning": {"type": "boolean", "description": "Only sessions with (true) or without (false) reasoning events."},
        "min_turns": {"type": "integer", "minimum": 0, "description": "Only sessions with at least this many owner messages."},
        "max_turns": {"type": "integer", "minimum": 0, "description": "Only sessions with at most this many owner messages."},
        "min_tool_calls": {"type": "integer", "minimum": 0, "description": "Only sessions with at least this many tool calls."},
        "max_bytes": {"type": "integer", "minimum": 0, "description": "Only sessions whose total content bytes are at most this many."},
        "tool": {"type": "array", "items": {"type": "string"}, "description": "Only sessions that called every one of these tools, ignoring case."},
        "outcome": {"type": "string", "enum": outcomes, "description": "Only sessions whose card has this outcome."}
    }) else {
        unreachable!("an object literal")
    };
    o
}

fn parse_harness_id(id: &str) -> Result<Harness, String> {
    Harness::parse(id).ok_or_else(|| {
        let ids: Vec<&str> = Harness::ALL.iter().map(|h| h.id()).collect();
        format!("unknown harness {id:?}: one of {}", ids.join(", "))
    })
}

/// The `harness` in `args`, by its ID.
pub(super) fn harness(args: &Value) -> Result<Option<Harness>, String> {
    string(args, "harness")?
        .map(|id| parse_harness_id(&id))
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
    let exclude_harnesses = strings(args, "exclude_harness")?
        .iter()
        .map(|id| parse_harness_id(id))
        .collect::<Result<Vec<_>, _>>()?;
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
        exclude_tags: strings(args, "exclude_tag")?,
        exclude_projects: strings(args, "exclude_project")?,
        exclude_harnesses,
        has_reasoning: opt_flag(args, "has_reasoning")?,
        min_turns: number(args, "min_turns")?,
        max_turns: number(args, "max_turns")?,
        min_tool_calls: number(args, "min_tool_calls")?,
        max_bytes: number(args, "max_bytes")?,
        tools: strings(args, "tool")?,
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
