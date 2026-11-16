//! `read_session`: one session's events as text, in full or by event range, from the read path.

use distill_core::read::{self, Range, Read};
use serde_json::{Value, json};

use super::{ARCHIVED_DATA, Tool, number, string};
use crate::Server;

pub struct ReadSession;

impl Tool for ReadSession {
    fn name(&self) -> &'static str {
        "read_session"
    }

    fn description(&self) -> String {
        format!(
            "Read one archived session's events (messages, tool calls and results, reasoning) \
             as text, in full or from one event number to another. Events are numbered from 0; \
             the first line says which events are shown and how many the session has, so a \
             long session can be read in parts. {ARCHIVED_DATA} The text comes between a notice \
             that opens and one that closes it."
        )
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session": {
                    "type": "string",
                    "description": "The session: harness/id as list_sessions shows it, or its id alone when only one harness has it."
                },
                "from": {"type": "integer", "minimum": 0, "description": "The first event to read, by number."},
                "to": {"type": "integer", "minimum": 0, "description": "The last event to read, by number."}
            },
            "required": ["session"],
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let session = string(args, "session")?.ok_or("name the session to read")?;
        let range = Range {
            from: number(args, "from")?,
            to: number(args, "to")?,
        };
        match read::read(
            &server.home,
            &server.runtime,
            &server.reader,
            &session,
            range,
        ) {
            Ok(Read::Text(text)) => Ok(text),
            Ok(Read::Refused(why)) => Err(format!("refused: {why}")),
            // Only the owner and Distill itself get events, and neither reads from here.
            Ok(Read::Events(_)) => Err("this reader is not served by the MCP server".into()),
            Err(e) => Err(format!("the read failed: {e:#}")),
        }
    }
}
