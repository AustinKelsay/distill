//! `CLAUDE.md` and `AGENTS.md` are the same guide for different agents (#73), so an edit to one
//! that misses the other fails here.

use std::path::Path;

#[test]
fn claude_md_and_agents_md_are_the_same_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |name: &str| std::fs::read_to_string(root.join(name)).expect(name);
    assert!(
        read("CLAUDE.md") == read("AGENTS.md"),
        "CLAUDE.md and AGENTS.md differ: copy the edited one over the other"
    );
}
