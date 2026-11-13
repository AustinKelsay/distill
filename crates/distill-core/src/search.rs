//! Search and listing over normalized sessions. A linear scan of the files; fast enough for tens
//! of thousands of sessions, and there is no index to go stale.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::home::Home;
use crate::model::SessionMeta;
use crate::normalize;
use crate::source::SourceKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub source: SourceKind,
    pub session_id: String,
    pub title: Option<String>,
    pub started_at: Option<String>,
    pub seq: u64,
    pub kind: crate::model::EventKind,
    pub snippet: String,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub source: Option<SourceKind>,
    pub text: Option<String>,
    pub since: Option<String>,
    pub include_steward: bool,
}

pub fn list(home: &Home, f: &ListFilter, limit: usize) -> Result<Vec<SessionMeta>> {
    let needle = f.text.as_deref().map(str::to_lowercase);
    Ok(normalize::read_index(home)?
        .into_iter()
        .filter(|m| f.include_steward || !m.steward)
        .filter(|m| f.source.is_none_or(|s| m.source == s))
        .filter(|m| {
            f.since
                .as_deref()
                .is_none_or(|s| m.started_at.as_deref().is_some_and(|t| t >= s))
        })
        .filter(|m| {
            needle.as_deref().is_none_or(|n| {
                m.title
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains(n))
                    || m.cwd
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(n))
            })
        })
        .take(limit)
        .collect())
}

/// Find events whose text contains the query (case-insensitive), newest sessions first.
pub fn search(
    home: &Home,
    query: &str,
    source: Option<SourceKind>,
    limit: usize,
) -> Result<Vec<Hit>> {
    let q = query.to_lowercase();
    let mut hits = Vec::new();
    if q.trim().is_empty() {
        return Ok(hits);
    }
    for meta in normalize::read_index(home)? {
        if source.is_some_and(|s| s != meta.source) {
            continue;
        }
        for e in normalize::read_events(home, &meta)? {
            let Some(text) = e.text.as_deref() else {
                continue;
            };
            let lower = text.to_lowercase();
            let Some(pos) = lower.find(&q) else { continue };
            hits.push(Hit {
                source: meta.source,
                session_id: meta.session_id.clone(),
                title: meta.title.clone(),
                started_at: meta.started_at.clone(),
                seq: e.seq,
                kind: e.kind,
                snippet: snippet(text, &lower, pos, q.len()),
            });
            if hits.len() >= limit {
                return Ok(hits);
            }
        }
    }
    Ok(hits)
}

fn snippet(text: &str, lower: &str, pos: usize, len: usize) -> String {
    // Work in the lowercased string's byte offsets, then map back via char boundaries.
    let start = lower[..pos]
        .char_indices()
        .rev()
        .nth(80)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end_target = (pos + len + 160).min(lower.len());
    let end = (end_target..=lower.len())
        .find(|&i| lower.is_char_boundary(i))
        .unwrap_or(lower.len());
    let slice = if text.len() == lower.len() {
        &text[start..end]
    } else {
        &lower[start..end]
    };
    let mut s = slice.replace('\n', " ");
    if start > 0 {
        s.insert(0, '…');
    }
    if end < lower.len() {
        s.push('…');
    }
    s
}
