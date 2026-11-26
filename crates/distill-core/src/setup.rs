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
    // A spoke keeps its name and stays a spoke: its hub knows its sessions by that name
    // (ADR 0010).
    if let Some(was) = home.load_config().ok().and_then(|c| c.spoke)
        && config.spoke.as_ref() != Some(&was)
    {
        bail!(
            "this home is the spoke \"{0}\", and a machine keeps its name: run it again with \
             --spoke {0}",
            was.machine
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

/// The `distill` binary the timer runs: `named` when given (`DISTILL_CLI`, which DISTILL.app's
/// Info.plist and `distill gui` set, since the app's window has no `distill` beside it), else
/// the one beside `exe`, where the window ships next to it.
pub fn timer_cli(exe: &Path, named: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(cli) = named {
        return cli.is_file().then_some(cli.clone()).with_context(|| {
            format!(
                "the distill command named by DISTILL_CLI was not found: {}",
                cli.display()
            )
        });
    }
    let cli = exe.with_file_name(if cfg!(windows) {
        "distill.exe"
    } else {
        "distill"
    });
    cli.exists()
        .then_some(cli)
        .with_context(|| format!("the distill command was not found beside {}", exe.display()))
}

/// Install the timer to run the `distill` binary [`timer_cli`] finds.
pub fn install_timer(home: &Home, config: &Config) -> Result<PathBuf> {
    let named = std::env::var_os("DISTILL_CLI").map(PathBuf::from);
    let cli = timer_cli(&std::env::current_exe()?, named)?;
    schedule::install(&cli, home, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_runs_the_named_distill_else_the_one_beside_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("DISTILL.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        let window = app.join("DISTILL");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let cli = bin.join("distill");
        std::fs::write(&cli, "").unwrap();

        // Inside the app nothing is beside the window, so DISTILL_CLI names the command.
        assert!(timer_cli(&window, None).is_err());
        assert_eq!(timer_cli(&window, Some(cli.clone())).unwrap(), cli);
        let missing = timer_cli(&window, Some(bin.join("gone"))).unwrap_err();
        assert!(missing.to_string().contains("DISTILL_CLI"), "{missing}");

        // A window installed beside the command finds it there.
        assert_eq!(timer_cli(&bin.join("distill-gui"), None).unwrap(), cli);
    }
}
