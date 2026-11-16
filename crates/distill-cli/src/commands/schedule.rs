use anyhow::Result;
use clap::Subcommand;
use distill_core::schedule;

use super::Ctx;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the timer definition without installing it
    Print,
    Install,
    Uninstall,
    Status,
}

pub fn run(ctx: &Ctx, a: Args) -> Result<()> {
    let exe = std::env::current_exe()?;
    let home = &ctx.home;
    match a.cmd {
        Cmd::Print => {
            let c = home.load_config()?;
            if cfg!(target_os = "macos") {
                print!("{}", schedule::launchd_plist(&exe, home, &c));
            } else {
                let (s, t) = schedule::systemd_units(&exe, home, &c);
                println!("# distill-tick.service\n{s}\n# distill-tick.timer\n{t}");
            }
        }
        Cmd::Install => {
            let p = schedule::install(&exe, home, &home.load_config()?)?;
            println!("Installed {}", p.display());
        }
        Cmd::Uninstall => {
            schedule::uninstall()?;
            println!("Removed the Distill timer");
        }
        Cmd::Status => println!(
            "{}",
            if schedule::is_installed() {
                "installed"
            } else {
                "not installed"
            }
        ),
    }
    Ok(())
}
