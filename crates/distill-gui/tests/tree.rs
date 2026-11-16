//! What `--tree` dumps of the Sessions scenes: a covered finding's value is never in it (#86).
//! The scenes are fixtures, so the run reads no home; HOME and DISTILL_HOME point at a
//! temporary folder anyway. The values are the fixture's, built from the same pieces.

use std::path::Path;
use std::process::Command;

fn tree(scene: &str, dir: &Path) -> String {
    let out = dir.join(format!("{scene}.json"));
    let run = Command::new(env!("CARGO_BIN_EXE_distill-gui"))
        .args(["--scene", scene, "--tree"])
        .arg(&out)
        .env("HOME", dir)
        .env("DISTILL_HOME", dir.join("distill"))
        .output()
        .expect("run distill-gui");
    assert!(
        run.status.success(),
        "{scene}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::read_to_string(out).expect("a tree")
}

#[test]
fn covered_values_are_not_in_the_tree_dump() {
    let dir = tempfile::tempdir().unwrap();
    let token = ["FAKE", "-TOKEN-", "0000", "-NOT-REAL"].concat();
    let bearer = ["fake", "Bearer", "Value", "0000"].concat();
    let email = ["jo.example", "@", "example.com"].concat();
    let host = ["build-box", ".tail0000", ".ts.net"].concat();
    let person = "Avery Example";

    for scene in ["sessions", "sessions-search", "sessions-open"] {
        let t = tree(scene, dir.path());
        assert!(t.contains('‹'), "{scene} shows chips");
        for v in [&token, &bearer, &email, &host, person] {
            assert!(!t.contains(v), "{scene}: {v:?} in --tree");
        }
    }
    // With the email in the first message shown, it is there and the others are not.
    let t = tree("sessions-open-shown", dir.path());
    assert!(t.contains(&email));
    for v in [&token, &bearer, &host] {
        assert!(!t.contains(v), "{v:?} in --tree");
    }
}
