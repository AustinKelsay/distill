//! Backup targets of any kind but `folder` are reached through an adapter, `distill-backup-<kind>`
//! on PATH (issue #177, `docs/backup-adapters.md`). This test puts the reference adapter, which
//! keeps objects in a folder, on PATH. It is the only test in this binary, so changing the
//! environment races nothing.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use distill_core::archive::Archive;
use distill_core::backup::{self, BackupKey, store};
use distill_core::config::BackupTarget;
use distill_core::{Config, Harness, Home};

fn adapter(bin: &Path, kind: &str, script: &str) {
    let path = bin.join(format!("distill-backup-{kind}"));
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn target(kind: &str, dir: &Path) -> BackupTarget {
    toml::from_str(&format!(
        "name = \"offsite\"\nkind = \"{kind}\"\ndir = \"{}\"\n",
        dir.display()
    ))
    .unwrap()
}

#[test]
fn an_adapter_target_is_tested_backed_up_verified_and_restored_like_a_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let reference = include_str!("fixtures/distill-backup-reference");
    adapter(&bin, "reference", reference);
    adapter(&bin, "broken", "#!/bin/sh\nread line\nexit 3\n");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // SAFETY: the only test in this binary; nothing else reads the environment concurrently.
    unsafe { std::env::set_var("PATH", path) };

    let home = Home::new(tmp.path().join("home"));
    home.ensure().unwrap();
    let mut archive = Archive::open(&home).unwrap();
    archive
        .put(Harness::Codex, "/s/a.jsonl", "1", &b"a session\n"[..])
        .unwrap();
    let place = tmp.path().join("offsite");
    fs::create_dir(&place).unwrap();
    let t = target("reference", &place);
    let config = Config {
        backups: vec![t.clone()],
        ..Default::default()
    };
    config.save(&home.config_path()).unwrap();
    let key = BackupKey::create(&home).unwrap();

    // The adapter's own setting goes back into config.toml as written.
    let saved = home.load_config().unwrap();
    assert_eq!(saved.backups, vec![t.clone()]);

    // Setup's Test reaches the adapter and makes nothing; a target never used isn't started.
    assert_eq!(
        backup::test(&home, &t).unwrap(),
        "Ready. Nothing is copied until the first backup."
    );
    assert!(!place.join("objects").exists());
    assert!(store::open(&t, false).is_err(), "not set up yet");

    let r = backup::run(&home, &t).unwrap();
    assert!(r.ok && r.blobs_sent == 1, "{r:?}");
    assert!(
        backup::test(&home, &t)
            .unwrap()
            .contains("holds this home's run 0"),
        "a test reads the target's records"
    );
    let s = store::open(&t, false).unwrap();
    let check = backup::verify(&home, "offsite", &*s, &key, true).unwrap();
    assert!(check.ok(), "{check:?}");

    let restored = Home::new(tmp.path().join("restored"));
    let report = backup::restore(&restored, &*s, &key, false).unwrap();
    assert_eq!(report.blobs, 1, "{report:?}");
    drop(s);

    // What goes wrong says which adapter and why.
    let err = |t: &BackupTarget| format!("{:#}", store::open(t, false).err().unwrap());
    let missing = err(&target("nowhere", &place));
    assert!(
        missing.contains("no `distill-backup-nowhere` on PATH"),
        "{missing}"
    );
    let gone = err(&target("reference", &tmp.path().join("unplugged")));
    assert!(
        gone.contains("distill-backup-reference:") && gone.contains("unplugged is not there"),
        "the adapter's own error line: {gone}"
    );
    let broken = err(&target("broken", &place));
    assert!(
        broken.contains("distill-backup-broken stopped without answering"),
        "{broken}"
    );
    let odd = err(&target("../reference", &place));
    assert!(odd.contains("not a backup kind"), "{odd}");
}
