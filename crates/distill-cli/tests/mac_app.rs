//! `scripts/mac-app.sh` builds DISTILL.app (macOS only): named DISTILL, its window as the
//! executable, an icon, the distill command in LSEnvironment, and a valid signature. A stand-in
//! script plays the window, so nothing is launched.

#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::Command;

fn plist(app: &Path, key: &str) -> String {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{key}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn the_app_bundle_is_named_distill_and_names_the_command_for_the_timer() {
    let dir = tempfile::tempdir().unwrap();
    let window = dir.path().join("distill-gui");
    std::fs::write(&window, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&window, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = Command::new(repo.join("scripts/mac-app.sh"))
        .arg(&window)
        .arg(dir.path())
        .arg("/opt/tools/bin/distill")
        .status()
        .unwrap();
    assert!(status.success());

    let app = dir.path().join("DISTILL.app");
    assert_eq!(plist(&app, "CFBundleName"), "DISTILL");
    assert_eq!(plist(&app, "CFBundleIdentifier"), "dev.distill.app");
    assert_eq!(plist(&app, "CFBundleExecutable"), "DISTILL");
    assert_eq!(
        plist(&app, "LSEnvironment:DISTILL_CLI"),
        "/opt/tools/bin/distill"
    );
    assert!(app.join("Contents/MacOS/DISTILL").is_file());
    assert!(app.join("Contents/Resources/DISTILL.icns").is_file());
    let verify = Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(&app)
        .status()
        .unwrap();
    assert!(verify.success(), "the bundle's signature verifies");
}
