//! No code path but the read path returns session text (issue #75; ADR 0002). The faces' code
//! (the command line, the MCP server, the GUI) never names what reads stored text directly, and
//! inside core only the parsers, the archive and the read path do. Tests may: they check what
//! the parse step wrote. Core also keeps those readers
//! crate-private, so a face that called one would not compile; this catches the ways around
//! that, such as joining `events.jsonl` onto a path by hand.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// What reads stored session text or captures directly.
const DIRECT_READERS: [&str; 7] = [
    "events_path",
    "read_events",
    "events.jsonl",
    "read_blob",
    "blob_path",
    "normalize::parse",
    "raw_dir",
];

/// Files in core's `src/` allowed to name a direct reader: where they are defined, the parse step
/// that writes events, the archive, and the read path.
const CORE_ALLOWED: [&str; 4] = ["home.rs", "archive.rs", "normalize/", "read.rs"];

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

/// Each `crate/file: reader` under `crates` that names a direct reader where it may not, sorted.
fn violations(crates: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(crates).unwrap().filter_map(Result::ok) {
        let krate = entry.file_name().to_string_lossy().into_owned();
        let src = entry.path().join("src");
        let mut files = Vec::new();
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
        for f in files {
            let rel = f
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if krate == "distill-core" && CORE_ALLOWED.iter().any(|a| rel.starts_with(a)) {
                continue;
            }
            let text = fs::read_to_string(&f).unwrap();
            // A file's unit tests, like the test files, may check what the parse step wrote.
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for r in DIRECT_READERS {
                // As a whole name: `parser_version` is not `normalize::parse`.
                let name = Regex::new(&format!(r"\b{}\b", regex::escape(r))).unwrap();
                if name.is_match(code) {
                    found.push(format!("{krate}/src/{rel}: {r}"));
                }
            }
        }
    }
    found.sort();
    found
}

#[test]
fn only_the_read_path_reads_events() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let found = violations(crates);
    assert!(
        found.is_empty(),
        "these read stored session text without the read path (use distill_core::read):\n{}",
        found.join("\n")
    );
}

#[test]
fn a_face_that_reads_events_by_hand_is_caught() {
    let tmp = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let p = tmp.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    write(
        "distill-mcp/src/tools/peek.rs",
        r#"fn peek(d: &std::path::Path) { let _ = std::fs::read(d.join("events.jsonl")); }"#,
    );
    write(
        "distill-gui/src/screen.rs",
        "fn s() { home.events_path(h, id) }",
    );
    write("distill-core/src/read.rs", "fn a() { read_events() }");
    write("distill-core/src/search.rs", "fn b() { read_events() }");
    write("distill-core/tests/t.rs", "fn c() { read_events() }");
    write(
        "distill-cli/src/x.rs",
        "fn d() { normalize::parser_version(k) }",
    );

    assert_eq!(
        violations(tmp.path()),
        [
            "distill-core/src/search.rs: read_events",
            "distill-gui/src/screen.rs: events_path",
            "distill-mcp/src/tools/peek.rs: events.jsonl",
        ]
    );
}
