//! The harness pruning check: whether a harness could delete sessions before the sweep has
//! captured them, the one way the archive can lose data.
//!
//! What each of the eleven harnesses prunes, when, and which setting controls it is written
//! down in `docs/harness-pruning.md`, with the date it was checked. This module reads the
//! settings that exist. Each harness keeps its settings beside the folder its sessions are in,
//! so they are found from the harness's roots, and a relocated harness is read where it lives.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::harness::Harness;

/// How soon a harness deletes a session on its own, by its setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retention {
    /// Sessions untouched for this many minutes are deleted. Zero: they are deleted, or never
    /// written, straight away.
    pub minutes: u64,
    /// The setting, as the owner would look for it: the file and the key. For a fixed period,
    /// what it applies to.
    pub setting: String,
    /// The harness hard-codes the period: no setting changes it.
    pub fixed: bool,
    /// What it deletes, as a warning names it: `sessions`, or the files of a session it is
    /// limited to.
    pub deletes: String,
}

/// What the harness's own setting says, where it has one that deletes sessions. `None` when it
/// keeps its sessions until the owner deletes them.
pub fn retention(kind: Harness, roots: &[PathBuf]) -> Option<Retention> {
    match kind {
        Harness::ClaudeCode => claude_code_with(roots, CLAUDE_MANAGED_DIRS),
        // The daemon deletes side-question forks (`sessions/btw/`) whose parent is gone once
        // they are a week old. Not configurable.
        Harness::Droid => Some(Retention {
            minutes: 7 * 24 * 60,
            setting: "orphaned /btw side questions, a fixed 7 days".into(),
            fixed: true,
            deletes: "sessions".into(),
        }),
        Harness::Grok => grok(roots),
        Harness::Hermes => hermes(roots),
        // OpenCode keeps its sessions, but deletes the full tool outputs it spilled beside its
        // database once they are a week old, checking every hour. Not configurable. The sweep
        // captures them, so it must run before they go.
        Harness::OpenCode => Some(opencode(roots)),
        // These keep their sessions until the owner deletes them.
        Harness::Codex
        | Harness::Pi
        | Harness::Cursor
        | Harness::Antigravity
        | Harness::T3Code
        | Harness::Goose => None,
    }
}

/// The folder holding the root called `name`: the harness's own folder, where its settings are.
fn holding(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    roots
        .iter()
        .find(|r| r.file_name().is_some_and(|n| n == name))?
        .parent()
        .map(Path::to_path_buf)
}

fn days(days: f64, setting: String) -> Retention {
    Retention {
        minutes: (days.max(0.0) * 24.0 * 60.0) as u64,
        setting,
        fixed: false,
        deletes: "sessions".into(),
    }
}

/// OpenCode's spilled tool outputs, in `tool-output/` beside each database DISTILL reads.
fn opencode(roots: &[PathBuf]) -> Retention {
    use crate::normalize::opencode::{SPILL_DIR, SPILL_RETENTION_DAYS, database, spill_dir};
    let dirs: Vec<String> = roots
        .iter()
        .map(|r| spill_dir(&database(r)).display().to_string())
        .collect();
    let at = if dirs.is_empty() {
        format!("{SPILL_DIR}/ beside opencode.db")
    } else {
        dirs.join(", ")
    };
    Retention {
        minutes: SPILL_RETENTION_DAYS * 24 * 60,
        setting: format!("{at}, a fixed {SPILL_RETENTION_DAYS} days"),
        fixed: true,
        deletes: "spilled tool outputs".into(),
    }
}

/// Where Claude Code's managed settings (`managed-settings.json` and the drop-ins in
/// `managed-settings.d/`) live. They override the owner's own settings.
const CLAUDE_MANAGED_DIRS: &[&str] = if cfg!(target_os = "macos") {
    &["/Library/Application Support/ClaudeCode"]
} else {
    &["/etc/claude-code"]
};

/// Claude Code deletes transcripts older than `cleanupPeriodDays` (default 30, at least 1) in
/// the background after a session starts, and Desktop sessions older than
/// `desktopSessionCleanupPeriodDays` when that is set. The owner's settings are in
/// `settings.json` in its config folder (`CLAUDE_CONFIG_DIR`, `~/.claude`), which holds
/// `projects/`; managed settings override them. A project's own `.claude/settings.json` can set
/// the period too, and is not read here.
pub(crate) fn claude_code_with<P: AsRef<Path>>(
    roots: &[PathBuf],
    managed_dirs: &[P],
) -> Option<Retention> {
    let user = holding(roots, "projects")?.join("settings.json");
    let mut files = vec![user];
    for dir in managed_dirs {
        let dir = dir.as_ref();
        files.push(dir.join("managed-settings.json"));
        let mut drop_ins: Vec<PathBuf> = std::fs::read_dir(dir.join("managed-settings.d"))
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        drop_ins.retain(|p| p.extension().is_some_and(|e| e == "json"));
        drop_ins.sort();
        files.extend(drop_ins);
    }
    // Later files win, as managed settings and later drop-ins do.
    let mut cleanup = (30.0, None);
    let mut desktop = None;
    for file in &files {
        if let Some(n) = json_number(file, "cleanupPeriodDays") {
            cleanup = (n.max(1.0), Some(file));
        }
        if let Some(n) = json_number(file, "desktopSessionCleanupPeriodDays") {
            desktop = (n > 0.0).then_some((n, file));
        }
    }
    let at = |file: Option<&PathBuf>| {
        file.map(|f| format!(" in {}", f.display()))
            .unwrap_or_else(|| " (unset: 30 days)".into())
    };
    let mut r = days(cleanup.0, format!("cleanupPeriodDays{}", at(cleanup.1)));
    if let Some((n, file)) = desktop
        && n < cleanup.0
    {
        r = days(
            n,
            format!("desktopSessionCleanupPeriodDays{}", at(Some(file))),
        );
    }
    Some(r)
}

/// Grok deletes a session folder whose newest file is older than `[storage] cleanup_ttl_days`
/// when a session starts. Unset or 0 turns it off. The setting is in `config.toml` in its home
/// (`GROK_HOME`, `~/.grok`), which holds `sessions/`.
fn grok(roots: &[PathBuf]) -> Option<Retention> {
    let file = holding(roots, "sessions")?.join("config.toml");
    let text = std::fs::read_to_string(&file).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let ttl = v.get("storage")?.get("cleanup_ttl_days")?;
    let n = ttl.as_integer().map(|i| i as f64).or(ttl.as_float())?;
    (n > 0.0).then(|| {
        days(
            n,
            format!("[storage] cleanup_ttl_days in {}", file.display()),
        )
    })
}

/// Hermes deletes ended sessions inactive for `sessions.retention_days` (default 90) when it
/// starts, at most once a day, but only once `sessions.auto_prune` is on (it is off by
/// default). The settings are in `config.yaml` in its home (`HERMES_HOME`, `~/.hermes`), which
/// holds `sessions/` and `state.db`.
fn hermes(roots: &[PathBuf]) -> Option<Retention> {
    let dir = holding(roots, "sessions").or_else(|| holding(roots, "state.db"))?;
    let file = dir.join("config.yaml");
    let text = std::fs::read_to_string(&file).ok()?;
    let on = yaml_value(&text, "sessions", "auto_prune")?;
    if !matches!(on.to_ascii_lowercase().as_str(), "true" | "yes" | "on") {
        return None;
    }
    let n = match yaml_value(&text, "sessions", "retention_days") {
        Some(v) => v.parse::<f64>().ok()?,
        None => 90.0,
    };
    // A negative number makes Hermes skip the sweep.
    (n >= 0.0).then(|| days(n, format!("sessions.retention_days in {}", file.display())))
}

/// A plain value one level down in a YAML file (`section:` then an indented `key: value`),
/// without its comment or quotes. Enough for the settings read here, not YAML in general.
fn yaml_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let content = line.split(" #").next().unwrap_or(line).trim_end();
        if content.trim().is_empty() || content.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            inside = content.strip_suffix(':') == Some(section);
            continue;
        }
        if inside
            && let Some((k, v)) = content.trim().split_once(':')
            && k.trim() == key
        {
            return Some(v.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    None
}

/// A number at the top level of a JSON file, if the file and the key are there.
fn json_number(file: &Path, key: &str) -> Option<f64> {
    let text = std::fs::read_to_string(file).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get(key)?.as_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60;

    /// A harness folder at `dir/<name>` holding `file` with `text`, and the roots DISTILL reads.
    fn harness_dir(
        dir: &Path,
        name: &str,
        file: &str,
        text: Option<&str>,
        roots: &[&str],
    ) -> Vec<PathBuf> {
        let base = dir.join(name);
        std::fs::create_dir_all(&base).unwrap();
        let _ = std::fs::remove_file(base.join(file));
        if let Some(t) = text {
            std::fs::write(base.join(file), t).unwrap();
        }
        roots.iter().map(|r| base.join(r)).collect()
    }

    fn minutes(kind: Harness, roots: &[PathBuf]) -> Option<u64> {
        retention(kind, roots).map(|r| r.minutes)
    }

    #[test]
    fn grok_deletes_idle_sessions_only_when_cleanup_ttl_days_is_set() {
        let dir = tempfile::tempdir().unwrap();
        let grok = |text| harness_dir(dir.path(), "grok", "config.toml", text, &["sessions"]);
        assert_eq!(minutes(Harness::Grok, &grok(None)), None);
        assert_eq!(
            minutes(
                Harness::Grok,
                &grok(Some("[storage]\ncleanup_ttl_days = 0\n"))
            ),
            None
        );
        let roots = grok(Some("model = \"x\"\n[storage]\ncleanup_ttl_days = 2\n"));
        let r = retention(Harness::Grok, &roots).unwrap();
        assert_eq!(r.minutes, 2 * DAY);
        assert!(r.setting.contains("cleanup_ttl_days"), "{}", r.setting);
    }

    #[test]
    fn hermes_deletes_ended_sessions_only_when_auto_prune_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let hermes = |text| {
            harness_dir(
                dir.path(),
                "hermes",
                "config.yaml",
                text,
                &["sessions", "state.db"],
            )
        };
        assert_eq!(minutes(Harness::Hermes, &hermes(None)), None);
        let off = "sessions:\n  auto_prune: false\n  retention_days: 1\n";
        assert_eq!(minutes(Harness::Hermes, &hermes(Some(off))), None);
        // On, with the default ninety days.
        let on = "model:\n  default: x\nsessions:\n  # opt in\n  auto_prune: true\n";
        assert_eq!(minutes(Harness::Hermes, &hermes(Some(on))), Some(90 * DAY));
        let week = "sessions:\n  auto_prune: yes\n  retention_days: 7  # a week\nother:\n  retention_days: 1\n";
        let r = retention(Harness::Hermes, &hermes(Some(week))).unwrap();
        assert_eq!(r.minutes, 7 * DAY);
        assert!(
            r.setting.contains("sessions.retention_days"),
            "{}",
            r.setting
        );
        let now = "sessions:\n  auto_prune: true\n  retention_days: 0\n";
        assert_eq!(minutes(Harness::Hermes, &hermes(Some(now))), Some(0));
    }

    #[test]
    fn claude_code_deletes_sessions_after_cleanup_period_days_where_managed_settings_win() {
        let dir = tempfile::tempdir().unwrap();
        let claude = |text| harness_dir(dir.path(), "claude", "settings.json", text, &["projects"]);
        let managed = dir.path().join("managed");
        let read = |roots: &[PathBuf]| {
            claude_code_with(roots, std::slice::from_ref(&managed)).map(|r| r.minutes)
        };
        assert_eq!(read(&claude(None)), Some(30 * DAY));
        assert_eq!(
            read(&claude(Some(r#"{"cleanupPeriodDays":5}"#))),
            Some(5 * DAY)
        );
        // Claude Code's minimum is one day.
        assert_eq!(read(&claude(Some(r#"{"cleanupPeriodDays":0}"#))), Some(DAY));
        // Desktop sessions have their own period, when set.
        let both = r#"{"cleanupPeriodDays":60,"desktopSessionCleanupPeriodDays":2}"#;
        assert_eq!(read(&claude(Some(both))), Some(2 * DAY));

        // Managed settings override the owner's, in their file and in the drop-in folder.
        let roots = claude(Some(r#"{"cleanupPeriodDays":60}"#));
        std::fs::create_dir_all(managed.join("managed-settings.d")).unwrap();
        std::fs::write(
            managed.join("managed-settings.json"),
            r#"{"cleanupPeriodDays":20}"#,
        )
        .unwrap();
        assert_eq!(read(&roots), Some(20 * DAY));
        std::fs::write(
            managed.join("managed-settings.d/10-retention.json"),
            r#"{"cleanupPeriodDays":3}"#,
        )
        .unwrap();
        let r = claude_code_with(&roots, std::slice::from_ref(&managed)).unwrap();
        assert_eq!(r.minutes, 3 * DAY);
        assert!(r.setting.contains("10-retention.json"), "{}", r.setting);
        assert!(!r.fixed);
    }

    #[test]
    fn droid_deletes_orphaned_side_questions_after_a_fixed_week() {
        let r = retention(Harness::Droid, &[PathBuf::from("/x/.factory/sessions")]).unwrap();
        assert_eq!(r.minutes, 7 * DAY);
        assert!(r.fixed);
    }

    #[test]
    fn opencode_deletes_spilled_tool_outputs_after_a_fixed_week() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        std::fs::create_dir_all(&data).unwrap();
        // The root is the database, or the folder holding it.
        for root in [data.join("opencode.db"), data.clone()] {
            let r = retention(Harness::OpenCode, std::slice::from_ref(&root)).unwrap();
            assert_eq!(r.minutes, 7 * DAY);
            assert!(r.fixed);
            assert_eq!(r.deletes, "spilled tool outputs");
            assert!(
                r.setting
                    .contains(&data.join("tool-output").display().to_string()),
                "{}",
                r.setting
            );
        }
    }

    #[test]
    fn harnesses_that_keep_their_sessions_have_no_retention() {
        let dir = tempfile::tempdir().unwrap();
        for kind in [
            Harness::Codex,
            Harness::Pi,
            Harness::Cursor,
            Harness::Antigravity,
            Harness::T3Code,
            Harness::Goose,
        ] {
            assert_eq!(
                retention(kind, &[dir.path().join("sessions")]),
                None,
                "{kind}"
            );
        }
    }
}
