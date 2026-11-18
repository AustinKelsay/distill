//! Peak memory of building the word index from scratch over synthetic sessions of mixed sizes,
//! a few of them huge with many distinct words (#105). Ignored (it writes hundreds of MB):
//! `DISTILL_BENCH_SESSIONS=1500 DISTILL_BENCH_GIANTS=3 DISTILL_BENCH_GIANT_MB=60 cargo test -p
//! distill-core --release --test index_memory -- --ignored --nocapture`. Synthetic data only, in a
//! folder under `target/` that is deleted afterwards.

use std::io::Write;

use distill_core::{Harness, Home, archive::Archive, index, normalize};

fn peak_rss_mb() -> u64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let raw = ru.ru_maxrss as u64;
    // Bytes on macOS, kilobytes on Linux.
    if cfg!(target_os = "macos") {
        raw / (1024 * 1024)
    } else {
        raw / 1024
    }
}

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A Claude Code session of about `kb` KB: tool results made of distinct hex ids and paths, so
/// its vocabulary grows with its size.
fn session(seed: u64, kb: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut rand = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut out = Vec::with_capacity(kb * 1024 + 4096);
    let t = "2026-11-03T08:30:53Z";
    writeln!(out, r#"{{"type":"user","cwd":"/r","timestamp":"{t}","message":{{"role":"user","content":"please look at build {seed}"}}}}"#).unwrap();
    let mut i = 0;
    while out.len() < kb * 1024 {
        let mut body = String::new();
        while body.len() < 16_000 {
            let v = rand();
            body.push_str(&format!(
                "{v:016x} /src/mod_{:x}/file_{:x}.rs ",
                v >> 40,
                v & 0xffff
            ));
        }
        writeln!(out, r#"{{"type":"assistant","timestamp":"{t}","message":{{"role":"assistant","model":"m","content":[{{"type":"tool_use","id":"t{i}","name":"Bash","input":{{"command":"ls {i}"}}}}]}}}}"#).unwrap();
        writeln!(out, r#"{{"type":"user","timestamp":"{t}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t{i}","content":"{body}"}}]}}}}"#).unwrap();
        i += 1;
    }
    out
}

#[test]
#[ignore = "benchmark: writes hundreds of MB"]
fn building_the_index_over_big_sessions_stays_bounded() {
    let sessions = env("DISTILL_BENCH_SESSIONS", 1500);
    let giants = env("DISTILL_BENCH_GIANTS", 3);
    let giant_mb = env("DISTILL_BENCH_GIANT_MB", 60);
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("index-memory");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let home = Home::new(base.join("home"));
    home.ensure().unwrap();
    {
        let _lock = home.lock_archive().unwrap();
        let mut archive = Archive::open(&home).unwrap();
        for n in 0..sessions + giants {
            let kb = if n >= sessions {
                giant_mb * 1024
            } else {
                // 40 KB to about 2 MB, skewed small.
                40 + (n * 7919 % 50).pow(2) * 3 / 2
            };
            archive
                .put(
                    Harness::ClaudeCode,
                    &format!("/s/proj/b{n}.jsonl"),
                    "s",
                    session(n as u64 + 1, kb).as_slice(),
                )
                .unwrap();
        }
    }
    normalize::normalize(&home, true).unwrap();
    let before = peak_rss_mb();
    let t = std::time::Instant::now();
    let r = index::build(&home).unwrap();
    println!(
        "sessions={} giants={giants}x{giant_mb}MB: index peak RSS {} MB (before build {before} MB), {:?}, {r:?}",
        sessions + giants,
        peak_rss_mb(),
        t.elapsed()
    );
    assert_eq!(r.indexed as usize, sessions + giants);
    let _ = std::fs::remove_dir_all(&base);
}
