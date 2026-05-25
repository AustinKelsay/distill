use anyhow::{Context, Result};

use crate::config::AppPaths;

pub fn ensure_layout(paths: &AppPaths) -> Result<()> {
    std::fs::create_dir_all(&paths.app_home)
        .with_context(|| format!("failed to create {}", paths.app_home.display()))?;
    std::fs::create_dir_all(&paths.blobs_dir)
        .with_context(|| format!("failed to create {}", paths.blobs_dir.display()))?;
    Ok(())
}
