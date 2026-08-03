//! Library-owned parser identity registry for closed v1 Source kinds.
//!
//! Parser ids are fixed from adapter constants. Callers may advance only the
//! semantic version for a typed [`SourceKind`].

use semver::Version;

use super::{
    ParserIdentity, SourceKind, CLAUDE_PARSER_ID, CLAUDE_PARSER_VERSION, CODEX_PARSER_ID,
    CODEX_PARSER_VERSION, DROID_PARSER_ID, DROID_PARSER_VERSION, FIXTURE_PARSER_ID,
    FIXTURE_PARSER_VERSION, OPENCODE_PARSER_ID, OPENCODE_PARSER_VERSION, PI_PARSER_ID,
    PI_PARSER_VERSION,
};
use crate::error::{LibraryError, LibraryResult};

/// In-memory registry of one parser identity per closed v1 Source kind.
#[derive(Clone, Debug)]
pub struct ParserRegistry {
    fixture: ParserIdentity,
    codex: ParserIdentity,
    claude_code: ParserIdentity,
    opencode: ParserIdentity,
    droid: ParserIdentity,
    pi: ParserIdentity,
}

impl ParserRegistry {
    /**
     * Build the default v1 registry from adapter parser constants.
     */
    pub fn default_v1() -> Self {
        Self {
            fixture: ParserIdentity {
                id: FIXTURE_PARSER_ID.to_string(),
                version: FIXTURE_PARSER_VERSION.to_string(),
            },
            codex: ParserIdentity {
                id: CODEX_PARSER_ID.to_string(),
                version: CODEX_PARSER_VERSION.to_string(),
            },
            claude_code: ParserIdentity {
                id: CLAUDE_PARSER_ID.to_string(),
                version: CLAUDE_PARSER_VERSION.to_string(),
            },
            opencode: ParserIdentity {
                id: OPENCODE_PARSER_ID.to_string(),
                version: OPENCODE_PARSER_VERSION.to_string(),
            },
            droid: ParserIdentity {
                id: DROID_PARSER_ID.to_string(),
                version: DROID_PARSER_VERSION.to_string(),
            },
            pi: ParserIdentity {
                id: PI_PARSER_ID.to_string(),
                version: PI_PARSER_VERSION.to_string(),
            },
        }
    }

    /**
     * Return the registered parser identity for a closed Source kind.
     *
     * Parameters:
     * - `kind`: Closed v1 Source kind.
     */
    pub fn get(&self, kind: SourceKind) -> &ParserIdentity {
        match kind {
            SourceKind::Fixture => &self.fixture,
            SourceKind::Codex => &self.codex,
            SourceKind::ClaudeCode => &self.claude_code,
            SourceKind::OpenCode => &self.opencode,
            SourceKind::Droid => &self.droid,
            SourceKind::Pi => &self.pi,
        }
    }

    /**
     * Advance the registered parser version for one Source kind.
     *
     * The parser id remains the adapter-owned constant. Callers cannot supply
     * arbitrary parser ids.
     *
     * Parameters:
     * - `kind`: Closed v1 Source kind whose parser version advances.
     * - `version`: Strictly newer semantic version string.
     */
    pub fn set_version(
        &mut self,
        kind: SourceKind,
        version: impl Into<String>,
    ) -> LibraryResult<()> {
        let version = version.into();
        let requested = Version::parse(&version).map_err(|_| {
            LibraryError::InvalidArgument(format!(
                "{} parser version must be a semantic version",
                kind.as_str()
            ))
        })?;
        let current_identity = self.get(kind);
        let current = Version::parse(&current_identity.version).map_err(|_| {
            LibraryError::InvalidArgument(format!(
                "registered {} parser version is invalid",
                kind.as_str()
            ))
        })?;
        if requested <= current {
            return Err(LibraryError::InvalidArgument(format!(
                "{} parser version must advance beyond the registered version",
                kind.as_str()
            )));
        }
        self.get_mut(kind).version = version;
        Ok(())
    }

    /**
     * Mutable access to one registered parser identity.
     */
    fn get_mut(&mut self, kind: SourceKind) -> &mut ParserIdentity {
        match kind {
            SourceKind::Fixture => &mut self.fixture,
            SourceKind::Codex => &mut self.codex,
            SourceKind::ClaudeCode => &mut self.claude_code,
            SourceKind::OpenCode => &mut self.opencode,
            SourceKind::Droid => &mut self.droid,
            SourceKind::Pi => &mut self.pi,
        }
    }
}
