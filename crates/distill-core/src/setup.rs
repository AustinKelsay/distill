//! What `distill init` and the GUI's setup do: create the home, write a config and, when setup
//! finishes, install the timer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::harness::Harness;
use crate::home::Home;
use crate::schedule;

/// The default config with each harness enabled when its files are on this machine.
pub fn detected_config() -> Config {
    let mut c = Config::default();
    for kind in Harness::ALL {
        let found = c.roots_for(kind).iter().any(|r| r.exists());
        c.harness_mut(kind).enabled = found;
    }
    c
}

/// Create the home and write `config`. An existing config is kept unless `force` is set.
pub fn initialize(home: &Home, config: &Config, force: bool) -> Result<()> {
    if home.config_path().exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            home.config_path().display()
        );
    }
    home.ensure()?;
    config.save(&home.config_path())
}

/// Finish setup: create the home, write `config` over any config there, then install the timer
/// with `install`. Returns where the timer was installed. The GUI passes [`install_timer`];
/// tests pass a stand-in, because installing replaces the live `dev.distill.tick` job whatever
/// `HOME` is.
pub fn finish(
    home: &Home,
    config: &Config,
    install: impl FnOnce(&Home, &Config) -> Result<PathBuf>,
) -> Result<PathBuf> {
    initialize(home, config, true)?;
    install(home, config)
}

/// The `distill` binary beside `exe`: the GUI ships next to it, and the timer runs it.
pub fn sibling_cli(exe: &Path) -> Result<PathBuf> {
    let cli = exe.with_file_name(if cfg!(windows) {
        "distill.exe"
    } else {
        "distill"
    });
    cli.exists()
        .then_some(cli)
        .with_context(|| format!("the distill command was not found beside {}", exe.display()))
}

/// Install the timer to run the `distill` binary beside the running one.
pub fn install_timer(home: &Home, config: &Config) -> Result<PathBuf> {
    let cli = sibling_cli(&std::env::current_exe()?)?;
    schedule::install(&cli, home, config)
}
