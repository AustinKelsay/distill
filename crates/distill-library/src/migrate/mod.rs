//! Read-only legacy Electron Distill home import into a native Library home.
//!
//! Snapshots the legacy SQLite database read-only (including WAL sidecars), copies
//! Distill-owned capture bytes into the destination CAS, and maps representative
//! rows into the rebuild schema inside one destination transaction keyed by a
//! durable fingerprint.

mod content;
mod fingerprint;
mod import;
mod map;
mod paths;
mod redact;

pub use import::import_legacy_electron_home;
