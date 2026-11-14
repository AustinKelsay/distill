//! Join sessions to git: the commit the repo was at when a session started, the commits made
//! during it, and whether those commits were later reverted. Passive ground truth for quality.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use chrono::{DateTime, Duration};

use crate::model::GitOutcome;

/// How long one git command may take. Long enough for `log --all` on a big repository.
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Default)]
pub struct Cache {
    /// cwd → repo top level (None when not a repo).
    tops: HashMap<String, Option<String>>,
    /// repo → reverted commit hashes.
    reverts: HashMap<String, Vec<String>>,
    /// Folders where a git command hung this run; nothing more is run under them.
    stalled: HashSet<String>,
    /// Git commands that timed out this run.
    pub timeouts: u64,
    /// Whether the last [`Cache::outcome`] was cut short by a stalled folder (the session's or
    /// its repository's).
    last_stalled: bool,
    /// Test seam: another program and limit than `git` and [`GIT_TIMEOUT`].
    #[cfg(test)]
    program: Option<(std::path::PathBuf, std::time::Duration)>,
}

/// The folder a hang is charged to. macOS protects `~/Desktop`, `~/Documents` and
/// `~/Downloads` per folder, and a process without access (a launchd job after its binary was
/// replaced, say) blocks inside the kernel on the first read there, with no prompt to answer.
fn stall_scope(dir: &str) -> String {
    dir.split('/').take(4).collect::<Vec<_>>().join("/")
}

impl Cache {
    /// Whether git was given up on, this run, for the folder `dir` is in.
    pub fn stalled_for(&self, dir: &str) -> bool {
        self.stalled.contains(&stall_scope(dir))
    }

    /// Run git in `dir`. A command that hasn't finished within [`GIT_TIMEOUT`] is killed, and
    /// its folder is skipped for the rest of the run, so one blocked folder can't stall a tick.
    fn git(&mut self, dir: &str, args: &[&str]) -> Option<String> {
        let scope = stall_scope(dir);
        if self.stalled.contains(&scope) {
            return None;
        }
        #[cfg(test)]
        let (program, limit) = self
            .program
            .clone()
            .unwrap_or_else(|| ("git".into(), GIT_TIMEOUT));
        #[cfg(not(test))]
        let (program, limit) = ("git", GIT_TIMEOUT);
        let mut child = Command::new(program)
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        // Read on another thread, so a large output can't fill the pipe and stall the child.
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        let deadline = std::time::Instant::now() + limit;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        // The output is waited for under the same deadline: a process git started can keep the
        // pipe open after git itself has exited.
        let out = status.and_then(|_| {
            rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .ok()
        });
        let (Some(status), Some(out)) = (status, out) else {
            self.timeouts += 1;
            self.stalled.insert(scope);
            return None;
        };
        status
            .success()
            .then(|| String::from_utf8_lossy(&out).trim().to_string())
    }

    fn top(&mut self, cwd: &str) -> Option<String> {
        if let Some(t) = self.tops.get(cwd) {
            return t.clone();
        }
        let top = if Path::new(cwd).is_dir() {
            self.git(cwd, &["rev-parse", "--show-toplevel"])
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        self.tops.insert(cwd.to_string(), top.clone());
        top
    }

    fn reverted(&mut self, repo: &str) -> Vec<String> {
        if let Some(r) = self.reverts.get(repo) {
            return r.clone();
        }
        let list: Vec<String> = {
            let log = self
                .git(
                    repo,
                    &[
                        "log",
                        "--all",
                        "--grep=This reverts commit",
                        "--format=%B%x00",
                    ],
                )
                .unwrap_or_default();
            let re =
                regex::Regex::new(r"This reverts commit ([0-9a-f]{7,40})").expect("valid regex");
            re.captures_iter(&log).map(|c| c[1].to_string()).collect()
        };
        self.reverts.insert(repo.to_string(), list.clone());
        list
    }

    /// Commits are read from the session's own branch only. Counting the whole repository
    /// credited sessions with other agents' parallel work.
    pub fn outcome(
        &mut self,
        cwd: &str,
        branch: Option<&str>,
        started: &str,
        ended: &str,
    ) -> Option<GitOutcome> {
        // A command that hung leaves the outcome partial; then there is none, so the caller can
        // keep what an earlier run found.
        // A folder given up on earlier in the run answers nothing at all, which would read as
        // "no such branch"; that is no outcome either.
        let timeouts = self.timeouts;
        let outcome = self.outcome_inner(cwd, branch, started, ended);
        let repo_stalled = self
            .tops
            .get(cwd)
            .cloned()
            .flatten()
            .is_some_and(|r| self.stalled_for(&r));
        let stalled = self.timeouts != timeouts || self.stalled_for(cwd) || repo_stalled;
        self.last_stalled = stalled;
        (!stalled).then_some(outcome).flatten()
    }

    /// Start a new session: what the previous one's lookup ran into says nothing about it.
    pub fn begin_session(&mut self) {
        self.last_stalled = false;
    }

    /// Whether this session's [`Cache::outcome`] couldn't ask git (as opposed to git answering
    /// that there was nothing). False when no lookup ran for it.
    pub fn last_stalled(&self) -> bool {
        self.last_stalled
    }

    fn outcome_inner(
        &mut self,
        cwd: &str,
        branch: Option<&str>,
        started: &str,
        ended: &str,
    ) -> Option<GitOutcome> {
        let repo = self.top(cwd)?;
        let start = DateTime::parse_from_rfc3339(started).ok()?;
        let end = DateTime::parse_from_rfc3339(ended).ok()? + Duration::hours(1);
        // A branch that no longer exists (merged and deleted) gives no attribution.
        let rev = match branch {
            Some(b)
                if !b.is_empty()
                    && b != "HEAD"
                    && self
                        .git(&repo, &["rev-parse", "--verify", "--quiet", b])
                        .is_some() =>
            {
                Some(b)
            }
            _ => None,
        };
        let Some(rev) = rev else {
            return Some(GitOutcome {
                repo,
                branch: branch.map(str::to_string),
                ..Default::default()
            });
        };
        let sha_at_start = self
            .git(
                &repo,
                &[
                    "rev-list",
                    "-1",
                    &format!("--before={}", start.to_rfc3339()),
                    rev,
                ],
            )
            .filter(|s| !s.is_empty());
        let commits: Vec<String> = self
            .git(
                &repo,
                &[
                    "log",
                    rev,
                    "--format=%H",
                    &format!("--since={}", start.to_rfc3339()),
                    &format!("--until={}", end.to_rfc3339()),
                ],
            )
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        let reverted_list = self.reverted(&repo);
        let reverted = commits
            .iter()
            .filter(|c| reverted_list.iter().any(|r| c.starts_with(r.as_str())))
            .count() as u32;
        Some(GitOutcome {
            repo,
            branch: Some(rev.to_string()),
            sha_at_start,
            commits,
            reverted,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hung_git_is_killed_and_its_folder_skipped_for_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("git");
        std::fs::write(&stub, "#!/bin/sh\nsleep 30\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cache = Cache {
            program: Some((stub, std::time::Duration::from_millis(300))),
            ..Default::default()
        };
        let cwd = dir.path().to_string_lossy().into_owned();
        let started = std::time::Instant::now();
        assert!(
            cache
                .outcome(
                    &cwd,
                    Some("main"),
                    "2026-11-01T00:00:00Z",
                    "2026-11-01T01:00:00Z"
                )
                .is_none()
        );
        // The second session in the same folder doesn't wait again.
        assert!(
            cache
                .outcome(
                    &cwd,
                    Some("main"),
                    "2026-11-01T00:00:00Z",
                    "2026-11-01T01:00:00Z"
                )
                .is_none()
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(cache.timeouts, 1);

        // A git that exits at once but leaves a process holding its output open.
        let stub2 = dir.path().join("git2");
        std::fs::write(&stub2, "#!/bin/sh\n(sleep 30) &\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub2, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cache = Cache {
            program: Some((stub2, std::time::Duration::from_millis(300))),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        assert!(cache.git(&cwd, &["rev-parse", "--show-toplevel"]).is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(cache.timeouts, 1);

        // The repository is found, then the next command hangs: no partial outcome.
        let stub3 = dir.path().join("git3");
        std::fs::write(
            &stub3,
            format!("#!/bin/sh\ncase \"$*\" in *--show-toplevel*) echo {cwd};; *) exec sleep 30;; esac\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub3, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cache = Cache {
            program: Some((stub3, std::time::Duration::from_millis(300))),
            ..Default::default()
        };
        assert!(
            cache
                .outcome(
                    &cwd,
                    Some("main"),
                    "2026-11-01T00:00:00Z",
                    "2026-11-01T01:00:00Z"
                )
                .is_none()
        );
        assert!(cache.stalled_for(&cwd));
        assert!(cache.last_stalled());
        // The next session may not look git up at all (a steward session): no stall is carried.
        cache.begin_session();
        assert!(!cache.last_stalled());
        // Later sessions there, with the repository top cached, get no partial outcome either.
        assert!(
            cache
                .outcome(
                    &cwd,
                    Some("main"),
                    "2026-11-02T00:00:00Z",
                    "2026-11-02T01:00:00Z"
                )
                .is_none()
        );

        // The session's folder and its repository fall in different stall scopes (a session
        // run in /tmp inside a repository elsewhere), and the hang is at the repository.
        let repo_s = "/srv/repo-elsewhere".to_string();
        let stub4 = dir.path().join("git4");
        std::fs::write(
            &stub4,
            format!("#!/bin/sh\ncase \"$*\" in *--show-toplevel*) echo {repo_s};; *) exec sleep 30;; esac\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub4, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut cache = Cache {
            program: Some((stub4, std::time::Duration::from_millis(300))),
            ..Default::default()
        };
        let sub_s = "/tmp".to_string();
        assert_ne!(stall_scope(&sub_s), stall_scope(&repo_s));
        assert!(
            cache
                .outcome(
                    &sub_s,
                    Some("main"),
                    "2026-11-01T00:00:00Z",
                    "2026-11-01T01:00:00Z"
                )
                .is_none()
        );
        assert!(cache.last_stalled());
        assert!(
            cache
                .outcome(
                    &sub_s,
                    Some("main"),
                    "2026-11-02T00:00:00Z",
                    "2026-11-02T01:00:00Z"
                )
                .is_none()
        );
        assert!(cache.last_stalled());
    }

    use super::*;

    fn run(dir: &Path, args: &[&str], date: &str) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn finds_commits_in_window_and_reverts() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        run(d, &["init", "-q"], "2026-01-01T00:00:00Z");
        std::fs::write(d.join("a"), "1").unwrap();
        run(d, &["add", "a"], "2026-01-01T09:00:00Z");
        run(d, &["commit", "-q", "-m", "before"], "2026-01-01T09:00:00Z");
        std::fs::write(d.join("a"), "2").unwrap();
        run(
            d,
            &["commit", "-q", "-am", "during"],
            "2026-01-01T10:30:00Z",
        );
        let during = Cache::default()
            .git(d.to_str().unwrap(), &["rev-parse", "HEAD"])
            .unwrap();
        run(d, &["revert", "--no-edit", "HEAD"], "2026-01-02T10:00:00Z");
        let head = Cache::default()
            .git(d.to_str().unwrap(), &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap();
        // Work on another branch in the same window is not credited to this session.
        run(d, &["switch", "-q", "-c", "other"], "2026-01-01T10:40:00Z");
        std::fs::write(d.join("b"), "x").unwrap();
        run(d, &["add", "b"], "2026-01-01T10:40:00Z");
        run(
            d,
            &["commit", "-q", "-m", "elsewhere"],
            "2026-01-01T10:40:00Z",
        );
        let mut cache = Cache::default();
        let o = cache
            .outcome(
                d.to_str().unwrap(),
                Some(&head),
                "2026-01-01T10:00:00Z",
                "2026-01-01T11:00:00Z",
            )
            .unwrap();
        assert_eq!(o.commits, vec![during]);
        assert_eq!(o.reverted, 1);
        assert!(o.sha_at_start.is_some());
        let unknown = cache
            .outcome(
                d.to_str().unwrap(),
                None,
                "2026-01-01T10:00:00Z",
                "2026-01-01T11:00:00Z",
            )
            .unwrap();
        assert!(unknown.commits.is_empty(), "no branch, no attribution");
        assert!(
            cache
                .outcome(
                    "/definitely/not/a/repo",
                    None,
                    "2026-01-01T10:00:00Z",
                    "2026-01-01T11:00:00Z"
                )
                .is_none()
        );
    }
}
