//! Distill core: the raw archive, the files rebuilt from it, and the tick that keeps them
//! current. Every face (the command line, the MCP server, the GUI) goes through this crate.
//!
//! Capture (the sweep, the raw archive, `verify`, the timer) and the parsers with their mirror
//! rules and format-drift detection (ADR 0001); trust and the read path; the detector
//! (`detect`) and findings (`findings`); the word index and vectors (`index`); cards (`cards`)
//! and personal details found by a model (`details`); and reveals (`reveal`). Terms are in
//! `GLOSSARY.md`. The layout of a Distill home is fixed, and
//! [`Home`] has a path helper for every entry in it.

pub mod archive;
pub mod cards;
pub mod config;
pub mod details;
pub mod detect;
pub mod exposure;
pub mod findings;
pub mod harness;
pub mod home;
pub mod index;
pub mod model;
mod model_step;
pub mod normalize;
pub mod pruning;
pub mod read;
pub mod readlog;
pub mod reveal;
pub mod runlog;
pub mod runtime;
pub mod schedule;
pub mod setup;
pub mod status;
pub mod sweep;
pub mod tick;
pub mod trust;
pub mod util;

pub use config::Config;
pub use harness::Harness;
pub use home::Home;
