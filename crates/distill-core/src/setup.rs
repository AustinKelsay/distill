//! What finishing setup does, shared by `distill init` and the GUI.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::export;
use crate::home::Home;
use crate::schedule;

/// Create the home, write the config, and add the default dataset recipe if there is none.
pub fn initialize(home: &Home, config: &Config) -> Result<()> {
    home.ensure()?;
    config.save(&home.config_path())?;
    if export::list_recipes(home)?.is_empty() {
        export::save_recipe(home, "good-sessions", &export::default_recipe())?;
    }
    Ok(())
}

/// The `distill` binary installed next to `exe` (the GUI ships beside it).
pub fn sibling_cli(exe: &Path) -> Result<PathBuf> {
    let cli = exe.with_file_name(if cfg!(windows) {
        "distill.exe"
    } else {
        "distill"
    });
    cli.exists()
        .then_some(cli)
        .with_context(|| format!("the distill CLI was not found next to {}", exe.display()))
}

/// Initialize, then install the timer that runs `distill tick`. Returns the timer's path.
pub fn finish(home: &Home, config: &Config, distill_cli: &Path) -> Result<PathBuf> {
    initialize(home, config)?;
    schedule::install(distill_cli, home, config)
}
