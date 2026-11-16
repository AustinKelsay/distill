//! A session as the window draws it: each event as one string with its findings placed on it,
//! then cut into rows of at most [`BLOCK_LINES`] lines for the virtual list (the workaround from
//! #84: one row per event made the first frame showing a 127 KB tool result take 49 ms).

use distill_core::findings::{Field, Finding};
use distill_core::model::{Event, EventKind};
use distill_core::read::Excerpt;
use serde_json::Value;

use crate::cover::{self, Mark};

/// The most lines one row of the session holds.
pub const BLOCK_LINES: usize = 40;
/// The most bytes one row holds when its lines are long.
pub const BLOCK_BYTES: usize = 6_000;

/// One event, ready to draw.
#[derive(Debug, Clone)]
pub struct EventDoc {
    pub seq: u64,
    pub kind: EventKind,
    pub tool: Option<String>,
    pub ts: Option<String>,
    pub is_error: bool,
    pub model: Option<String>,
    /// The event's text, then its tool input, as drawn when every finding is shown.
    pub text: String,
    /// Its findings on `text`; ids are `f<n>`, `n` the finding's place in the excerpt.
    pub marks: Vec<Mark>,
}

/// A run of one event's text: what one virtual row draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    /// Index into the session's events.
    pub event: usize,
    pub start: usize,
    pub end: usize,
    /// The event's first row, which carries its heading.
    pub first: bool,
}

/// The open session, drawn.
#[derive(Debug, Clone)]
pub struct SessionDoc {
    pub events: Vec<EventDoc>,
    pub blocks: Vec<Block>,
    /// How many findings, and how many of them secrets.
    pub findings: usize,
    pub secrets: usize,
}

impl SessionDoc {
    pub fn of(x: &Excerpt) -> SessionDoc {
        let events: Vec<EventDoc> = x.events.iter().map(|e| event(e, &x.findings)).collect();
        let blocks = blocks(&events);
        SessionDoc {
            findings: x.findings.len(),
            secrets: x.findings.iter().filter(|f| f.is_secret()).count(),
            events,
            blocks,
        }
    }
}

/// `e` as one string with its findings (those of `all` in it) placed on it. A finding that can't
/// be placed covers the whole event, so a wrong place never shows a value.
fn event(e: &Event, all: &[Finding]) -> EventDoc {
    let mine: Vec<(usize, &Finding)> = all
        .iter()
        .enumerate()
        .filter(|(_, f)| f.event == e.seq)
        .collect();
    let mut text = String::new();
    let mut marks = Vec::new();
    let mut placed = vec![false; mine.len()];
    if let Some(t) = e.text.as_deref() {
        text.push_str(t);
        for (i, (n, f)) in mine.iter().enumerate() {
            if f.field == Field::Text && f.end <= t.len() {
                marks.push(mark(f, *n, 0));
                placed[i] = true;
            }
        }
    }
    if let Some(input) = &e.input {
        if !text.is_empty() {
            text.push('\n');
        }
        let mut w = Input {
            out: &mut text,
            marks: &mut marks,
            mine: &mine,
            placed: &mut placed,
        };
        w.value(input, "", 0);
    }
    if let Some(i) = placed.iter().position(|p| !p) {
        let (n, f) = mine[i];
        marks = vec![Mark {
            start: 0,
            end: text.len(),
            ..mark(f, n, 0)
        }];
    }
    let marks = if text.is_empty() {
        Vec::new()
    } else {
        cover::fit(&text, &marks)
    };
    EventDoc {
        seq: e.seq,
        kind: e.kind,
        tool: e.tool.clone(),
        ts: e.ts.clone(),
        is_error: e.is_error.unwrap_or(false),
        model: e.model.clone(),
        text,
        marks,
    }
}

fn mark(f: &Finding, n: usize, base: usize) -> Mark {
    Mark {
        start: base + f.start,
        end: base + f.end,
        kind: f.kind,
        id: format!("f{n}"),
    }
}

/// Writes a tool call's input as indented `key: value` lines, placing the findings whose JSON
/// pointer names each key or value as it is written.
struct Input<'a> {
    out: &'a mut String,
    marks: &'a mut Vec<Mark>,
    mine: &'a [(usize, &'a Finding)],
    placed: &'a mut [bool],
}

impl Input<'_> {
    fn place(&mut self, pointer: &str, key: bool, base: usize, len: usize) {
        for (i, (n, f)) in self.mine.iter().enumerate() {
            if let Field::Input { pointer: p, key: k } = &f.field
                && p == pointer
                && *k == key
                && f.end <= len
            {
                self.marks.push(mark(f, *n, base));
                self.placed[i] = true;
            }
        }
    }

    /// A scalar as written, with its findings.
    fn scalar(&mut self, v: &Value, pointer: &str) {
        let s = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".into(),
            Value::Array(_) => "[]".into(),
            Value::Object(_) => "{}".into(),
        };
        let base = self.out.len();
        self.out.push_str(&s);
        self.place(pointer, false, base, s.len());
    }

    fn nested(v: &Value) -> bool {
        match v {
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            _ => false,
        }
    }

    fn value(&mut self, v: &Value, pointer: &str, indent: usize) {
        match v {
            Value::Object(o) if !o.is_empty() => {
                for (i, (k, x)) in o.iter().enumerate() {
                    if i > 0 {
                        self.out.push('\n');
                    }
                    self.out.push_str(&" ".repeat(indent));
                    let at = format!("{pointer}/{}", k.replace('~', "~0").replace('/', "~1"));
                    let base = self.out.len();
                    self.out.push_str(k);
                    self.place(&at, true, base, k.len());
                    self.out.push(':');
                    if Self::nested(x) {
                        self.out.push('\n');
                        self.value(x, &at, indent + 2);
                    } else {
                        self.out.push(' ');
                        self.scalar(x, &at);
                    }
                }
            }
            Value::Array(a) if !a.is_empty() => {
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        self.out.push('\n');
                    }
                    self.out.push_str(&" ".repeat(indent));
                    let at = format!("{pointer}/{i}");
                    self.out.push('-');
                    if Self::nested(x) {
                        self.out.push('\n');
                        self.value(x, &at, indent + 2);
                    } else {
                        self.out.push(' ');
                        self.scalar(x, &at);
                    }
                }
            }
            _ => self.scalar(v, pointer),
        }
    }
}

/// Every event cut into rows of at most [`BLOCK_LINES`] lines and about [`BLOCK_BYTES`] bytes,
/// never inside a finding. An event with no text still gets one row, for its heading.
pub fn blocks(events: &[EventDoc]) -> Vec<Block> {
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let t = &e.text;
        let mut start = 0;
        let mut first = true;
        loop {
            let (end, next) = cut(t, start, &e.marks);
            out.push(Block {
                event: i,
                start,
                end,
                first,
            });
            first = false;
            match next {
                Some(n) => start = n,
                None => break,
            }
        }
    }
    out
}

/// Where the row starting at `start` ends, and where the next one starts (None: this is the last).
fn cut(t: &str, start: usize, marks: &[Mark]) -> (usize, Option<usize>) {
    let rest = &t[start..];
    // After BLOCK_LINES lines, or at the line break before BLOCK_BYTES.
    let by_lines = rest
        .match_indices('\n')
        .nth(BLOCK_LINES - 1)
        .map(|(at, _)| start + at);
    let limit = floor(t, (start + BLOCK_BYTES).min(t.len()));
    let mut end = match by_lines {
        Some(e) if e <= limit => e,
        _ if limit >= t.len() => return (t.len(), None),
        _ => {
            // Too many bytes: the last break before the limit, else a space, else the limit.
            let window = &t[start..limit];
            match window.rfind('\n').or_else(|| window.rfind(' ')) {
                Some(at) if at > 0 => start + at,
                _ => limit,
            }
        }
    };
    // Never inside a finding: move past it, to the next break if there is one close by.
    while let Some(m) = marks.iter().find(|m| m.start < end && end < m.end) {
        end = m.end;
    }
    if end >= t.len() {
        return (t.len(), None);
    }
    let gap = matches!(t.as_bytes()[end], b'\n' | b' ');
    let next = if gap { end + 1 } else { end };
    (end, (next < t.len() || gap).then_some(next.min(t.len())))
}

fn floor(t: &str, mut i: usize) -> usize {
    while !t.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str, marks: Vec<Mark>) -> EventDoc {
        EventDoc {
            seq: 0,
            kind: EventKind::ToolResult,
            tool: None,
            ts: None,
            is_error: false,
            model: None,
            text: text.into(),
            marks,
        }
    }

    #[test]
    fn rows_cut_at_line_breaks_and_rejoin_to_the_whole_text() {
        let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let e = doc(text.trim_end(), vec![]);
        let b = blocks(std::slice::from_ref(&e));
        assert_eq!(b.len(), 3);
        let joined: Vec<&str> = b.iter().map(|b| &e.text[b.start..b.end]).collect();
        assert_eq!(joined.join("\n"), e.text);
        assert!(b[0].first && !b[1].first);
    }

    #[test]
    fn a_long_line_is_cut_at_a_space_and_never_inside_a_finding() {
        let text = "word ".repeat(3_000);
        let text = text.trim_end();
        let m = Mark {
            start: 5_990,
            end: 6_020,
            kind: distill_core::detect::Kind::Jwt,
            id: "f0".into(),
        };
        let e = doc(text, vec![m.clone()]);
        let b = blocks(std::slice::from_ref(&e));
        assert!(b.len() >= 2);
        for r in &b {
            assert!(r.end - r.start <= BLOCK_BYTES + 40);
            assert!(m.end <= r.start || m.start >= r.end || (m.start >= r.start && m.end <= r.end));
        }
        // Each row starts where the last ended, or just past the break it was cut at.
        assert!(
            b.windows(2)
                .all(|w| w[1].start == w[0].end || w[1].start == w[0].end + 1)
        );
        assert_eq!(b.last().unwrap().end, text.len());
    }

    #[test]
    fn an_empty_event_still_has_a_row() {
        let e = doc("", vec![]);
        assert_eq!(blocks(std::slice::from_ref(&e)).len(), 1);
    }
}
