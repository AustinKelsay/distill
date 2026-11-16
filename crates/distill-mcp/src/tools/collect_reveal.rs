//! `collect_reveal`: the values of a reveal request the owner approved, once, for the reader
//! that filed it.

use distill_core::reveal::{self, Collected};
use serde_json::{Value, json};

use super::{ARCHIVED_DATA, Tool, string};
use crate::Server;

pub struct CollectReveal;

impl Tool for CollectReveal {
    fn name(&self) -> &'static str {
        "collect_reveal"
    }

    fn description(&self) -> String {
        format!(
            "Collect the real values of a reveal request once the owner has said yes, by the ID \
             request_reveal returned. Only the model that filed the request may collect, only \
             once, and only within 24 hours of the request; before the owner decides, this \
             says the request is not approved yet. Use the values for the task the request \
             named and nothing else. {ARCHIVED_DATA}"
        )
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request": {"type": "string", "description": "The request ID request_reveal returned."}
            },
            "required": ["request"],
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let request = string(args, "request")?.unwrap_or_default();
        match reveal::collect(&server.home, &server.runtime, &server.reader, &request) {
            Ok(Collected::Released(r)) => Ok(r.text()),
            Ok(Collected::Refused(why)) => Err(format!("refused: {why}")),
            Err(e) => Err(format!("the collection failed: {e:#}")),
        }
    }
}
