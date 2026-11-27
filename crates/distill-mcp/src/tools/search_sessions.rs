//! `search_sessions`: sessions that have every word of a query or are close to it in meaning, best
//! first in one list, with each one's facts,
//! its card (or the opening of what the owner first wrote) and the matching passage, from the
//! read path.

use distill_core::read::{self, PAGE_MAX, SEARCH_DEFAULT, Searched};
use serde_json::{Value, json};

use super::list_sessions::{filter, filter_properties, page};
use super::{ARCHIVED_DATA, Tool, string};
use crate::Server;

pub struct SearchSessions;

impl Tool for SearchSessions {
    fn name(&self) -> &'static str {
        "search_sessions"
    }

    fn description(&self) -> String {
        format!(
            "Search archived sessions by exact words (an error string, a file name, a command) \
             and by meaning (a loose description of past work). A session matches when it has \
             every word of the query, in its text or its card, ignoring case (words are letters, \
             digits and underscores, and a part of a snake_case name finds the whole), or when \
             what the owner wrote in it or its card's summary is close in meaning to the query. \
             Both kinds of match are merged into one list; when search by meaning is off, the \
             first line says so and the results are word matches only. Results come best first, each with the session's \
             facts (harness, project, branch, models, times, counts; for a session an agent started, the session it was started from), its card (a title, a \
             summary, tags and an outcome, written by a model) or, with no card yet, the \
             opening of what the owner first wrote, and the passage that matched. Filter by \
             harness, machine, project (a folder; sessions in it and below), model (part of its name), \
             start date, the card's tag and outcome, exclusions (exclude_tag, exclude_project, \
             exclude_harness), and session shape (has_reasoning, min_turns, max_turns, \
             min_tool_calls, max_bytes, tool). At most {PAGE_MAX} results per call \
             ({SEARCH_DEFAULT} unless asked); the first line says how many match and the offset \
             where the next page starts. Mirrors of sessions already in the archive, and \
             sessions the previous build's steward ran, are left out unless asked for. Secrets \
             read as placeholders, `[secret: <kind> #<fingerprint>]`; search for a fingerprint \
             to find every session that holds the same secret. Read a result in full with \
             read_session. {ARCHIVED_DATA} The cards, openings and passages come between a notice that \
             opens and one that closes them."
        )
    }

    fn input_schema(&self) -> Value {
        let mut properties = filter_properties();
        properties.insert("query".into(), json!({"type": "string", "description": "What to find: words a session has every one of, or a description of what it was about."}));
        properties.insert("offset".into(), json!({"type": "integer", "minimum": 0, "description": "How many matching sessions to skip: where the page starts."}));
        properties.insert("limit".into(), json!({"type": "integer", "minimum": 1, "maximum": PAGE_MAX, "description": format!("How many results at most (default {SEARCH_DEFAULT}, most {PAGE_MAX}).")}));
        json!({
            "type": "object",
            "properties": properties,
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let query = string(args, "query")?.ok_or("query is required: the words to find")?;
        let filter = filter(args)?;
        let page = page(args, SEARCH_DEFAULT)?;
        match read::search(
            &server.home,
            &server.runtime,
            &server.reader,
            &query,
            &filter,
            page,
        ) {
            Ok(Searched::Text(text)) => Ok(text),
            Ok(Searched::Refused(why)) => Err(format!("refused: {why}")),
            Ok(Searched::Hits(_)) => Err("this reader is not served by the MCP server".into()),
            Err(e) => Err(format!("the search failed: {e:#}")),
        }
    }
}
