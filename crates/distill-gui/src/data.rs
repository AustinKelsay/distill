//! Everything the window asks of distill-core, run on one worker thread so no file I/O happens
//! in the frame loop. Handlers queue a [`Request`] in `State::outbox`; `poll` hands the outbox to
//! the worker and lands each [`Reply`] it sends back, woken by Rune's `Waker`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use chrono::{DateTime, Utc};
use distill_core::config::Config;
use distill_core::source::{self, Detection, SourceKind};
use distill_core::stats::{self, Dashboard};
use distill_core::{Home, export, schedule, setup};
use rune::Waker;

/// Who the dashboard's approvals are recorded as.
pub const APPROVED_BY: &str = "dashboard";

/// Something the window wants done off the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    LoadDashboard,
    /// Walk each source's roots for setup's "found" column.
    Detect(Vec<(SourceKind, Vec<PathBuf>)>),
    SaveConfig(Box<Config>),
    Approve {
        recipe: String,
        build: String,
    },
    CheckSchedule,
}

/// What the worker found. Errors arrive as their display text.
#[derive(Debug)]
pub enum Reply {
    Dashboard(Result<Box<Dashboard>, String>),
    Detected(Vec<Detection>),
    Saved(Result<(), String>),
    Approved {
        recipe: String,
        build: String,
        result: Result<(), String>,
    },
    Schedule(bool),
}

/// The two ends of the worker's channels, held in state while a window runs.
pub struct Link {
    pub jobs: Sender<Request>,
    pub replies: Receiver<Reply>,
}

/// Start the worker for `home`. Each reply wakes the window so `poll` lands it.
pub fn spawn(home: Home, waker: Waker) -> Link {
    let (jobs, job_rx) = channel::<Request>();
    let (reply_tx, replies) = channel::<Reply>();
    thread::Builder::new()
        .name("distill-gui-worker".into())
        .spawn(move || {
            for req in job_rx {
                let reply = run(&home, req);
                if reply_tx.send(reply).is_err() {
                    break;
                }
                waker();
            }
        })
        .expect("spawn worker thread");
    Link { jobs, replies }
}

fn run(home: &Home, req: Request) -> Reply {
    match req {
        Request::LoadDashboard => Reply::Dashboard(
            stats::dashboard(home)
                .map(Box::new)
                .map_err(|e| format!("{e:#}")),
        ),
        Request::Detect(sources) => Reply::Detected(
            sources
                .into_iter()
                .map(|(kind, roots)| source::detect(kind, &roots))
                .collect(),
        ),
        // Finishing setup writes the config, adds the default recipe and installs the timer,
        // so the machine starts collecting without another step.
        Request::SaveConfig(config) => Reply::Saved(
            std::env::current_exe()
                .map_err(anyhow::Error::from)
                .and_then(|exe| setup::sibling_cli(&exe))
                .and_then(|cli| setup::finish(home, &config, &cli))
                .map(|_| ())
                .map_err(|e| format!("{e:#}")),
        ),
        Request::Approve { recipe, build } => {
            let result =
                export::approve(home, &recipe, &build, APPROVED_BY).map_err(|e| format!("{e:#}"));
            Reply::Approved {
                recipe,
                build,
                result,
            }
        }
        Request::CheckSchedule => Reply::Schedule(schedule::is_installed()),
    }
}

/// What `main` reads before the window opens: where the home is and its config, if any.
#[derive(Debug, Clone)]
pub struct Boot {
    pub home: Home,
    pub config: Option<Config>,
    pub config_error: Option<String>,
}

impl Boot {
    /// One small file read, done in `main` before any window or frame exists.
    pub fn load() -> anyhow::Result<Boot> {
        let home = Home::resolve(None)?;
        let path = home.config_path();
        let (config, config_error) = if path.exists() {
            match home.load_config() {
                Ok(c) => (Some(c), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            }
        } else {
            (None, None)
        };
        Ok(Boot {
            home,
            config,
            config_error,
        })
    }
}

/// The detect request for every source the config knows.
pub fn detect_request(config: &Config) -> Request {
    Request::Detect(
        SourceKind::ALL
            .iter()
            .map(|&k| (k, config.roots_for(k)))
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Formatting

/// `1.4 GB`, `312 KB`, `0 B`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i + 1 < UNITS.len() {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// `12,345`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// "just now", "12 min ago", "3 h ago", "2 days ago", or the date for older times.
pub fn ago(rfc3339: &str, now: DateTime<Utc>) -> String {
    let Ok(t) = DateTime::parse_from_rfc3339(rfc3339) else {
        return rfc3339.to_string();
    };
    let secs = (now - t.with_timezone(&Utc)).num_seconds().max(0);
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..604_800 => {
            let d = secs / 86_400;
            if d == 1 {
                "yesterday".to_string()
            } else {
                format!("{d} days ago")
            }
        }
        _ => t.format("%b %-d").to_string(),
    }
}

/// `2026-09-07` → `Sep 7`.
pub fn short_date(ymd: &str) -> String {
    chrono::NaiveDate::parse_from_str(ymd, "%Y-%m-%d")
        .map(|d| d.format("%b %-d").to_string())
        .unwrap_or_else(|_| ymd.to_string())
}

/// A path with the user's home folder written as `~`.
pub fn tilde(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().into_owned();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

/// `$2.00`, `$0.04`.
pub fn dollars(v: f64) -> String {
    format!("${v:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_counts_and_ages() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1_450_000_000), "1.4 GB");
        assert_eq!(human_bytes(312_000), "312 KB");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(12), "12");
        let now = DateTime::parse_from_rfc3339("2026-11-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(ago("2026-11-13T11:48:00Z", now), "12 min ago");
        assert_eq!(ago("2026-11-13T09:00:00Z", now), "3 h ago");
        assert_eq!(ago("2026-11-12T09:00:00Z", now), "yesterday");
        assert_eq!(short_date("2026-09-07"), "Sep 7");
    }
}
