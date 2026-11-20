//! No part of DISTILL opens a network listener (spec, "Exposure"; ADR 0005). The exposure check
//! confirms it for the processes running now; this keeps code that could open one out of every
//! crate. Tests may bind a port, to have one that refuses connections.

use std::fs;
use std::path::{Path, PathBuf};

/// What binds a port to listen on.
const LISTENERS: [&str; 2] = ["TcpListener", "UdpSocket"];

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

#[test]
fn no_crate_opens_a_network_listener() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut found = Vec::new();
    for entry in fs::read_dir(crates).unwrap().filter_map(Result::ok) {
        let mut files = Vec::new();
        let src = entry.path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
        for f in files {
            if f.file_name().is_some_and(|n| n == "tests.rs") {
                continue;
            }
            let text = fs::read_to_string(&f).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for l in LISTENERS {
                if code.contains(l) {
                    found.push(format!(
                        "{}: {l}",
                        f.strip_prefix(crates).unwrap().display()
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "these could open a network listener:\n{}",
        found.join("\n")
    );
}
