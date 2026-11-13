//! Join sessions to git: the commit the repo was at when a session started, the commits made
//! during it, and whether those commits were later reverted. Passive ground truth for quality.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use chrono::{DateTime, Duration};

use crate::model::GitOutcome;

#[derive(Default)]
pub struct Cache {
    /// cwd → repo top level (None when not a repo).
    tops: HashMap<String, Option<String>>,
    /// repo → reverted commit hashes.
    reverts: HashMap<String, Vec<String>>,
}

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Cache {
    fn top(&mut self, cwd: &str) -> Option<String> {
        if let Some(t) = self.tops.get(cwd) {
            return t.clone();
        }
        let top = if Path::new(cwd).is_dir() {
            git(cwd, &["rev-parse", "--show-toplevel"]).filter(|s| !s.is_empty())
        } else {
            None
        };
        self.tops.insert(cwd.to_string(), top.clone());
        top
    }

    fn reverted(&mut self, repo: &str) -> &Vec<String> {
        self.reverts.entry(repo.to_string()).or_insert_with(|| {
            let log = git(
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
        })
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
        let repo = self.top(cwd)?;
        let start = DateTime::parse_from_rfc3339(started).ok()?;
        let end = DateTime::parse_from_rfc3339(ended).ok()? + Duration::hours(1);
        // A branch that no longer exists (merged and deleted) gives no attribution.
        let rev = branch.filter(|b| {
            !b.is_empty()
                && *b != "HEAD"
                && git(&repo, &["rev-parse", "--verify", "--quiet", b]).is_some()
        });
        let Some(rev) = rev else {
            return Some(GitOutcome {
                repo,
                branch: branch.map(str::to_string),
                ..Default::default()
            });
        };
        let sha_at_start = git(
            &repo,
            &[
                "rev-list",
                "-1",
                &format!("--before={}", start.to_rfc3339()),
                rev,
            ],
        )
        .filter(|s| !s.is_empty());
        let commits: Vec<String> = git(
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
        let reverted_list = self.reverted(&repo).clone();
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
        let during = git(d.to_str().unwrap(), &["rev-parse", "HEAD"]).unwrap();
        run(d, &["revert", "--no-edit", "HEAD"], "2026-01-02T10:00:00Z");
        let head = git(d.to_str().unwrap(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
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
