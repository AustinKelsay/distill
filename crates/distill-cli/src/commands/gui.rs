use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::Ctx;

#[derive(clap::Args)]
pub struct Args {}

/// Start `distill-gui` (installed beside this binary, else on `PATH`) over this home.
pub fn run(ctx: &Ctx, _: Args) -> Result<()> {
    let exe = std::env::current_exe()?;
    let gui = exe.with_file_name(if cfg!(windows) {
        "distill-gui.exe"
    } else {
        "distill-gui"
    });
    let prog = if gui.exists() {
        gui
    } else {
        PathBuf::from("distill-gui")
    };
    let status = std::process::Command::new(&prog)
        .env("DISTILL_HOME", &ctx.home.root)
        .status()
        .with_context(|| {
            format!(
                "start {} (build it with `cargo build -p distill-gui`)",
                prog.display()
            )
        })?;
    if !status.success() {
        bail!("distill-gui exited with {status}");
    }
    Ok(())
}
