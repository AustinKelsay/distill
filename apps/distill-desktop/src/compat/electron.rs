use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

#[derive(Clone, Debug)]
pub struct ElectronCompatStore {
    home: PathBuf,
}

impl ElectronCompatStore {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn home_path(&self) -> &Path {
        &self.home
    }

    pub fn database_path(&self) -> PathBuf {
        self.home.join("distill-electron.db")
    }

    pub fn database_exists(&self) -> bool {
        self.database_path().exists()
    }

    pub fn open_read_only(&self) -> Result<Connection> {
        let path = self.database_path();
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("failed to open SQLite database at {}", path.display()))?;
        Ok(connection)
    }
}
