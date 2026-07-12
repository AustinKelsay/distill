//! Bounded provider-subprocess policy used by OpenCode (#28).
//!
//! This module does not implement provider adapters. It owns hard duration and
//! stdout/stderr byte caps with stable redacted errors. Stdin is written on a
//! helper thread after stdout/stderr readers start so large payloads cannot
//! deadlock the caller. Child cleanup joins readers on every exit path.
//!
//! On Unix the runner places the child in its own process group and signals that
//! group on kill when possible. Descendants that deliberately detach from that
//! group remain outside this direct-child cleanup boundary.

use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::{LibraryError, LibraryResult};

/// Hard limits applied to provider helper processes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderProcessLimits {
    /// Maximum wall-clock duration before the child is killed.
    pub max_duration: Duration,
    /// Maximum accepted stdout bytes.
    pub max_stdout_bytes: usize,
    /// Maximum accepted stderr bytes.
    pub max_stderr_bytes: usize,
}

impl Default for ProviderProcessLimits {
    fn default() -> Self {
        Self {
            max_duration: Duration::from_secs(30),
            max_stdout_bytes: 8 * 1024 * 1024,
            max_stderr_bytes: 1024 * 1024,
        }
    }
}

/// Captured bounded subprocess output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedProcessOutput {
    /// Process exit code when the process exited normally.
    pub exit_code: Option<i32>,
    /// Captured stdout bytes under the configured cap.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes under the configured cap.
    pub stderr: Vec<u8>,
}

/**
 * Run a child command under hard duration and stdout/stderr byte caps.
 *
 * On timeout or output overflow the child is killed and a typed
 * [`LibraryError::ProviderProcessBoundExceeded`] is returned. Diagnostics never
 * include the raw command argv or provider payloads.
 */
pub fn run_bounded_command(
    mut command: Command,
    limits: ProviderProcessLimits,
    stdin_bytes: Option<&[u8]>,
) -> LibraryResult<BoundedProcessOutput> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Prefer process-group kill so descendants that remain in the child's
        // group are reaped together. A failed setup aborts spawn; this prevents
        // an unverified negative-PID kill from targeting an unrelated group.
        unsafe {
            command.pre_exec(setpgid_self);
        }
    }

    #[cfg(unix)]
    let uses_process_group = true;
    #[cfg(not(unix))]
    let uses_process_group = false;

    command
        .stdin(if stdin_bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(LibraryError::Io)?;
    let kill_flag = Arc::new(AtomicBool::new(false));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    let out_flag = Arc::clone(&kill_flag);
    let err_flag = Arc::clone(&kill_flag);
    let max_out = limits.max_stdout_bytes;
    let max_err = limits.max_stderr_bytes;

    // Start readers before any stdin write so a large stdin cannot fill the pipe
    // and deadlock while nobody is draining stdout/stderr.
    let out_handle = thread::spawn(move || read_capped(stdout, max_out, &out_flag));
    let err_handle = thread::spawn(move || read_capped(stderr, max_err, &err_flag));
    let stdin_handle = spawn_stdin_writer(stdin, stdin_bytes.map(|bytes| bytes.to_vec()));

    let started = Instant::now();
    let status = loop {
        if kill_flag.load(Ordering::SeqCst) {
            kill_child(&mut child, uses_process_group);
            let cleanup = cleanup_handles(stdin_handle, out_handle, err_handle, true);
            let _ = child.wait();
            return match cleanup {
                Err(err) => Err(err),
                Ok(_) => Err(LibraryError::ProviderProcessBoundExceeded {
                    detail: "provider output exceeded configured byte cap".into(),
                }),
            };
        }
        if started.elapsed() > limits.max_duration {
            kill_child(&mut child, uses_process_group);
            let cleanup = cleanup_handles(stdin_handle, out_handle, err_handle, true);
            let _ = child.wait();
            return match cleanup {
                Err(err) => Err(err),
                Ok(_) => Err(LibraryError::ProviderProcessBoundExceeded {
                    detail: "provider process exceeded configured duration".into(),
                }),
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(1)),
            Err(err) => {
                kill_child(&mut child, uses_process_group);
                let _ = cleanup_handles(stdin_handle, out_handle, err_handle, true);
                let _ = child.wait();
                return Err(LibraryError::Io(err));
            }
        }
    };

    let (stdout, stderr) = cleanup_handles(stdin_handle, out_handle, err_handle, false)?;

    Ok(BoundedProcessOutput {
        exit_code: status.code(),
        stdout,
        stderr,
    })
}

/**
 * Enforce stdout/stderr byte caps without spawning a child.
 *
 * Deterministic helper used by contracts on all platforms.
 */
#[cfg(feature = "test-leases")]
pub fn enforce_output_bounds_for_test(
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    limits: ProviderProcessLimits,
) -> LibraryResult<BoundedProcessOutput> {
    if stdout_bytes.len() > limits.max_stdout_bytes {
        return Err(LibraryError::ProviderProcessBoundExceeded {
            detail: "provider stdout exceeded configured byte cap".into(),
        });
    }
    if stderr_bytes.len() > limits.max_stderr_bytes {
        return Err(LibraryError::ProviderProcessBoundExceeded {
            detail: "provider stderr exceeded configured byte cap".into(),
        });
    }
    Ok(BoundedProcessOutput {
        exit_code: Some(0),
        stdout: stdout_bytes.to_vec(),
        stderr: stderr_bytes.to_vec(),
    })
}

fn spawn_stdin_writer(
    stdin: Option<std::process::ChildStdin>,
    bytes: Option<Vec<u8>>,
) -> Option<JoinHandle<LibraryResult<()>>> {
    let (Some(mut stdin), Some(bytes)) = (stdin, bytes) else {
        return None;
    };
    Some(thread::spawn(move || {
        stdin.write_all(&bytes).map_err(LibraryError::Io)?;
        Ok(())
    }))
}

fn cleanup_handles(
    stdin_handle: Option<JoinHandle<LibraryResult<()>>>,
    out_handle: JoinHandle<LibraryResult<Vec<u8>>>,
    err_handle: JoinHandle<LibraryResult<Vec<u8>>>,
    ignore_stdin_errors: bool,
) -> LibraryResult<(Vec<u8>, Vec<u8>)> {
    let stdin_error = if let Some(handle) = stdin_handle {
        match handle.join() {
            Ok(Ok(())) => None,
            Ok(Err(err)) => Some(err),
            Err(_) => Some(LibraryError::InvalidArgument(
                "stdin writer panicked".into(),
            )),
        }
    } else {
        None
    };

    // Always join both reader threads before returning. A failure on one pipe
    // must not leave the other reader detached with an inherited descriptor.
    let stdout_joined = out_handle.join();
    let stderr_joined = err_handle.join();

    if !ignore_stdin_errors {
        if let Some(err) = stdin_error {
            return Err(err);
        }
    }
    let stdout = match stdout_joined {
        Ok(Ok(stdout)) => stdout,
        Ok(Err(err)) => return Err(err),
        Err(_) => {
            return Err(LibraryError::InvalidArgument(
                "stdout reader panicked".into(),
            ));
        }
    };
    let stderr = match stderr_joined {
        Ok(Ok(stderr)) => stderr,
        Ok(Err(err)) => return Err(err),
        Err(_) => {
            return Err(LibraryError::InvalidArgument(
                "stderr reader panicked".into(),
            ));
        }
    };
    Ok((stdout, stderr))
}

fn kill_child(child: &mut Child, uses_process_group: bool) {
    #[cfg(unix)]
    if uses_process_group {
        let pid = child.id() as i32;
        let _ = kill_process_group(pid);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn read_capped<R: Read>(
    reader: Option<R>,
    max_bytes: usize,
    kill_flag: &AtomicBool,
) -> LibraryResult<Vec<u8>> {
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > max_bytes {
                    kill_flag.store(true, Ordering::SeqCst);
                    return Err(LibraryError::ProviderProcessBoundExceeded {
                        detail: "provider output exceeded configured byte cap".into(),
                    });
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(err) => return Err(LibraryError::Io(err)),
        }
    }
    Ok(buf)
}

#[cfg(unix)]
fn setpgid_self() -> io::Result<()> {
    extern "C" {
        fn setpgid(pid: i32, pgid: i32) -> i32;
    }
    if unsafe { setpgid(0, 0) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn kill_process_group(pid: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGKILL: i32 = 9;
    unsafe { kill(-pid, SIGKILL) }
}
