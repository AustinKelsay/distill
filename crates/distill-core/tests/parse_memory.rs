//! Peak memory of a full parse over big synthetic Codex captures (#105). The benchmark is
//! ignored (it writes hundreds of MB): `DISTILL_BENCH_CAPTURES=5 cargo test -p distill-core
//! --release --test parse_memory -- --ignored --nocapture`. Synthetic data only, in a folder
//! under `target/` that is deleted afterwards.

use std::io::Write;

use distill_core::{Harness, Home, archive::Archive, normalize};

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

/// A capture shaped like the owner's largest: many ~1 MB `compacted` lines that replay the
/// history, `event_msg` noise, and ordinary response items with sizeable tool output.
fn write_capture(path: &std::path::Path, seed: u64, mb: usize) {
    let f = std::fs::File::create(path).unwrap();
    let mut z = zstd::Encoder::new(f, 1).unwrap();
    let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut text = |n: usize| -> String {
        let mut s = String::with_capacity(n);
        while s.len() < n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.push_str(&format!("w{:x} ", x & 0xfffff));
        }
        s
    };
    writeln!(z, r#"{{"timestamp":"2026-11-03T08:30:53Z","type":"session_meta","payload":{{"id":"bench-{seed}","cwd":"/r"}}}}"#).unwrap();
    writeln!(z, r#"{{"timestamp":"2026-11-03T08:30:54Z","type":"turn_context","payload":{{"model":"gpt-6","cwd":"/r"}}}}"#).unwrap();
    let mut written = 0usize;
    let mut i = 0u64;
    while written < mb * 1024 * 1024 {
        let line = if i % 8 == 7 {
            let t = text(1_000_000);
            format!(
                r#"{{"timestamp":"2026-11-03T08:31:00Z","type":"compacted","payload":{{"replacement_history":[{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{t}"}}]}}]}}}}"#
            )
        } else if i % 8 == 3 {
            let t = text(40_000);
            format!(
                r#"{{"timestamp":"2026-11-03T08:31:01Z","type":"response_item","payload":{{"type":"function_call_output","call_id":"c{i}","output":"{t}"}}}}"#
            )
        } else if i % 8 == 1 {
            format!(
                r#"{{"timestamp":"2026-11-03T08:31:02Z","type":"event_msg","payload":{{"type":"token_count","info":"{}"}}}}"#,
                text(500)
            )
        } else {
            let t = text(2_000);
            format!(
                r#"{{"timestamp":"2026-11-03T08:31:03Z","type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{t}"}}]}}}}"#
            )
        };
        written += line.len() + 1;
        writeln!(z, "{line}").unwrap();
        i += 1;
    }
    z.finish().unwrap();
}

#[test]
#[ignore = "benchmark: writes hundreds of MB"]
fn a_full_parse_of_giant_captures_stays_bounded() {
    let captures: usize = std::env::var("DISTILL_BENCH_CAPTURES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let mb: usize = std::env::var("DISTILL_BENCH_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(250);
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("parse-memory");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let home = Home::new(base.join("home"));
    home.ensure().unwrap();
    {
        let _lock = home.lock_archive().unwrap();
        let mut archive = Archive::open(&home).unwrap();
        for n in 0..captures {
            let file = base.join(format!("c{n}.jsonl.zst"));
            write_capture(&file, n as u64 + 1, mb);
            archive
                .put(
                    Harness::Codex,
                    &format!("/u/.codex/archived_sessions/rollout-{n}.jsonl.zst"),
                    "s",
                    std::fs::File::open(&file).unwrap(),
                )
                .unwrap();
            std::fs::remove_file(&file).unwrap();
        }
    }
    let before = peak_rss_mb();
    let r = normalize::normalize(&home, true).unwrap();
    println!(
        "captures={captures} decompressed~{mb}MB each: peak RSS {} MB (before parse {before} MB); {r:?}",
        peak_rss_mb()
    );
    assert_eq!(r.normalized as usize, captures);
    let _ = std::fs::remove_dir_all(&base);
}
