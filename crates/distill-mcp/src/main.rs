use distill_core::Home;

fn main() -> anyhow::Result<()> {
    let home = Home::resolve(None)?;
    let stdin = std::io::stdin();
    distill_mcp::serve(&home, stdin.lock(), std::io::stdout())
}
