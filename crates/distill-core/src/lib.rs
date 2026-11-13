//! Distill core: the raw archive, the derived session files built from it, and the steward that
//! keeps them current. Every caller (CLI, MCP server, GUI) goes through this crate.
//!
//! Layout of a Distill home:
//!
//! ```text
//! config.toml
//! raw/<aa>/<sha256>        exact source bytes, content-addressed
//! raw/manifest.jsonl       append-only log of every capture
//! sessions/<source>/<id>/  meta.json, events.jsonl, card.json
//! sessions/index.jsonl     one meta per line, rebuilt by normalize
//! recipes/<name>.toml      dataset definitions
//! exports/<recipe>/<build> data.jsonl, manifest.json, approval.json
//! runs/<date>.jsonl        what each sweep, normalize and steward run did
//! state/                   caches that can be deleted and rebuilt
//! ```

pub mod archive;
pub mod card;
pub mod config;
pub mod export;
pub mod git;
pub mod home;
pub mod model;
pub mod normalize;
pub mod redact;
pub mod runlog;
pub mod schedule;
pub mod search;
pub mod setup;
pub mod source;
pub mod stats;
pub mod steward;
pub mod sweep;
pub mod tick;
pub mod util;

pub use config::Config;
pub use home::Home;
pub use source::SourceKind;
