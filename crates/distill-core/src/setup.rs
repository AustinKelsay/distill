//! What `distill init` does: create the home and write a config.

use anyhow::{Result, bail};

use crate::config::Config;
use crate::harness::Harness;
use crate::home::Home;

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
