//! The bound on what a parse holds (#105): a capture is read as a stream, one line at a time,
//! so a capture of any size costs about its longest line, not its length.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use distill_core::{Harness, normalize};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(l.size(), Relaxed) + l.size();
        PEAK.fetch_max(now, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let now = if new >= l.size() {
            LIVE.fetch_add(new - l.size(), Relaxed) + (new - l.size())
        } else {
            LIVE.fetch_sub(l.size() - new, Relaxed) - (l.size() - new)
        };
        PEAK.fetch_max(now, Relaxed);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// A Codex capture made up as it is read: a small response item, then a ~1 MB `compacted`
/// record, `lines` times over. It cannot be held whole, since it is never stored.
struct Synthetic {
    lines: usize,
    line: Vec<u8>,
    at: usize,
    n: usize,
}

impl Read for Synthetic {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.at == self.line.len() {
            if self.n == self.lines {
                return Ok(0);
            }
            self.line = if self.n.is_multiple_of(2) {
                format!(
                    r#"{{"timestamp":"2026-11-03T08:30:55Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"line {}"}}]}}}}"#,
                    self.n
                )
                .into_bytes()
            } else {
                let filler = "x".repeat(1_000_000);
                format!(r#"{{"type":"compacted","payload":{{"text":"{filler}"}}}}"#).into_bytes()
            };
            self.line.push(b'\n');
            self.at = 0;
            self.n += 1;
        }
        let n = buf.len().min(self.line.len() - self.at);
        buf[..n].copy_from_slice(&self.line[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[test]
fn a_giant_capture_is_parsed_without_holding_it_whole() {
    // 80 lines, 40 of them ~1 MB: about 40 MB streamed through.
    let source = Synthetic {
        lines: 80,
        line: Vec::new(),
        at: 0,
        n: 0,
    };
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let parsed = normalize::parse_stream(Harness::Codex, "rollout-x.jsonl", source).unwrap();
    let held = PEAK.load(Relaxed) - base;
    assert_eq!(parsed.events.len(), 40);
    assert!(parsed.skipped.is_empty(), "ignored records are not counted");
    // Whole, the capture is 40 MB. A line at a time it is a few copies of the longest line.
    assert!(held < 8_000_000, "the parse held {held} bytes at once");
}
