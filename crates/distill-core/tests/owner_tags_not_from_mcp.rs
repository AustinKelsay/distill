//! Only the owner adds or hides a tag on a session, from the GUI or the command line (issue
//! #158): an agent that read a session through the MCP server must not be able to relabel the
//! archive, for the same reason only the GUI approves a reveal (ADR 0004). The MCP server's code
//! never names `tag_session`, `OwnerTags` or `owner_tags_path`, by any path.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

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

/// Each `crate/file` of the MCP server whose code names a way to write the owner's tags, sorted.
fn violations(crates: &Path) -> Vec<String> {
    let name = Regex::new(r"\b(tag_session|OwnerTags|owner_tags_path)\b").unwrap();
    let mut files = Vec::new();
    let src = crates.join("distill-mcp").join("src");
    if src.is_dir() {
        rust_files(&src, &mut files);
    }
    let mut found: Vec<String> = files
        .into_iter()
        .filter(|f| {
            let text = fs::read_to_string(f).unwrap();
            name.is_match(text.split("#[cfg(test)]").next().unwrap_or_default())
        })
        .map(|f| {
            let rel = f.strip_prefix(crates).unwrap().to_string_lossy();
            rel.replace('\\', "/")
        })
        .collect();
    found.sort();
    found
}

#[test]
fn no_mcp_tool_adds_or_hides_a_tag() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let found = violations(crates);
    assert!(
        found.is_empty(),
        "only the owner edits tags, from the GUI or the command line (#158):\n{}",
        found.join("\n")
    );
}

#[test]
fn a_tool_that_edits_tags_is_caught() {
    let tmp = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let p = tmp.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write(
        "distill-mcp/src/tools/no.rs",
        "use distill_core::tags::{tag_session as t};",
    );
    write(
        "distill-mcp/src/tools/fine.rs",
        "// The owner tags a session from the window.",
    );
    write(
        "distill-cli/src/commands/tag.rs",
        "fn run() { tags::tag_session(&home, &c, s, &a, &h) }",
    );
    assert_eq!(violations(tmp.path()), ["distill-mcp/src/tools/no.rs"]);
}
