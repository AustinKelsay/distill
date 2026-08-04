//! Per-Source preference persistence.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::adapter::SourceKind;
use crate::error::{LibraryError, LibraryResult};
use crate::ops::paths::canonicalize_configured_root;
use crate::types::SourcePreference;

/**
 * List known Source preferences, ensuring closed kinds appear even before first upsert.
 */
pub fn list_source_preferences(conn: &Connection) -> LibraryResult<Vec<SourcePreference>> {
    let mut by_kind = std::collections::BTreeMap::new();
    for kind in SourceKind::all() {
        by_kind.insert(
            kind.as_str().to_string(),
            SourcePreference {
                kind: kind.as_str().to_string(),
                enabled: false,
                configured_root: None,
                display_name: None,
                data_root: None,
            },
        );
    }

    let mut stmt = conn.prepare(
        "SELECT kind, enabled, configured_root, display_name, data_root
         FROM sources
         ORDER BY kind ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SourcePreference {
            kind: row.get(0)?,
            enabled: row.get::<_, i64>(1)? != 0,
            configured_root: row.get(2)?,
            display_name: row.get(3)?,
            data_root: row.get(4)?,
        })
    })?;
    for row in rows {
        let pref = row?;
        by_kind.insert(pref.kind.clone(), pref);
    }
    Ok(by_kind.into_values().collect())
}

/**
 * Upsert enabled/disabled and optional configured-root preference for one Source.
 *
 * Parameters:
 * - `conn`: Library SQLite connection.
 * - `kind`: closed Source kind string.
 * - `enabled`: whether Sync may include this Source.
 * - `configured_root`: optional override; `None` clears the override.
 */
pub fn upsert_source_preference(
    conn: &Connection,
    kind: &str,
    enabled: bool,
    configured_root: Option<&Path>,
) -> LibraryResult<SourcePreference> {
    let source_kind = SourceKind::parse(kind)
        .ok_or_else(|| LibraryError::InvalidArgument(format!("unknown source kind: {kind}")))?;
    let canonical_root = match configured_root {
        Some(root) => Some(canonicalize_configured_root(root)?),
        None => None,
    };
    let now = chrono::Utc::now().to_rfc3339();
    let root_text = canonical_root
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let display_name = default_display_name(source_kind);

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM sources WHERE kind = ?1",
            [source_kind.as_str()],
            |row| row.get(0),
        )
        .optional()?;

    if existing.is_some() {
        conn.execute(
            "UPDATE sources
             SET enabled = ?1,
                 configured_root = ?2,
                 updated_at = ?3
             WHERE kind = ?4",
            params![
                if enabled { 1 } else { 0 },
                root_text,
                now,
                source_kind.as_str()
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO sources (
                kind, display_name, data_root, metadata_json, created_at, updated_at,
                enabled, configured_root
             ) VALUES (?1, ?2, NULL, '{}', ?3, ?3, ?4, ?5)",
            params![
                source_kind.as_str(),
                display_name,
                now,
                if enabled { 1 } else { 0 },
                root_text
            ],
        )?;
    }

    list_source_preferences(conn)?
        .into_iter()
        .find(|pref| pref.kind == source_kind.as_str())
        .ok_or_else(|| {
            LibraryError::NotFound(format!("source preference {}", source_kind.as_str()))
        })
}

fn default_display_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Fixture => "Fixture",
        SourceKind::Codex => "Codex",
        SourceKind::ClaudeCode => "Claude Code",
        SourceKind::OpenCode => "OpenCode",
        SourceKind::Droid => "Droid",
        SourceKind::Pi => "Pi",
    }
}
