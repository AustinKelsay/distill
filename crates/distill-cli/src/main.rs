//! Distill CLI binary entrypoint.

use std::process::ExitCode;

use distill_cli::{run, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse_from_env();
    run(cli)
}
