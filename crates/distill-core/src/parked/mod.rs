//! Code from the previous build kept for the tickets that port it, with no callers until then.
//!
//! - [`redact`]: the old redactor's patterns and tests, for the detector ticket. Its text
//!   rewriting is not ported (ADR 0001, 0002).
//! - [`ollama`]: the check that a model is stored on this machine rather than served from a
//!   cloud, for the providers ticket.

pub mod ollama;
pub mod redact;
