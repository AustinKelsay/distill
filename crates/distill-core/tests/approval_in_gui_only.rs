//! Only the GUI approves or denies a reveal request (issue #72, "Reveals"; ADR 0004): an agent
//! with a shell could run a command-line approval itself, and one with the MCP server could call
//! an MCP one. The command line's and the MCP server's code never name `reveal::approve` or
//! `reveal::deny`, by any path.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// The faces that must not approve.
const NOT_THE_GUI: [&str; 2] = ["distill-cli", "distill-mcp"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().filter_map(Result::ok) {
        let p = entry.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Each `crate/file` under `crates` whose code names an approval, sorted.
fn violations(crates: &Path) -> Vec<String> {
    // As whole words: "approves" and "denied" in a sentence are not calls.
    let name = Regex::new(r"\b(approve|deny)\b").unwrap();
    let mut found = Vec::new();
    for krate in NOT_THE_GUI {
        let src = crates.join(krate).join("src");
        let mut files = Vec::new();
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
        for f in files {
            let text = fs::read_to_string(&f).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            if name.is_match(code) {
                let rel = f.strip_prefix(crates).unwrap().to_string_lossy();
                found.push(rel.replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_command_and_no_mcp_tool_approves_or_denies_a_reveal() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let found = violations(crates);
    assert!(
        found.is_empty(),
        "only the GUI approves or denies a reveal request (ADR 0004):\n{}",
        found.join("\n")
    );
}

#[test]
fn a_face_that_approves_is_caught() {
    let tmp = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let p = tmp.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write(
        "distill-cli/src/commands/ok.rs",
        "fn run() { reveal::approve(&home, id) }",
    );
    write(
        "distill-mcp/src/tools/no.rs",
        "use distill_core::reveal::{deny as d};",
    );
    write(
        "distill-mcp/src/tools/fine.rs",
        "// The owner approves or denies it; once approved...",
    );
    write(
        "distill-gui/src/approvals.rs",
        "fn a() { reveal::approve() }",
    );
    assert_eq!(
        violations(tmp.path()),
        [
            "distill-cli/src/commands/ok.rs",
            "distill-mcp/src/tools/no.rs"
        ]
    );
}
