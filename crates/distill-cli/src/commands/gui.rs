use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::Ctx;

#[derive(clap::Args)]
pub struct Args {}

/// Open the window over this home: `distill-gui` beside this binary (a build or a non-macOS
/// install), else DISTILL.app's window in ~/Applications (macOS, from `scripts/install.sh`),
/// else `distill-gui` on `PATH`. It is told this binary as `DISTILL_CLI`, the command the
/// timer runs.
pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let exe = std::env::current_exe()?;
    let prog = window(&exe);
    let status = std::process::Command::new(&prog)
        .env("DISTILL_HOME", &ctx.home.root)
        .env("DISTILL_CLI", &exe)
        .status()
        .with_context(|| {
            format!(
                "start {} (install it with scripts/install.sh, or build it with `cargo build -p distill-gui`)",
                prog.display()
            )
        })?;
    if !status.success() {
        bail!("the window exited with {status}");
    }
    Ok(())
}

fn window(exe: &std::path::Path) -> PathBuf {
    let beside = exe.with_file_name(if cfg!(windows) {
        "distill-gui.exe"
    } else {
        "distill-gui"
    });
    if beside.exists() {
        return beside;
    }
    if cfg!(target_os = "macos")
        && let Some(home) = std::env::var_os("HOME")
    {
        let app = PathBuf::from(home).join("Applications/DISTILL.app/Contents/MacOS/DISTILL");
        if app.exists() {
            return app;
        }
    }
    PathBuf::from("distill-gui")
}
