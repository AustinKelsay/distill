mod connection;
mod import;
mod migrations;
mod paths;
mod schema;

use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::config::AppPaths;

pub use import::SyncReport;

#[derive(Clone, Debug)]
pub struct RustStore {
    app_paths: AppPaths,
}

impl RustStore {
    pub fn initialize(app_paths: AppPaths) -> Result<Self> {
        paths::ensure_layout(&app_paths)?;
        let mut connection = connection::open_read_write(&app_paths.db_path)?;
        migrations::migrate_to_latest(&mut connection)?;
        Ok(Self { app_paths })
    }

    pub fn app_home(&self) -> &Path {
        &self.app_paths.app_home
    }

    pub fn database_path(&self) -> &Path {
        &self.app_paths.db_path
    }

    pub fn database_exists(&self) -> bool {
        self.database_path().exists()
    }

    pub fn open_read_only(&self) -> Result<Connection> {
        connection::open_read_only(self.database_path())
    }
}

#[cfg(test)]
mod tests {
    use super::RustStore;
    use crate::storage::migrations::PHASE1_SCHEMA_VERSION;
    use rusqlite::Connection;
    use tempfile::tempdir;

    use crate::config::AppPaths;

    fn temp_paths() -> AppPaths {
        let dir = tempdir().unwrap();
        let app_home = dir.keep();
        AppPaths {
            db_path: app_home.join("distill.db"),
            blobs_dir: app_home.join("blobs"),
            prefs_path: app_home.join("preferences.json"),
            app_home,
        }
    }

    #[test]
    fn initializes_layout_and_schema() {
        let paths = temp_paths();
        RustStore::initialize(paths.clone()).unwrap();
        assert!(paths.app_home.exists());
        assert!(paths.blobs_dir.exists());
        assert!(paths.db_path.exists());
        let connection = Connection::open(paths.db_path).unwrap();
        let version: i64 = connection
            .query_row(
                "SELECT version FROM schema_migrations WHERE version = ?1",
                [PHASE1_SCHEMA_VERSION.0],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, PHASE1_SCHEMA_VERSION.0);
    }

    #[test]
    fn seeds_sources_and_labels() {
        let paths = temp_paths();
        RustStore::initialize(paths.clone()).unwrap();
        let connection = Connection::open(paths.db_path).unwrap();
        let source_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))
            .unwrap();
        let label_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM labels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(source_count, 3);
        assert_eq!(label_count, 5);
    }
}
