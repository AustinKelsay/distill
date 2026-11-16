//! Headless snapshots of every screen: each scene in `fixture::SCENES` is rendered and its
//! element tree (keys, roles and the text shown) compared with `snapshots/<scene>.txt`.
//! `DISTILL_UPDATE_SNAPSHOTS=1 cargo test -p distill-gui` rewrites them. The pictures beside
//! them come from `--snap` (see `snapshots/README.md`) and are for people, not checked here.

use std::path::PathBuf;

use super::*;
use crate::fixture::SCENES;

/// Tall enough that every section of a screen is laid out on screen.
pub const SNAP_SIZE: (f32, f32) = (1080.0, 1600.0);

/// One line per element that shows text or names itself, indented by how many keyed elements
/// hold it.
pub fn dump(rt: &Runtime<State>) -> String {
    let mut out = String::new();
    for e in rt.elements() {
        if e.exiting {
            continue;
        }
        let Some(shown) = e.text.clone().or(e.label.clone()) else {
            continue;
        };
        let depth = e
            .path
            .split('/')
            .filter(|s| !(s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_digit())))
            .count();
        let name = e.key.clone().unwrap_or_else(|| e.role.to_string());
        out.push_str(&format!(
            "{}{name}: {}\n",
            "  ".repeat(depth.saturating_sub(1)),
            shown.replace('\n', "\\n")
        ));
    }
    out
}

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("snapshots")
}

#[test]
fn every_screen_matches_its_snapshot() {
    let update = std::env::var_os("DISTILL_UPDATE_SNAPSHOTS").is_some();
    let user = std::env::var("HOME").unwrap_or_default();
    let mut failed = Vec::new();
    for (name, scene) in SCENES {
        let mut s = crate::fixture::fresh();
        scene(&mut s);
        let rt = runtime_sized(s, SNAP_SIZE);
        let got = dump(&rt);
        assert!(
            user.len() < 2 || !got.contains(&user),
            "{name} shows a path in this machine's home folder"
        );
        let path = snapshot_dir().join(format!("{name}.txt"));
        if update {
            std::fs::create_dir_all(snapshot_dir()).unwrap();
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(want) if want == got => {}
            Ok(_) => failed.push(format!("{name}: differs from {}", path.display())),
            Err(_) => failed.push(format!("{name}: no snapshot at {}", path.display())),
        }
    }
    assert!(
        failed.is_empty(),
        "{}\nrerun with DISTILL_UPDATE_SNAPSHOTS=1 to accept the new screens",
        failed.join("\n")
    );
}

#[test]
fn every_screen_has_a_scene() {
    for entry in crate::nav::SCREENS {
        assert!(
            SCENES.iter().any(|(n, _)| n.starts_with(entry.id)),
            "no scene for {}",
            entry.label
        );
    }
}
