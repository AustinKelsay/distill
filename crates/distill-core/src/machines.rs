//! A hub pulling its spokes' new captures over SSH, and a spoke's half of it, the export
//! (ADR 0010, issue #138). Neither side listens: the hub runs `ssh <host> distill export --since
//! <bytes>`, where `<bytes>` is the length of its copy of that spoke's manifest, so the copy is its
//! own cursor and is rebuilt with it. [`crate::archive`] writes and reads the captures.

use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::archive::{self, Transfer};
use crate::config::{Config, MachineConfig};
use crate::home::Home;
use crate::tick::Deadline;
use crate::util::{append_jsonl, now_rfc3339};

/// One line of a spoke's export log: what a pull took, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportEntry {
    pub at: String,
    #[serde(flatten)]
    pub sent: Transfer,
    /// Why the export stopped short, if it did: the hub hung up, or the cursor wasn't this
    /// archive's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Export this spoke's captures from byte `since` of its manifest to `out`, and log it. Only a
/// spoke exports: its `[spoke]` names it to the hub.
pub fn export(home: &Home, since: u64, out: &mut dyn Write) -> Result<Transfer> {
    let Some(spoke) = home.spoke()? else {
        bail!(
            "only a spoke exports, and this home has no [spoke]: set one up with `distill init \
             --spoke <machine>`"
        );
    };
    // A sweep here waits, so the manifest and the blobs it names stay as they were read.
    let _lock = home.lock_archive()?;
    // What went before an export stopped short (the hub hung up) is logged too.
    let mut sent = Transfer::default();
    let result = archive::export(home, &spoke.machine, since, out, &mut sent);
    let entry = ExportEntry {
        at: now_rfc3339(),
        sent: sent.clone(),
        error: result.as_ref().err().map(|e| format!("{e:#}")),
    };
    append_jsonl(&home.export_log_path(), &entry)?;
    result.map(|()| sent)
}

/// The `since` an SSH forced command was asked for. `original` is `SSH_ORIGINAL_COMMAND`, which
/// must end `export --since <bytes>`, as [`pull`] sends it; whatever else it asks for is not run.
pub fn forced_since(original: Option<&str>) -> Result<u64> {
    let words: Vec<&str> = original.unwrap_or_default().split_whitespace().collect();
    match words.as_slice() {
        [_, .., "export", "--since", n] => n
            .parse()
            .with_context(|| format!("{n:?} is not a byte count")),
        _ => bail!("this key may only run `distill export --since <bytes>`"),
    }
}

/// What a pull from one machine took.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pulled {
    pub machine: String,
    /// When this machine's pull ended: a tick logs it only once all its steps are done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(flatten)]
    pub took: Transfer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PullReport {
    pub machines: Vec<Pulled>,
    /// Why nothing was pulled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl PullReport {
    pub fn ok(&self) -> bool {
        self.machines.iter().all(|p| p.error.is_none())
    }

    pub fn summary(&self) -> String {
        if let Some(why) = &self.skipped {
            return format!("pull skipped: {why}");
        }
        let parts: Vec<String> = self
            .machines
            .iter()
            .map(|p| match &p.error {
                Some(e) => format!("{}: failed ({e})", p.machine),
                None => format!(
                    "{}: {} new, {}{}",
                    p.machine,
                    crate::util::count(p.took.lines, "capture"),
                    crate::util::count(p.took.blobs, "blob"),
                    if p.took.stopped {
                        ", stopped at the tick's end"
                    } else {
                        ""
                    }
                ),
            })
            .collect();
        format!("pulled {}", parts.join("; "))
    }
}

/// Pull each machine in `[[machines]]`, or only the one named `only`, until `deadline`. A machine
/// that can't be reached, or whose archive isn't the one the hub has a copy of, fails alone.
pub fn pull(
    home: &Home,
    config: &Config,
    only: Option<&str>,
    deadline: Deadline,
) -> Result<PullReport> {
    if home.spoke()?.is_some() {
        bail!("a spoke pulls from no machine: it only captures");
    }
    if config.machines.is_empty() {
        return Ok(PullReport {
            skipped: Some("no [[machines]] are set up".into()),
            ..Default::default()
        });
    }
    if let Some(name) = only
        && !config.machines.iter().any(|m| m.name == name)
    {
        bail!("no machine is named {name:?} in [[machines]]");
    }
    // The archive lock, as the sweep and a backup take it: two pulls at once would append to one
    // copy together, and a backup would read a copy a pull is changing. The tick doesn't wait.
    let _lock = if deadline == Deadline::NONE {
        home.lock_archive()?
    } else {
        match home.try_lock("archive")? {
            Some(lock) => lock,
            None => {
                return Ok(PullReport {
                    skipped: Some("a sweep, a backup or another pull holds the archive".into()),
                    ..Default::default()
                });
            }
        }
    };
    crate::archive::clear_staging(home)?;
    let mut report = PullReport::default();
    for m in config
        .machines
        .iter()
        .filter(|m| only.is_none_or(|n| n == m.name))
    {
        if deadline.passed() {
            break;
        }
        let mut pulled = pull_one(home, m, deadline, "ssh", QUIET, OVERRUN);
        // To the millisecond, so a pull by hand that ends just after a tick's sorts after it.
        pulled.at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        report.machines.push(pulled);
    }
    Ok(report)
}

/// How long a pull waits for the next bytes before it gives up. A spoke sends without a pause
/// once ssh is in, so a capture still coming is let finish past the deadline, and an export
/// that went quiet is not waited on.
const QUIET: Duration = Duration::from_secs(60);

/// Counts the bytes read through it, for the watch on a pull, and stops waiting for more once
/// the watch gave up: a process ssh left behind can hold the pipe open after ssh is gone.
struct Counted<R> {
    inner: R,
    seen: Arc<AtomicU64>,
    gave_up: Arc<AtomicBool>,
}

impl<R: Read + std::os::fd::AsRawFd> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut ready = libc::pollfd {
            fd: self.inner.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            if self.gave_up.load(Ordering::Relaxed) {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            // SAFETY: one valid pollfd, for a descriptor `inner` keeps open.
            match unsafe { libc::poll(&mut ready, 1, 100) } {
                0 => {}
                -1 => {
                    let e = std::io::Error::last_os_error();
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
                _ => break,
            }
        }
        let n = self.inner.read(buf)?;
        self.seen.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Add the spoke `name`, reached at `host`, to `config`'s `[[machines]]`, as `config.toml` would
/// accept it: a machine name not taken, and a host.
pub fn add(config: &mut Config, name: &str, host: &str) -> Result<()> {
    let m = MachineConfig {
        name: name.trim().to_string(),
        host: host.trim().to_string(),
        command: None,
    };
    m.check()?;
    crate::home::not_hub(&m.name)?;
    if config.machines.iter().any(|x| x.name == m.name) {
        bail!("a machine is already named {:?}", m.name);
    }
    config.machines.push(m);
    Ok(())
}

/// Check that `m` answers over SSH as the spoke this hub has a copy of, as a pull would start:
/// it runs the export there and reads only its header, so nothing is taken, and says whether
/// it has captures to pull. The spoke logs the export it hung up on.
pub fn check(home: &Home, m: &MachineConfig) -> Result<String> {
    if home.spoke()?.is_some() {
        bail!("a spoke pulls from no machine: it only captures");
    }
    let (pulled, waiting) = over_ssh(home, m, Want::Check, "ssh", CHECK_QUIET, Some(CHECK_LIMIT));
    match (pulled.error, waiting) {
        (Some(e), _) => bail!(e),
        (None, Some(true)) => Ok(format!(
            "{} answers as {}, with captures to pull",
            m.host, m.name
        )),
        (None, _) => Ok(format!(
            "{} answers as {}, with nothing new to pull",
            m.host, m.name
        )),
    }
}

/// How long a check waits for the spoke's first bytes.
const CHECK_QUIET: Duration = Duration::from_secs(20);

/// How long a check may take in all: a spoke that keeps sending without finishing its header
/// is never quiet.
const CHECK_LIMIT: Duration = Duration::from_secs(30);

/// How long a tick's pull may run past the tick's end while a capture is still coming, before
/// it is stopped mid-capture; the next pull goes on from the last whole one.
const OVERRUN: Duration = Duration::from_secs(60);

/// What a run of the export over SSH is for.
enum Want {
    /// Take its captures, stopping between two at the deadline.
    Take(Deadline),
    /// Read its header only.
    Check,
}

fn pull_one(
    home: &Home,
    m: &MachineConfig,
    deadline: Deadline,
    ssh: &str,
    quiet: Duration,
    overrun: Duration,
) -> Pulled {
    let limit = deadline
        .finish_by()
        .map(|t| t.saturating_duration_since(Instant::now()) + overrun);
    over_ssh(home, m, Want::Take(deadline), ssh, quiet, limit).0
}

/// Run `m`'s export over SSH for `want`: what a pull took, or why it failed, and for a check
/// whether the spoke has captures to pull.
fn over_ssh(
    home: &Home,
    m: &MachineConfig,
    want: Want,
    ssh: &str,
    quiet: Duration,
    limit: Option<Duration>,
) -> (Pulled, Option<bool>) {
    let until = limit.map(|l| Instant::now() + l);
    let checking = matches!(want, Want::Check);
    let mut pulled = Pulled {
        machine: m.name.clone(),
        at: None,
        took: Transfer::default(),
        error: None,
    };
    // A check compares the header with the copy as it was when the export was asked for: a pull
    // can grow it meanwhile, and the check holds no lock.
    let copy = match archive::copy_of(home, &m.name) {
        Ok(c) => c,
        Err(e) => {
            pulled.error = Some(format!("{e:#}"));
            return (pulled, None);
        }
    };
    let since = copy.since;
    pulled.took.since = since;
    pulled.took.to = since;
    let command = m.command.as_deref().unwrap_or("distill");
    let spawned = Command::new(ssh)
        // No prompt, so the timer can run it; and a connection that goes quiet is dropped
        // rather than holding the tick.
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"])
        .args([
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=4",
            "--",
        ])
        .arg(&m.host)
        .arg(format!("{command} export --since {since}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            pulled.error = Some(format!("could not run ssh: {e}"));
            return (pulled, None);
        }
    };
    // What ssh says, gathered as it comes: a proxy it started can hold the pipe open after ssh
    // exits, so the pull waits for its end only briefly.
    let said = Arc::new(Mutex::new(Vec::new()));
    let drain = {
        let (mut stderr, said) = (child.stderr.take().expect("piped"), said.clone());
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n @ 1..) = stderr.read(&mut buf) {
                said.lock().map(|mut s| s.extend_from_slice(&buf[..n])).ok();
            }
        })
    };
    let seen = Arc::new(AtomicU64::new(0));
    let (done, gave_up, overran) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let mut stdout = BufReader::new(Counted {
        inner: child.stdout.take().expect("piped"),
        seen: seen.clone(),
        gave_up: gave_up.clone(),
    });
    // An export that sends nothing for `quiet`, or still sends at `until`, is stopped, so a
    // spoke that hangs or trickles holds neither the tick nor the archive lock.
    let child = Arc::new(Mutex::new(child));
    let watch = {
        let (child, done, gave_up, overran) = (
            child.clone(),
            done.clone(),
            gave_up.clone(),
            overran.clone(),
        );
        std::thread::spawn(move || {
            let (mut bytes, mut since_bytes) = (0, Instant::now());
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                let now = seen.load(Ordering::Relaxed);
                if until.is_some_and(|t| Instant::now() >= t) {
                    overran.store(true, Ordering::Relaxed);
                    gave_up.store(true, Ordering::Relaxed);
                    let _ = child.lock().map(|mut c| c.kill());
                    return;
                }
                if now != bytes {
                    (bytes, since_bytes) = (now, Instant::now());
                } else if since_bytes.elapsed() >= quiet {
                    gave_up.store(true, Ordering::Relaxed);
                    let _ = child.lock().map(|mut c| c.kill());
                    return;
                }
            }
        })
    };
    let mut waiting = None;
    let took = match want {
        Want::Take(deadline) => archive::take_into(
            home,
            &m.name,
            &mut stdout,
            &|| deadline.passed(),
            &mut pulled.took,
        ),
        Want::Check => archive::check_export(&m.name, &copy, &mut stdout).map(|w| {
            waiting = Some(w);
        }),
    };
    // A check is done once the header is read: the rest isn't wanted.
    let stopped = pulled.took.stopped || waiting.is_some();
    if took.is_err() || stopped {
        let _ = child.lock().map(|mut c| c.kill());
    }
    drop(stdout);
    let status = loop {
        match child.lock().map(|mut c| c.try_wait()) {
            Ok(Ok(Some(status))) => break Ok(status),
            Ok(Ok(None)) => std::thread::sleep(Duration::from_millis(20)),
            Ok(Err(e)) => break Err(e),
            Err(_) => break Err(std::io::Error::other("the watch on ssh failed")),
        }
    };
    done.store(true, Ordering::Relaxed);
    let _ = watch.join();
    let waited = Instant::now();
    while !drain.is_finished() && waited.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let said =
        String::from_utf8_lossy(&said.lock().map(|s| s.clone()).unwrap_or_default()).into_owned();
    if took.is_err() && overran.load(Ordering::Relaxed) {
        pulled.error = Some(if checking {
            format!("{} was still sending at the check's time limit", m.host)
        } else {
            format!(
                "{} was still sending at the pull's time limit, so it stopped: the next pull \
                 goes on from the last whole capture",
                m.host
            )
        });
        return (pulled, None);
    }
    if took.is_err() && gave_up.load(Ordering::Relaxed) {
        pulled.error = Some(format!(
            "{} sent nothing for {}s, so the pull stopped",
            m.host,
            quiet.as_secs()
        ));
        return (pulled, None);
    }
    let last = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::trim);
    match took {
        Ok(()) => {
            let failed = status.as_ref().map_or(true, |s| !s.success());
            if failed && !stopped {
                pulled.error = Some(match last {
                    Some(l) => format!("ssh {}: {l}", m.host),
                    None => format!("ssh {} failed", m.host),
                });
            }
        }
        // What the other side said says more than a stream that ended early. When the pull
        // refused what came, ssh only says how it was stopped.
        Err(e) => {
            let ended = format!("{e:#}").starts_with(archive::ENDED_EARLY);
            pulled.error = Some(match last {
                Some(l) if ended && status.as_ref().is_ok_and(|s| !s.success()) => {
                    format!("ssh {}: {l}", m.host)
                }
                _ => format!("{e:#}"),
            });
        }
    }
    (pulled, waiting)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Archive;
    use crate::config::Spoke;
    use crate::harness::Harness;
    use std::fs;

    fn hub_of(dir: &std::path::Path) -> (Home, Config) {
        let home = Home::new(dir.join("hub"));
        home.ensure().unwrap();
        let config = Config {
            machines: vec![MachineConfig {
                name: "nas".into(),
                host: "me@nas".into(),
                command: None,
            }],
            ..Config::default()
        };
        (home, config)
    }

    #[test]
    fn the_tick_doesnt_wait_on_a_sweep_or_backup_holding_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let (home, config) = hub_of(dir.path());
        let _held = home.lock_archive().unwrap();
        let r = pull(
            &home,
            &config,
            None,
            Deadline::after(Duration::from_secs(60)),
        )
        .unwrap();
        assert!(r.machines.is_empty(), "{r:?}");
        assert!(r.skipped.unwrap().contains("holds the archive"));
    }

    /// A stand-in for ssh that runs `body` as a shell script.
    #[cfg(unix)]
    fn fake_ssh(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let ssh = dir.join("ssh");
        fs::write(&ssh, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        ssh
    }

    #[cfg(unix)]
    #[test]
    fn a_check_reads_the_spokes_header_and_takes_nothing() {
        use sha2::Digest;
        let dir = tempfile::tempdir().unwrap();
        let (home, config) = hub_of(dir.path());
        let m = &config.machines[0];
        let empty = hex::encode(sha2::Sha256::digest(b""));
        let header = |name: &str, end: u64| {
            format!(
                "printf 'distill-export 1\\n{{\"machine\":\"{name}\",\"since\":0,\"prefix\":\"{empty}\",\"end\":{end}}}\\n'\nexec sleep 30"
            )
        };
        let check = |body: &str| {
            let ssh = fake_ssh(dir.path(), body);
            let start = Instant::now();
            let r = over_ssh(&home, m, Want::Check, ssh.to_str().unwrap(), QUIET, None);
            assert!(start.elapsed() < Duration::from_secs(10), "{r:?}");
            r
        };
        // Captures waiting: the rest of the export isn't read, and nothing is written.
        let (p, waiting) = check(&header("nas", 120));
        assert_eq!((p.error, waiting), (None, Some(true)));
        assert!(!home.raw_dir_of(Some("nas")).exists());
        let (p, waiting) = check(&header("nas", 0));
        assert_eq!((p.error, waiting), (None, Some(false)));
        // Another machine answering, and one that can't be reached.
        let (p, _) = check(&header("box", 0));
        assert!(p.error.unwrap().contains(r#"is "box", not "nas""#));
        let (p, waiting) =
            check("echo 'ssh: connect to host nas port 22: Connection refused' >&2\nexit 255");
        assert_eq!(waiting, None);
        assert_eq!(
            p.error.unwrap(),
            "ssh me@nas: ssh: connect to host nas port 22: Connection refused"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_spoke_that_keeps_sending_without_finishing_is_stopped_at_the_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (home, config) = hub_of(dir.path());
        let m = &config.machines[0];
        // A byte at a time, never a whole frame, so it is never quiet.
        let ssh = fake_ssh(dir.path(), "while :; do printf d; sleep 0.05; done");
        let ssh = ssh.to_str().unwrap();
        let start = Instant::now();
        let p = pull_one(
            &home,
            m,
            Deadline::after(Duration::from_millis(200)),
            ssh,
            QUIET,
            Duration::from_millis(300),
        );
        assert!(start.elapsed() < Duration::from_secs(10), "{p:?}");
        let e = p.error.unwrap();
        assert!(e.contains("still sending at the pull's time limit"), "{e}");
        let start = Instant::now();
        let (p, waiting) = over_ssh(
            &home,
            m,
            Want::Check,
            ssh,
            QUIET,
            Some(Duration::from_millis(500)),
        );
        assert!(start.elapsed() < Duration::from_secs(10), "{p:?}");
        assert_eq!(waiting, None);
        let e = p.error.unwrap();
        assert!(e.contains("still sending at the check's time limit"), "{e}");
    }

    #[test]
    fn a_spoke_is_added_as_config_toml_would_take_it() {
        let mut config = Config::default();
        add(&mut config, " nas ", " me@nas ").unwrap();
        assert_eq!(
            (
                config.machines[0].name.as_str(),
                config.machines[0].host.as_str()
            ),
            ("nas", "me@nas")
        );
        for (name, host, says) in [
            ("nas", "me@other", "already named"),
            ("Nas", "me@nas", "not a machine name"),
            ("box", "-oProxyCommand=x", "not a host"),
            ("box", " ", "not a host"),
            ("hub", "me@hub", "can't name a spoke"),
        ] {
            let e = add(&mut config, name, host).unwrap_err();
            assert!(format!("{e:#}").contains(says), "{name} {host}: {e:#}");
        }
        assert_eq!(config.machines.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn an_export_that_goes_quiet_is_stopped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (home, config) = hub_of(dir.path());
        // A stand-in for ssh: a header for an empty copy, then nothing, and a process left
        // behind that holds its output open.
        let ssh = dir.path().join("ssh");
        use sha2::Digest;
        let empty = hex::encode(sha2::Sha256::digest(b""));
        fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nprintf 'distill-export 1\\n{{\"machine\":\"nas\",\"since\":0,\"prefix\":\"{empty}\",\"end\":0}}\\n'\nsleep 20 &\nexec sleep 30\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        let start = Instant::now();
        let p = pull_one(
            &home,
            &config.machines[0],
            Deadline::NONE,
            ssh.to_str().unwrap(),
            Duration::from_millis(500),
            OVERRUN,
        );
        assert!(start.elapsed() < Duration::from_secs(10), "{p:?}");
        assert!(p.error.unwrap().contains("sent nothing"));
    }

    /// Takes `room` bytes, then fails as a pipe whose reader went away does.
    struct HungUp {
        room: usize,
    }

    impl Write for HungUp {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.room == 0 {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            let n = buf.len().min(self.room);
            self.room -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_export_the_hub_hung_up_on_logs_what_went_before() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        Config {
            spoke: Some(Spoke {
                machine: "nas".into(),
            }),
            ..Config::default()
        }
        .save(&home.config_path())
        .unwrap();
        let mut a = Archive::open(&home).unwrap();
        a.put(Harness::Pi, "s", "1", &b"one\n"[..]).unwrap();
        a.put(Harness::Pi, "t", "1", &b"two\n"[..]).unwrap();
        let mut whole = Vec::new();
        export(&home, 0, &mut whole).unwrap();
        // Room for everything but the last line and the end.
        let at = whole.windows(5).rposition(|w| w == b"line ").unwrap();
        assert!(export(&home, 0, &mut HungUp { room: at }).is_err());
        let log = fs::read_to_string(home.export_log_path()).unwrap();
        let last: ExportEntry = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!((last.sent.lines, last.sent.blobs), (1, 2), "{last:?}");
        assert!(last.error.is_some(), "{last:?}");

        // Cut inside a large blob: what of it went is counted.
        let big = "x".repeat(99_999) + "\n";
        a.put(Harness::Pi, "u", "1", big.as_bytes()).unwrap();
        let mut whole = Vec::new();
        export(&home, 0, &mut whole).unwrap();
        let at = whole.windows(5).rposition(|w| w == b"blob ").unwrap();
        let payload = at + whole[at..].iter().position(|b| *b == b'\n').unwrap() + 1;
        assert!(export(&home, 0, &mut HungUp { room: at + 80_000 }).is_err());
        let log = fs::read_to_string(home.export_log_path()).unwrap();
        let last: ExportEntry = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(last.sent.blobs, 2, "{last:?}");
        assert_eq!(
            last.sent.bytes,
            (8 + at + 80_000 - payload) as u64,
            "{last:?}"
        );
    }

    #[test]
    fn a_forced_command_runs_only_an_export_since_a_byte_count() {
        for (asked, since) in [
            ("distill export --since 0", 0),
            ("/opt/bin/distill export --since 412", 412),
        ] {
            assert_eq!(forced_since(Some(asked)).unwrap(), since, "{asked}");
        }
        for asked in [
            None,
            Some(""),
            Some("distill export"),
            Some("distill read claude-code/s1"),
            Some("distill export --since 5; rm -rf ~"),
            Some("distill export --since -1"),
        ] {
            assert!(forced_since(asked).is_err(), "{asked:?}");
        }
    }
}
