//! Finishing setup writes the config and installs the timer. Installing replaces the live
//! `dev.distill.tick` job whatever HOME is, so this test puts stand-ins for `launchctl` and
//! `systemctl` first on PATH and points HOME at a temporary folder. It is the only test in this
//! binary, so changing the environment races nothing.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use distill_core::config::Config;
use distill_core::{Harness, Home, schedule, setup};

fn stub(dir: &Path, name: &str, log: &Path) {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho \"{name} $*\" >> '{}'\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn finishing_setup_writes_the_config_with_the_spec_names_and_installs_the_timer() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.path().join("calls.log");
    stub(&bin, "launchctl", &log);
    stub(&bin, "systemctl", &log);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // SAFETY: the only test in this binary; nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("HOME", &user);
        std::env::set_var("PATH", path);
    }

    let home = Home::new(dir.path().join("home"));
    let mut config = Config::default();
    config.harness_mut(Harness::Codex).enabled = false;
    config.privacy.personal_terms = vec!["Acme".into()];
    config.steward.card_model = Some("small:1b".into());
    config.schedule.sweep_every_minutes = 30;
    distill_core::trust::add(&mut config, "anthropic", true).unwrap();

    let cli = Path::new("/opt/distill/bin/distill");
    let timer = setup::finish(&home, &config, |h, c| schedule::install(cli, h, c)).unwrap();

    let text = std::fs::read_to_string(home.config_path()).unwrap();
    for name in [
        "[[harnesses]]",
        "[privacy]",
        "personal_terms",
        "[[providers]]",
        "[steward]",
        "card_model",
        "[schedule]",
        "sweep_every_minutes = 30",
    ] {
        assert!(text.contains(name), "{name} in {text}");
    }
    assert_eq!(Config::load(&home.config_path()).unwrap(), config);

    assert!(timer.starts_with(&user), "{}", timer.display());
    let unit = std::fs::read_to_string(&timer).unwrap();
    assert!(unit.contains(&home.root.display().to_string()) || cfg!(target_os = "linux"));
    let calls = std::fs::read_to_string(&log).unwrap();
    if cfg!(target_os = "macos") {
        assert!(calls.contains("launchctl bootstrap"), "{calls}");
    } else {
        assert!(calls.contains("systemctl --user enable"), "{calls}");
    }

    // A second finish (setup opened again later) overwrites the config and reinstalls.
    config.schedule.sweep_every_minutes = 15;
    setup::finish(&home, &config, |h, c| schedule::install(cli, h, c)).unwrap();
    assert_eq!(Config::load(&home.config_path()).unwrap(), config);
}
