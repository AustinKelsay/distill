mod claude_code;
mod codex;
mod types;

use anyhow::Result;

pub use claude_code::ClaudeCodeConnector;
pub use codex::CodexConnector;
pub use types::{
    CaptureSnapshot, DiscoveredCapture, DiscoveredSource, ImportFailureEntry, ImportSourceSummary,
    NormalizedArtifact, NormalizedMessage, NormalizedSession, ParsedCapture, ParsedCaptureRecord,
    SourceConnector, SourceKind,
};

pub fn configured_rust_connectors() -> Result<Vec<Box<dyn SourceConnector>>> {
    Ok(vec![
        Box::new(CodexConnector::from_env()?),
        Box::new(ClaudeCodeConnector::from_env()?),
    ])
}
