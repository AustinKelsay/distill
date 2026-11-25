//! `list_tags`: the tags on the matching sessions' cards, each with how many sessions carry it,
//! from the read path, so an agent can see what topics exist before it searches.

use distill_core::read::{self, Tagged};
use serde_json::{Value, json};

use super::list_sessions::{filter, filter_properties};
use super::{ARCHIVED_DATA, Tool};
use crate::Server;

pub struct ListTags;

impl Tool for ListTags {
    fn name(&self) -> &'static str {
        "list_tags"
    }

    fn description(&self) -> String {
        format!(
            "List the tags on archived sessions' cards (short topics a model wrote: a language, \
             a tool, a project, a kind of task), each with how many sessions carry it, most \
             used first. Takes the same filters as list_sessions; with a tag, it lists the tags \
             that go with it. Use a tag from here as list_sessions' or search_sessions' tag \
             filter. Mirrors and sessions the previous build's steward ran are left out unless \
             asked for. {ARCHIVED_DATA} The tags come between a notice that opens and one that \
             closes them."
        )
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": filter_properties(),
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let filter = filter(args)?;
        match read::tags(&server.home, &server.runtime, &server.reader, &filter) {
            Ok(Tagged::Text(text)) => Ok(text),
            Ok(Tagged::Refused(why)) => Err(format!("refused: {why}")),
            Ok(Tagged::Counts(_)) => Err("this reader is not served by the MCP server".into()),
            Err(e) => Err(format!("the tag list failed: {e:#}")),
        }
    }
}
