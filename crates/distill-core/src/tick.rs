//! One scheduled pass: sweep, normalize, let the steward work if it is due, rebuild datasets.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Harness};
use crate::export;
use crate::home::Home;
use crate::normalize::{self, NormalizeReport};
use crate::propose;
use crate::runlog;
use crate::steward::{self, RunOptions, StewardReport};
use crate::sweep::{self, SweepReport};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickReport {
    pub sweep: SweepReport,
    pub normalize: NormalizeReport,
    pub steward: Option<StewardReport>,
    /// Recipes the steward proposed this tick, waiting to be accepted.
    #[serde(default)]
    pub proposed: Vec<String>,
    pub builds: Vec<String>,
    pub errors: Vec<String>,
}

pub fn steward_due(home: &Home, config: &Config) -> Result<bool> {
    if config.steward.harness == Harness::None {
        return Ok(false);
    }
    let Some(last) = runlog::last_ok(home, "steward")? else {
        return Ok(true);
    };
    let Ok(last) = DateTime::parse_from_rfc3339(&last) else {
        return Ok(true);
    };
    Ok(Utc::now() - last.with_timezone(&Utc)
        >= Duration::hours(config.schedule.enrich_every_hours as i64))
}

pub fn tick(home: &Home, config: &Config, force_steward: bool) -> Result<TickReport> {
    let mut report = TickReport {
        sweep: sweep::sweep(home, config, None)?,
        ..Default::default()
    };
    report.normalize = normalize::normalize(home, false)?;
    if force_steward || steward_due(home, config)? {
        // Weekly: ask which datasets are worth making. First, so a card backlog can't spend
        // the day's budget or calls before it gets its one call.
        if propose::due(home, config) {
            match propose::propose(home, config) {
                Ok(r) => report.proposed = r.proposed.into_iter().map(|p| p.name).collect(),
                Err(e) => report.errors.push(format!("propose: {e:#}")),
            }
        }
        match steward::run(home, config, &RunOptions::default()) {
            Ok(r) => report.steward = Some(r),
            Err(e) => report.errors.push(format!("steward: {e:#}")),
        }
    }
    for name in export::list_recipes(home)? {
        match export::build(home, &name) {
            Ok(b) => report.builds.push(format!("{}/{}", b.recipe, b.build)),
            Err(e) => report.errors.push(format!("recipe {name}: {e:#}")),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;
    use crate::source::SourceKind;

    #[test]
    fn a_due_proposal_gets_its_call_before_the_card_backlog() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            let s = r#"{"type":"user","cwd":"/w","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"task"}}
"#;
            archive
                .put(
                    SourceKind::ClaudeCode,
                    "/u/.claude/projects/-w/s.jsonl",
                    "f",
                    s.as_bytes(),
                )
                .unwrap();
        }
        let stub = dir.path().join("stub.sh");
        std::fs::write(&stub, "#!/bin/sh\ncat > /dev/null\necho '[]'\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut config = Config::default();
        // Nothing on this machine is swept; the archive above is the backlog.
        for s in &mut config.sources {
            s.enabled = false;
        }
        config.steward.harness = Harness::Command;
        config.steward.command = vec![stub.to_string_lossy().into_owned()];
        config.steward.max_sessions_per_run = 1;

        let r = tick(&home, &config, true).unwrap();
        assert!(
            r.errors.iter().all(|e| !e.starts_with("propose")),
            "{:?}",
            r.errors
        );
        let today = runlog::today(&home).unwrap();
        assert!(today.iter().any(|e| e.step == "propose" && e.ok));
        assert_eq!(
            r.steward.unwrap().carded,
            0,
            "the one call went to the proposal"
        );
    }
}
