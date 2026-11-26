//! A backup target reached through its adapter (ADR 0009, issue #177): the executable
//! `distill-backup-<kind>` on `PATH`, spoken to in lines over its standard input and output, as
//! git remote helpers are. `docs/backup-adapters.md` is the protocol. Core holds no network code
//! for backups: the adapter moves opaque names and ciphertext and never sees a key.

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};

use super::store::{Store, object_name};
use crate::config::BackupTarget;
use crate::util::private_file;

/// What one use of a target may do, as `hello` tells the adapter.
#[derive(Debug, Clone, Copy)]
pub(super) enum Open {
    /// Check that the target is there and takes a write, making nothing that stays.
    Probe,
    /// The target's first use: the adapter may set up its place.
    First,
    /// A target used before: a place that is missing is an error.
    Again,
}

impl Open {
    fn word(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::First => "first",
            Self::Again => "again",
        }
    }
}

/// A running adapter for one target, until it is dropped.
pub(super) struct Adapter {
    program: String,
    io: Mutex<Io>,
    /// Where the files of `put` and `get` go: a folder only the owner can use, removed on drop.
    scratch: tempfile::TempDir,
}

struct Io {
    child: Child,
    stdin: Option<ChildStdin>,
    /// `None` once closed, so an adapter still writing stops rather than blocks.
    stdout: Option<BufReader<ChildStdout>>,
}

impl Adapter {
    /// Start `target`'s adapter and say hello: the protocol version, what this use may do, and
    /// the target's table as one line of JSON.
    pub(super) fn open(target: &BackupTarget, open: Open) -> Result<Self> {
        let kind = &target.kind;
        let fits = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
        if kind.is_empty() || kind.starts_with('-') || !kind.bytes().all(fits) {
            bail!(
                "backup target {:?} has kind {kind:?}, which is not a backup kind (lowercase \
                 letters, digits and -)",
                target.name
            );
        }
        let program = format!("distill-backup-{kind}");
        let mut child = Command::new(&program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => anyhow!(
                    "no `{program}` on PATH, for backup target {:?} of kind {kind:?}",
                    target.name
                ),
                _ => anyhow!(e).context(format!("start {program}")),
            })?;
        let io = Io {
            stdin: child.stdin.take(),
            stdout: child.stdout.take().map(BufReader::new),
            child,
        };
        let scratch = tempfile::Builder::new()
            .prefix("distill-backup-")
            // Owner-only from the start: the system's temporary folder is shared.
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir()
            .context("make a folder for backup objects in transit")?;
        if scratch.path().to_string_lossy().contains('\n') {
            bail!("the temporary folder's path holds a line break");
        }
        let adapter = Self {
            program,
            io: Mutex::new(io),
            scratch,
        };
        let table = serde_json::to_string(target)?;
        adapter.call(&format!("hello 1 {} {table}", open.word()), |_, _| Ok(()))?;
        Ok(adapter)
    }

    /// Send the command `line` and read its reply. `ok [rest]` goes to `then` with the adapter's
    /// output, to read what follows the reply; `error <message>` is the adapter's error.
    fn call<T>(
        &self,
        line: &str,
        then: impl FnOnce(&str, &mut dyn BufRead) -> Result<T>,
    ) -> Result<T> {
        let mut io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let io = &mut *io;
        let sent = io
            .stdin
            .as_mut()
            .context("the adapter's input is closed")
            .and_then(|s| Ok(writeln!(s, "{line}").and_then(|()| s.flush())?));
        let mut reply = String::new();
        let read = match sent {
            Ok(()) => match io.stdout.as_mut() {
                Some(out) => out.read_line(&mut reply).map_err(anyhow::Error::from),
                None => Err(anyhow!("the adapter's output is closed")),
            },
            Err(e) => Err(e),
        };
        if !matches!(read, Ok(n) if n > 0) {
            // Closing both pipes lets one still running finish, even one still writing.
            io.stdin = None;
            io.stdout = None;
            let status = io
                .child
                .wait()
                .map_or_else(|e| e.to_string(), |s| s.to_string());
            let garbled = read
                .as_ref()
                .err()
                .and_then(|e| e.downcast_ref::<std::io::Error>());
            if garbled.is_some_and(|e| e.kind() == std::io::ErrorKind::InvalidData) {
                bail!(
                    "{} answered with something that isn't text ({status})",
                    self.program
                );
            }
            bail!("{} stopped without answering ({status})", self.program);
        }
        let reply = reply.trim_end_matches(['\n', '\r']);
        if let Some(message) = reply.strip_prefix("error") {
            bail!("{}: {}", self.program, message.trim_start());
        }
        match reply.strip_prefix("ok") {
            Some(rest) if rest.is_empty() || rest.starts_with(' ') => {
                let out = io.stdout.as_mut().expect("read from above");
                then(rest.trim_start(), out)
            }
            _ => Err(self.unexpected(reply)),
        }
    }

    /// A path in the scratch folder for `name`'s bytes in transit, gone again afterwards.
    fn transit(&self, name: &str) -> PathBuf {
        self.scratch.path().join(name)
    }

    fn unexpected(&self, what: &str) -> anyhow::Error {
        anyhow!("{} answered {what:?}, which is not a reply", self.program)
    }
}

impl Store for Adapter {
    fn list(&self) -> Result<BTreeSet<String>> {
        self.call("list", |rest, out| {
            let n: usize = rest.parse().map_err(|_| self.unexpected(rest))?;
            let mut names = BTreeSet::new();
            for _ in 0..n {
                let mut name = String::new();
                if out.read_line(&mut name)? == 0 {
                    bail!("{} stopped partway through its list", self.program);
                }
                let name = name.trim_end_matches(['\n', '\r']);
                object_name(name)
                    .with_context(|| format!("{} listed a name it can't hold", self.program))?;
                names.insert(name.to_string());
            }
            Ok(names)
        })
    }

    fn put(&self, name: &str, from: &mut dyn Read) -> Result<bool> {
        object_name(name)?;
        let path = self.transit(name);
        let stored = (|| {
            let mut f = private_file()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("create {}", path.display()))?;
            std::io::copy(from, &mut f)?;
            drop(f);
            self.call(
                &format!("put {name} {}", path.display()),
                |rest, _| match rest {
                    "stored" => Ok(true),
                    "present" => Ok(false),
                    _ => Err(self.unexpected(rest)),
                },
            )
        })();
        let _ = fs::remove_file(&path);
        stored
    }

    fn get(&self, name: &str, to: &mut dyn Write) -> Result<()> {
        object_name(name)?;
        let path = self.transit(name);
        let got = (|| {
            self.call(&format!("get {name} {}", path.display()), |_, _| Ok(()))?;
            copy_out(&path, to)
        })();
        let _ = fs::remove_file(&path);
        got
    }

    fn sync(&self) -> Result<()> {
        self.call("sync", |_, _| Ok(()))
    }
}

fn copy_out(path: &Path, to: &mut dyn Write) -> Result<()> {
    let mut f = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    std::io::copy(&mut f, to)?;
    Ok(())
}

impl Drop for Adapter {
    /// Close the adapter's input, which tells it to finish, and its output, so one still writing
    /// (a list cut short) stops rather than blocks, and wait for it.
    fn drop(&mut self) {
        let io = self.io.get_mut().unwrap_or_else(|e| e.into_inner());
        io.stdin = None;
        io.stdout = None;
        let _ = io.child.wait();
    }
}
