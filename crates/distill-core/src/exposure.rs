//! The exposure check (ADR 0005): whether DISTILL is no more exposed than the harness folders.
//!
//! Four rules pass or fail: the home and everything in it are owner-only, the fingerprint key is
//! owner-only, the home is not in a synced folder, and no DISTILL process listens on a network
//! port. Alongside them, as information: whether the disk is encrypted, whether backups are set
//! up and what other copies carry, and the folders the previous build left that hold session-derived data.
//! `distill exposure` prints it, `--fix` tightens an existing home's modes, and the status report
//! carries it.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::home::{Home, user_home};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exposure {
    /// One result per rule, in the order of [`Rule::ALL`].
    pub rules: Vec<RuleResult>,
    /// Whether the disk is encrypted, when this machine can tell. Information, not a rule.
    pub disk_encrypted: Option<bool>,
    /// Whether backup targets are set up, and a reminder that other copies carry the archive.
    pub backups: String,
    /// Folders the previous build left that hold session-derived data, with their modes.
    pub previous_build: Vec<LeftFolder>,
}

impl Exposure {
    /// No rule fails. A rule that could not be checked is not a failure.
    pub fn passes(&self) -> bool {
        self.rules.iter().all(|r| r.outcome != Outcome::Fail)
    }

    /// What the disk's encryption means for the archive, in a sentence.
    pub fn disk(&self) -> &'static str {
        match self.disk_encrypted {
            Some(true) => "The disk is encrypted.",
            Some(false) => {
                "The disk is not encrypted: anyone holding this machine's disk can read the archive."
            }
            None => "DISTILL could not tell whether the disk is encrypted.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: Rule,
    pub outcome: Outcome,
    pub message: String,
    /// What broke the rule: paths with their modes, the synced folder, or the listeners.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The home and every file and folder in it are owner-only.
    OwnerOnly,
    /// The fingerprint key is owner-only.
    KeyOwnerOnly,
    /// The home is not inside a synced folder.
    NotSynced,
    /// No DISTILL process is listening on a network port.
    NoListener,
}

impl Rule {
    pub const ALL: [Rule; 4] = [
        Rule::OwnerOnly,
        Rule::KeyOwnerOnly,
        Rule::NotSynced,
        Rule::NoListener,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Rule::OwnerOnly => "home is owner-only",
            Rule::KeyOwnerOnly => "fingerprint key is owner-only",
            Rule::NotSynced => "home is not in a synced folder",
            Rule::NoListener => "no network listener",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    /// This machine has no way to check it (no `lsof`).
    Unchecked,
}

/// A folder the previous build left that holds session-derived data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeftFolder {
    pub path: String,
    /// Its mode, in octal: `0700`.
    pub mode: String,
    pub owner_only: bool,
    /// What of the sessions it holds.
    pub holds: String,
}

/// What the check asks the machine: which DISTILL processes listen on a port (None when it
/// can't tell), and whether the disk is encrypted.
pub struct Probes {
    pub listeners: fn() -> Option<Vec<String>>,
    pub disk_encrypted: fn() -> Option<bool>,
}

impl Probes {
    pub const SYSTEM: Probes = Probes {
        listeners: system_listeners,
        disk_encrypted: system_disk_encrypted,
    };
}

/// What each of the previous build's folders in a home ([`Home::PREVIOUS_BUILD`]) holds.
fn previous_build_holds(name: &str) -> &'static str {
    match name {
        "exports" => "dataset builds: session text",
        "recipes" => "dataset recipes and the steward's proposals, drawn from cards",
        "steward" => "the steward's working folder and its last answers",
        _ => "the previous build's files",
    }
}

/// The app before that kept every captured line in `~/.distill/distill.db`.
const OLD_APP: (&str, &str) = (".distill", "the old app's database: every captured line");

/// How many offending paths a rule names; the message counts them all.
const NAMED: usize = 20;

/// Check `home` on this machine, with the user home from `HOME`.
pub fn check(home: &Home) -> Exposure {
    let user = user_home().unwrap_or_default();
    check_at(home, &user, &Probes::SYSTEM)
}

/// Check `home`, with `user` as the user home the synced folders and the old app's folder are
/// under.
pub fn check_at(home: &Home, user: &Path, probes: &Probes) -> Exposure {
    Exposure {
        rules: vec![
            owner_only(home),
            key_owner_only(home),
            not_synced(home, user),
            no_listener(probes),
        ],
        disk_encrypted: (probes.disk_encrypted)(),
        backups: backups_note(home),
        previous_build: previous_build(home, user),
    }
}

/// Where the archive's copies are, and what they carry.
fn backups_note(home: &Home) -> String {
    let other = "Any other copy of the home, such as a disk backup, carries the archive readable, \
                 every secret in it included: keep it encrypted too.";
    match home.load_config().map(|c| c.backups.len()) {
        Err(_) => {
            format!("config.toml could not be read to tell whether backups are set up. {other}")
        }
        Ok(0) => format!(
            "No backup target is set up in config.toml, so DISTILL keeps no copy of the raw archive (`distill backup`). {other}"
        ),
        Ok(n) => format!(
            "Backups to {} are encrypted on this machine before they leave it. {other}",
            crate::util::count(n as u64, "target")
        ),
    }
}

/// Take the group's and others' permissions off the home and everything in it. Links are left
/// alone, and so is what they point to. Returns how many files and folders changed.
pub fn fix(home: &Home) -> Result<u64> {
    let mut fixed = 0;
    for (path, mode) in entries(home)? {
        if loose(mode) {
            set_mode(&path, mode & 0o700).with_context(|| format!("tighten {}", path.display()))?;
            fixed += 1;
        }
    }
    Ok(fixed)
}

fn loose(mode: u32) -> bool {
    mode & 0o077 != 0
}

/// The home and every file and folder in it, with their permission bits, skipping links. A
/// missing home has none.
fn entries(home: &Home) -> Result<Vec<(PathBuf, u32)>> {
    if !home.root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(&home.root) {
        let entry = entry.with_context(|| format!("read {}", home.root.display()))?;
        if entry.depth() > 0 && entry.path_is_symlink() {
            continue;
        }
        let meta = entry.metadata()?;
        out.push((entry.into_path(), mode_of(&meta)));
    }
    Ok(out)
}

fn owner_only(home: &Home) -> RuleResult {
    let entries = match entries(home) {
        Ok(e) => e,
        Err(e) => {
            return RuleResult {
                rule: Rule::OwnerOnly,
                outcome: Outcome::Fail,
                message: format!("Could not read the whole home: {e:#}."),
                paths: Vec::new(),
            };
        }
    };
    if entries.is_empty() {
        return pass(Rule::OwnerOnly, "There is no home here yet.");
    }
    let open: Vec<_> = entries.iter().filter(|(_, m)| loose(*m)).collect();
    if open.is_empty() {
        return pass(
            Rule::OwnerOnly,
            "The home and everything in it are readable by the owner alone.",
        );
    }
    RuleResult {
        rule: Rule::OwnerOnly,
        outcome: Outcome::Fail,
        message: format!(
            "{} in the home can be read or entered by other accounts: run `distill exposure --fix`.",
            match open.len() {
                1 => "1 file or folder".to_string(),
                n => format!("{n} files and folders"),
            }
        ),
        paths: open
            .iter()
            .take(NAMED)
            .map(|(p, m)| format!("{} ({})", p.display(), octal(*m)))
            .collect(),
    }
}

fn key_owner_only(home: &Home) -> RuleResult {
    let path = home.fingerprint_key_path();
    let Ok(meta) = fs::metadata(&path) else {
        if home.load_config().is_ok_and(|c| c.spoke.is_some()) {
            return pass(
                Rule::KeyOwnerOnly,
                "A spoke has no fingerprint key: it never scans, and the hub fingerprints what it pulls.",
            );
        }
        return pass(
            Rule::KeyOwnerOnly,
            "There is no fingerprint key yet; the first scan makes one, owner-only.",
        );
    };
    let mode = mode_of(&meta);
    if !loose(mode) {
        return pass(
            Rule::KeyOwnerOnly,
            "The fingerprint key is readable by the owner alone.",
        );
    }
    RuleResult {
        rule: Rule::KeyOwnerOnly,
        outcome: Outcome::Fail,
        message: format!(
            "The fingerprint key ({}) can be read by other accounts, and with it any fingerprint can be tested against a guess: run `distill exposure --fix`.",
            octal(mode)
        ),
        paths: vec![path.display().to_string()],
    }
}

fn not_synced(home: &Home, user: &Path) -> RuleResult {
    match synced_folder(&home.root, user) {
        None => pass(Rule::NotSynced, "The home is not in a synced folder."),
        Some(folder) => RuleResult {
            rule: Rule::NotSynced,
            outcome: Outcome::Fail,
            message: "The home is inside a synced folder, so the archive is uploaded with it: move the home to a folder that does not sync.".into(),
            paths: vec![folder.display().to_string()],
        },
    }
}

/// The synced folder `path` is in, if any: one under `~/Library/Mobile Documents` or
/// `~/Library/CloudStorage`, or a Dropbox, Google Drive or OneDrive folder in the user home
/// (spec, "Exposure"). Links are resolved first, so a link into a synced folder is in it.
fn synced_folder(path: &Path, user: &Path) -> Option<PathBuf> {
    if user.as_os_str().is_empty() {
        return None;
    }
    let user = resolved(user);
    let rel = resolved(path).strip_prefix(&user).ok()?.to_path_buf();
    let names: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    match names.as_slice() {
        [lib, store, ..]
            if lib == "Library" && (store == "Mobile Documents" || store == "CloudStorage") =>
        {
            Some(user.join(lib).join(store))
        }
        [first, ..] => {
            let lower = first.to_lowercase();
            ["dropbox", "google drive", "onedrive"]
                .iter()
                .any(|s| lower == *s || lower.starts_with(&format!("{s} ")))
                .then(|| user.join(first))
        }
        [] => None,
    }
}

/// `path` with its links resolved, as far as it exists; the part that doesn't exist yet is kept
/// as written.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |p, n| p.join(n));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn no_listener(probes: &Probes) -> RuleResult {
    match (probes.listeners)() {
        None => RuleResult {
            rule: Rule::NoListener,
            outcome: Outcome::Unchecked,
            message: "Could not list network listeners: `lsof` is not installed.".into(),
            paths: Vec::new(),
        },
        Some(found) if found.is_empty() => pass(
            Rule::NoListener,
            "No DISTILL process is listening on a network port.",
        ),
        Some(found) => RuleResult {
            rule: Rule::NoListener,
            outcome: Outcome::Fail,
            message: "A DISTILL process is listening on a network port. DISTILL never opens one: stop the process.".into(),
            paths: found,
        },
    }
}

fn previous_build(home: &Home, user: &Path) -> Vec<LeftFolder> {
    let in_home = home
        .previous_build_dirs()
        .map(|(name, path)| (path, previous_build_holds(name)));
    let old_app = std::iter::once((user.join(OLD_APP.0), OLD_APP.1));
    in_home
        .chain(old_app)
        .filter_map(|(path, holds)| {
            let meta = fs::symlink_metadata(&path).ok()?;
            let mode = mode_of(&meta);
            Some(LeftFolder {
                path: path.display().to_string(),
                mode: octal(mode),
                owner_only: !loose(mode),
                holds: holds.into(),
            })
        })
        .collect()
}

fn pass(rule: Rule, message: &str) -> RuleResult {
    RuleResult {
        rule,
        outcome: Outcome::Pass,
        message: message.into(),
        paths: Vec::new(),
    }
}

fn octal(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Network listeners held by DISTILL's processes (`distill`, `distill-mcp`, `distill-gui`, and
/// `DISTILL`, the window inside DISTILL.app), from `lsof`. None when `lsof` is not on this machine.
fn system_listeners() -> Option<Vec<String>> {
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof", "/bin/lsof"]
        .into_iter()
        .find(|p| Path::new(p).exists())?;
    let out = Command::new(lsof)
        .args(["-nP", "+c", "0", "-a", "-c", "/^distill/i", "-i"])
        .output()
        .ok()?;
    Some(listeners_in(&String::from_utf8_lossy(&out.stdout)))
}

/// The listening sockets in `lsof -nP +c 0 -i` output that belong to a DISTILL process: a TCP
/// socket in `LISTEN`, or a UDP socket with no peer.
fn listeners_in(lsof: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in lsof.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let [command, pid, _, _, _, _, _, node, name @ ..] = cols.as_slice() else {
            continue;
        };
        if *command != "distill" && *command != "DISTILL" && !command.starts_with("distill-") {
            continue;
        }
        let name = name.join(" ");
        let addr = match *node {
            "TCP" => name.strip_suffix(" (LISTEN)"),
            "UDP" if !name.contains("->") => Some(name.as_str()),
            _ => None,
        };
        if let Some(addr) = addr {
            out.push(format!("{command} (pid {pid}) on {addr}"));
        }
    }
    out
}

/// Whether the disk is encrypted: FileVault on macOS, a dm-crypt device on Linux. None when this
/// machine can't tell.
fn system_disk_encrypted() -> Option<bool> {
    if cfg!(target_os = "macos") {
        let out = Command::new("/usr/bin/fdesetup")
            .arg("status")
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        if text.contains("FileVault is On") {
            Some(true)
        } else if text.contains("FileVault is Off") {
            Some(false)
        } else {
            None
        }
    } else {
        // Without a dm-crypt device the disk may still be encrypted some other way.
        let crypt = fs::read_dir("/sys/block").ok()?.flatten().any(|d| {
            fs::read_to_string(d.path().join("dm/uuid")).is_ok_and(|u| u.starts_with("CRYPT-"))
        });
        crypt.then_some(true)
    }
}

#[cfg(test)]
mod tests;
