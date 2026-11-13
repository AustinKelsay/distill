//! The timer that runs `distill tick`: a launchd agent on macOS, a systemd user timer on Linux.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::home::{Home, user_home};

pub const LABEL: &str = "dev.distill.tick";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn launchd_plist(exe: &Path, home: &Home, config: &Config) -> String {
    let interval = config.schedule.sweep_every_minutes.max(5) * 60;
    let log = home.runs_dir().join("tick.log");
    let path_env = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>tick</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>DISTILL_HOME</key><string>{home}</string>
    <key>PATH</key><string>{path}</string>
  </dict>
  <key>StartInterval</key><integer>{interval}</integer>
  <key>RunAtLoad</key><true/>
  <key>LowPriorityIO</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        exe = xml_escape(&exe.display().to_string()),
        home = xml_escape(&home.root.display().to_string()),
        path = xml_escape(&path_env),
        log = xml_escape(&log.display().to_string()),
    )
}

pub fn systemd_units(exe: &Path, home: &Home, config: &Config) -> (String, String) {
    let service = format!(
        "[Unit]\nDescription=Distill sweep and steward\n\n[Service]\nType=oneshot\nEnvironment=DISTILL_HOME={}\nExecStart={} tick\nNice=10\n",
        home.root.display(),
        exe.display()
    );
    let timer = format!(
        "[Unit]\nDescription=Run Distill every {m} minutes\n\n[Timer]\nOnBootSec=5min\nOnUnitActiveSec={m}min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        m = config.schedule.sweep_every_minutes.max(5)
    );
    (service, timer)
}

pub fn launchd_path() -> Result<PathBuf> {
    Ok(user_home()?
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// Install and load the timer for the current user.
pub fn install(exe: &Path, home: &Home, config: &Config) -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let path = launchd_path()?;
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, launchd_plist(exe, home, config))?;
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?
            .trim()
            .to_string();
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LABEL}")])
            .status();
        let st = Command::new("launchctl")
            .args(["bootstrap", &format!("gui/{uid}")])
            .arg(&path)
            .status()?;
        if !st.success() {
            bail!("launchctl bootstrap failed");
        }
        Ok(path)
    } else if cfg!(target_os = "linux") {
        let dir = user_home()?.join(".config/systemd/user");
        std::fs::create_dir_all(&dir)?;
        let (service, timer) = systemd_units(exe, home, config);
        std::fs::write(dir.join("distill-tick.service"), service)?;
        std::fs::write(dir.join("distill-tick.timer"), timer)?;
        let st = Command::new("systemctl")
            .args(["--user", "enable", "--now", "distill-tick.timer"])
            .status()?;
        if !st.success() {
            bail!("systemctl enable failed");
        }
        Ok(dir.join("distill-tick.timer"))
    } else {
        bail!("scheduling is supported on macOS and Linux")
    }
}

pub fn uninstall() -> Result<()> {
    if cfg!(target_os = "macos") {
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?
            .trim()
            .to_string();
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LABEL}")])
            .status();
        let path = launchd_path()?;
        if path.exists() {
            std::fs::remove_file(&path).context("remove plist")?;
        }
    } else if cfg!(target_os = "linux") {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "distill-tick.timer"])
            .status();
    }
    Ok(())
}

pub fn is_installed() -> bool {
    if cfg!(target_os = "macos") {
        launchd_path().is_ok_and(|p| p.exists())
    } else {
        user_home().is_ok_and(|h| h.join(".config/systemd/user/distill-tick.timer").exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_tick_on_the_configured_interval() {
        let home = Home::new("/tmp/d & h");
        let mut c = Config::default();
        c.schedule.sweep_every_minutes = 30;
        let p = launchd_plist(Path::new("/usr/local/bin/distill"), &home, &c);
        assert!(p.contains("<integer>1800</integer>"));
        assert!(p.contains("<string>tick</string>"));
        assert!(p.contains("/tmp/d &amp; h"));
    }
}
