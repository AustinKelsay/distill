//! Distill core: the raw archive, the files rebuilt from it, and the tick that keeps them
//! current. Every face (the command line, the MCP server, the GUI) goes through this crate.
//!
//! What is here today is what the rebuild keeps (ADR 0001): capture (the sweep, the raw archive,
//! `verify`, the timer) and the parsers with their mirror rules and format-drift detection. The
//! spec is issue #72; terms are in `GLOSSARY.md`. The layout of a Distill home is fixed by the
//! spec, and [`Home`] has a path helper for every entry in it.

pub mod archive;
pub mod config;
pub mod harness;
pub mod home;
pub mod model;
pub mod normalize;
pub mod parked;
pub mod read;
pub mod readlog;
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
