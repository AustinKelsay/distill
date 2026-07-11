//! Public `Library` product seam.

use std::path::Path;

use rusqlite::Connection;

use crate::adapter::FixtureAdapter;
use crate::error::LibraryResult;
use crate::ingest;
use crate::query;
use crate::storage::{ensure_home_layout, migrate_to_latest, open_connection, DistillPaths};
use crate::types::{
    ActivityEventSummary, HealthReport, IngestReport, SearchHit, SessionDetail,
    DEFAULT_MAX_CAPTURE_BYTES, MAX_PAGE_SIZE,
};

/// Deep Distill Library over one Distill home.
pub struct Library {
    paths: DistillPaths,
    conn: Connection,
    max_capture_bytes: u64,
}

impl Library {
    /**
     * Open or create a Distill home, apply checksummed migrations, and return a Library.
     *
     * Parameters:
     * - `home`: Distill home directory path. Created with mode `0o700` when missing.
     */
    pub fn open(home: impl AsRef<Path>) -> LibraryResult<Self> {
        Self::open_with_limits(home, DEFAULT_MAX_CAPTURE_BYTES)
    }

    /**
     * Open a Distill home with an explicit maximum Capture size.
     *
     * Parameters:
     * - `home`: Distill home directory.
     * - `max_capture_bytes`: rejection threshold for Capture acceptance.
     */
    pub fn open_with_limits(home: impl AsRef<Path>, max_capture_bytes: u64) -> LibraryResult<Self> {
        let paths = ensure_home_layout(home.as_ref())?;
        let mut conn = open_connection(&paths)?;
        migrate_to_latest(&mut conn)?;
        Ok(Self {
            paths,
            conn,
            max_capture_bytes,
        })
    }

    /// Absolute Distill home path.
    pub fn home(&self) -> &Path {
        &self.paths.home
    }

    /**
     * Ingest a Fixture root through the production SourceAdapter seam.
     *
     * Parameters:
     * - `fixture_root`: directory containing `distill.fixture.json`.
     */
    pub fn ingest_fixture(
        &mut self,
        fixture_root: impl AsRef<Path>,
    ) -> LibraryResult<IngestReport> {
        let adapter = FixtureAdapter::new(fixture_root.as_ref().to_path_buf());
        ingest::ingest_adapter(
            &mut self.conn,
            &self.paths,
            &adapter,
            self.max_capture_bytes,
        )
    }

    /**
     * Load a bounded first slice of one Session Projection by identity.
     *
     * Parameters:
     * - `source_kind`: Source kind string such as `fixture`.
     * - `external_session_id`: Stable Session Identity.
     */
    pub fn session_slice(
        &self,
        source_kind: &str,
        external_session_id: &str,
        message_limit: u32,
        artifact_limit: u32,
    ) -> LibraryResult<Option<SessionDetail>> {
        validate_page_size(message_limit)?;
        validate_page_size(artifact_limit)?;
        query::get_session(
            &self.conn,
            source_kind,
            external_session_id,
            message_limit,
            artifact_limit,
        )
    }

    /**
     * Search the current Session Projection via FTS5.
     *
     * Parameters:
     * - `query`: free-text query. Punctuation-only input returns no hits.
     */
    pub fn search(&self, query_text: &str, limit: u32) -> LibraryResult<Vec<SearchHit>> {
        validate_page_size(limit)?;
        query::search(&self.conn, query_text, limit)
    }

    /**
     * Replay Distill-owned Capture bytes by Capture id.
     *
     * Parameters:
     * - `capture_id`: accepted Capture row id.
     */
    pub fn replay_capture(&self, capture_id: i64) -> LibraryResult<Vec<u8>> {
        query::replay_capture(&self.conn, &self.paths.home, capture_id)
    }

    /**
     * Report schema, content, and FTS health for this Distill home.
     */
    pub fn health(&self) -> LibraryResult<HealthReport> {
        query::health(&self.conn, &self.paths.home)
    }

    /**
     * List Activity Events for contract assertions.
     */
    pub fn recent_activity(&self, limit: u32) -> LibraryResult<Vec<ActivityEventSummary>> {
        validate_page_size(limit)?;
        query::list_activity(&self.conn, limit)
    }
}

fn validate_page_size(limit: u32) -> LibraryResult<()> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(crate::error::LibraryError::InvalidArgument(format!(
            "page size must be between 1 and {MAX_PAGE_SIZE}"
        )));
    }
    Ok(())
}
