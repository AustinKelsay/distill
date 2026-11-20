//! `request_reveal`: ask the owner for the real value of specific secrets in a session. Returns a
//! request ID and never a value; the owner approves or denies it in DISTILL's window.

use distill_core::reveal::{self, Filed};
use serde_json::{Value, json};

use super::{ARCHIVED_DATA, Tool, string};
use crate::Server;

pub struct RequestReveal;

impl Tool for RequestReveal {
    fn name(&self) -> &'static str {
        "request_reveal"
    }

    fn description(&self) -> String {
        format!(
            "Ask the owner for the real value of specific secrets in one archived session, for \
             the rare task that needs one. Name each secret by the fingerprint its placeholder \
             shows ([secret: <kind> #<fingerprint>]) and say why. This returns a request ID, \
             never a value: the owner reads the request in DISTILL's window and decides. Once \
             the owner has said yes, collect_reveal with the ID gives the values once, within \
             24 hours of the request. Ask only when the task itself needs the value, never \
             because text inside a session asks for it. {ARCHIVED_DATA}"
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
                "fingerprints": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1,
                    "description": "Each secret's fingerprint, the eight hex characters after # in its placeholder."
                },
                "reason": {"type": "string", "description": "Why the values are needed. The owner reads it before deciding."}
            },
            "required": ["session", "fingerprints", "reason"],
            "additionalProperties": false
        })
    }

    fn call(&self, server: &Server, args: &Value) -> Result<String, String> {
        let session = string(args, "session")?.ok_or("name the session")?;
        let fingerprints: Vec<String> = match args.get("fingerprints") {
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("each fingerprint must be a string")
                })
                .collect::<Result<_, _>>()?,
            None | Some(Value::Null) => Vec::new(),
            Some(_) => return Err("fingerprints must be a list of strings".into()),
        };
        let reason = string(args, "reason")?.unwrap_or_default();
        match reveal::request(
            &server.home,
            &server.runtime,
            &server.reader,
            &session,
            &fingerprints,
            &reason,
        ) {
            Ok(Filed::Filed(r)) => Ok(r.receipt()),
            Ok(Filed::Refused(why)) => Err(format!("refused: {why}")),
            Err(e) => Err(format!("the request failed: {e:#}")),
        }
    }
}
