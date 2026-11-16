use distill_core::Home;
use distill_core::runtime::LocalRuntime;

fn main() -> anyhow::Result<()> {
    let reader = match distill_mcp::reader_from_args(std::env::args().skip(1)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("distill-mcp: {e:#}");
            std::process::exit(2);
        }
    };
    let server = distill_mcp::Server {
        home: Home::resolve(None)?,
        runtime: LocalRuntime::ollama(),
        reader,
    };
    let stdin = std::io::stdin();
    server.serve(stdin.lock(), std::io::stdout())
}
