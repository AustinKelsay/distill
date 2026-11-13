//! One scheduled pass: sweep, normalize, let the steward work if it is due, rebuild datasets.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Harness};
use crate::export;
use crate::home::Home;
use crate::normalize::{self, NormalizeReport};
use crate::runlog;
use crate::steward::{self, RunOptions, StewardReport};
use crate::sweep::{self, SweepReport};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickReport {
    pub sweep: SweepReport,
    pub normalize: NormalizeReport,
    pub steward: Option<StewardReport>,
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
