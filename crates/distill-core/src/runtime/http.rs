//! One HTTP/1.0 request to a server on this machine, bounded as a whole by its timeout: the
//! connection, the request and the answer all share one deadline, so neither a server that reads
//! slowly nor an answer that trickles in can outlast it. The tick's time budget relies on that.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::RuntimeError;

/// The status code and the JSON body (`Value::Null` when it is not JSON) of one request. A
/// connection refused is [`RuntimeError::NotRunning`].
pub(super) fn request(
    addr: &SocketAddr,
    method: &str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<(String, Value), RuntimeError> {
    let until = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(addr, timeout.min(Duration::from_secs(2)))
        .map_err(|_| RuntimeError::NotRunning)?;
    let failed = |e: std::io::Error| RuntimeError::Failed(e.to_string());
    let late = || {
        RuntimeError::Failed(format!(
            "{path}: no whole answer within {}s",
            timeout.as_secs()
        ))
    };
    // A socket timeout: the deadline came with no more progress.
    let timed_out = |e: &std::io::Error| {
        matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        )
    };
    let body = body.map(Value::to_string).unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        addr,
        body.len()
    );
    // Write and read against the whole request's deadline, so neither a server that reads
    // slowly nor an answer that trickles in can outlast it.
    let mut sent = request.as_bytes();
    while !sent.is_empty() {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(late());
        }
        stream.set_write_timeout(Some(left)).map_err(failed)?;
        match stream.write(sent) {
            Ok(0) => return Err(RuntimeError::Failed(format!("{path}: the server hung up"))),
            Ok(n) => sent = &sent[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if timed_out(&e) => return Err(late()),
            Err(e) => return Err(failed(e)),
        }
    }
    let mut answer = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(late());
        }
        stream.set_read_timeout(Some(left)).map_err(failed)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => answer.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if timed_out(&e) => return Err(late()),
            Err(e) => return Err(failed(e)),
        }
    }
    let text = String::from_utf8_lossy(&answer);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| RuntimeError::Failed("an answer with no body".into()))?;
    let status = head.split_whitespace().nth(1).unwrap_or("").to_string();
    Ok((status, serde_json::from_str(body).unwrap_or(Value::Null)))
}
