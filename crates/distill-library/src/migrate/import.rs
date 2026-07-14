//! Legacy Electron → native Library import orchestration.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::content::{resolve_legacy_capture_content, store_resolved_content};
use super::fingerprint::{compute_source_fingerprints, SourceFingerprints};
use super::map::{
    import_activity, import_curation, import_exports, import_sessions_and_projections,
};
use super::paths::{open_legacy_readonly, snapshot_legacy_database, validate_legacy_import_paths};
use crate::error::LibraryResult;
use crate::storage::{ContentRef, DistillPaths};
use crate::types::{
    LegacyImportCounts, LegacyImportReport, LegacyImportSkip, LEGACY_IMPORT_PARSER_ID,
    LEGACY_IMPORT_PARSER_VERSION,
};

/// In-memory id maps from legacy INTEGER ids to destination ids.
pub(super) struct IdMaps {
    pub(super) sources: HashMap<i64, i64>,
    pub(super) source_kinds: HashMap<i64, String>,
    pub(super) captures: HashMap<i64, i64>,
    pub(super) attempts: HashMap<i64, i64>,
    /// (source_kind, external_session_id) → destination session id
    pub(super) sessions: HashMap<(String, String), i64>,
    /// legacy session id → destination session id
    pub(super) session_by_legacy: HashMap<i64, i64>,
    pub(super) tags: HashMap<i64, i64>,
    pub(super) labels: HashMap<i64, i64>,
    pub(super) exports: HashMap<i64, i64>,
    /// capture_id → attempt_id for projection linkage
    pub(super) capture_attempt: HashMap<i64, i64>,
    /// (source_kind, external_session_id) → best attempt for current projection
    pub(super) session_attempt: HashMap<(String, String), i64>,
}

impl IdMaps {
    fn new() -> Self {
        Self {
            sources: HashMap::new(),
            source_kinds: HashMap::new(),
            captures: HashMap::new(),
            attempts: HashMap::new(),
            sessions: HashMap::new(),
            session_by_legacy: HashMap::new(),
            tags: HashMap::new(),
            labels: HashMap::new(),
            exports: HashMap::new(),
            capture_attempt: HashMap::new(),
            session_attempt: HashMap::new(),
        }
    }
}

/**
 * Import a legacy Electron Distill home into an already-open native Library home.
 *
 * A private source snapshot opens read-only. Destination writes use one atomic
 * SQLite transaction plus a durable fingerprint marker so identical sources
 * reuse the prior redacted report without duplicating rows.
 */
pub fn import_legacy_electron_home(
    dest_conn: &mut Connection,
    paths: &DistillPaths,
    source_home: &Path,
) -> LibraryResult<LegacyImportReport> {
    let (source_home, _dest_home) = validate_legacy_import_paths(source_home, &paths.home)?;
    let initial_fingerprints = compute_source_fingerprints(&source_home)?;
    let snapshot = snapshot_legacy_database(&source_home, &paths.staging)?;
    let fingerprints = compute_source_fingerprints(&source_home)?;
    if initial_fingerprints.source_fingerprint != fingerprints.source_fingerprint {
        return Err(crate::error::LibraryError::InvalidArgument(
            "legacy home changed during migration; retry with the Electron app closed".into(),
        ));
    }
    if let Some(prior) = load_prior_report(dest_conn, &fingerprints.source_fingerprint)? {
        return Ok(prior);
    }

    let source = open_legacy_readonly(snapshot.path())?;
    let mut skips = Vec::new();
    let mut counts = LegacyImportCounts::default();
    let mut maps = IdMaps::new();

    // Persist capture bytes before the destination transaction (CAS is checksum-keyed).
    let staged_captures = stage_captures(&source, &source_home, paths, &mut skips, &mut counts)?;

    let mut created_export_paths = Vec::new();
    let result = (|| {
        let tx = dest_conn.transaction()?;
        import_sources(&source, &tx, &mut maps, &mut counts, &mut skips)?;
        import_staged_captures(
            &tx,
            &staged_captures.items,
            &mut maps,
            &mut counts,
            &mut skips,
        )?;
        import_capture_facts(&source, &tx, &mut maps, &mut counts)?;
        import_sessions_and_projections(&source, &tx, &mut maps, &mut counts, &mut skips)?;
        import_curation(&source, &tx, &mut maps, &mut counts, &mut skips)?;
        created_export_paths.extend(import_exports(
            &source,
            &tx,
            &source_home,
            paths,
            &mut maps,
            &mut counts,
            &mut skips,
        )?);
        import_activity(&source, &tx, &mut maps, &mut counts, &mut skips)?;

        let report = LegacyImportReport {
            ok: true,
            reused_prior_import: false,
            source_fingerprint: fingerprints.source_fingerprint.clone(),
            source_db_sha256: fingerprints.source_db_sha256.clone(),
            content_fingerprint: fingerprints.content_fingerprint.clone(),
            counts,
            skips,
        };
        persist_marker(&tx, &fingerprints, &report)?;
        tx.commit()?;
        Ok(report)
    })();

    match result {
        Ok(report) => {
            cleanup_unreferenced_import_files(
                dest_conn,
                paths,
                &staged_captures.created_blob_paths,
                &[],
            )?;
            Ok(report)
        }
        Err(err) => {
            cleanup_unreferenced_import_files(
                dest_conn,
                paths,
                &staged_captures.created_blob_paths,
                &created_export_paths,
            )?;
            Err(err)
        }
    }
}

/** Remove files created by an import whose destination transaction rolled back. */
fn cleanup_unreferenced_import_files(
    conn: &Connection,
    paths: &DistillPaths,
    created_blob_paths: &[std::path::PathBuf],
    created_export_paths: &[std::path::PathBuf],
) -> LibraryResult<()> {
    for path in created_blob_paths {
        let Some(relative_path) = path
            .strip_prefix(&paths.home)
            .ok()
            .map(|value| value.to_string_lossy().replace('\\', "/"))
        else {
            continue;
        };
        let references: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captures WHERE blob_path = ?1",
            [&relative_path],
            |row| row.get(0),
        )?;
        if references == 0 && path.is_file() && !path.is_symlink() {
            fs::remove_file(path)?;
        }
    }
    for path in created_export_paths {
        let relative = path
            .strip_prefix(&paths.home)
            .ok()
            .map(|value| value.to_string_lossy().replace('\\', "/"));
        let Some(relative) = relative else {
            continue;
        };
        let references: i64 = conn.query_row(
            "SELECT COUNT(*) FROM exports WHERE output_path = ?1",
            [&relative],
            |row| row.get(0),
        )?;
        if references == 0 && path.is_file() && !path.is_symlink() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn load_prior_report(
    conn: &Connection,
    source_fingerprint: &str,
) -> LibraryResult<Option<LegacyImportReport>> {
    let row: Option<String> = conn
        .query_row(
            "SELECT report_json FROM legacy_import_markers WHERE source_fingerprint = ?1",
            [source_fingerprint],
            |row| row.get(0),
        )
        .optional()?;
    match row {
        Some(json) => {
            let mut report: LegacyImportReport = serde_json::from_str(&json)?;
            report.reused_prior_import = true;
            Ok(Some(report))
        }
        None => Ok(None),
    }
}

fn persist_marker(
    conn: &Connection,
    fingerprints: &SourceFingerprints,
    report: &LegacyImportReport,
) -> LibraryResult<()> {
    conn.execute(
        "INSERT INTO legacy_import_markers (
            source_fingerprint, source_db_sha256, content_fingerprint, report_json, imported_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            fingerprints.source_fingerprint,
            fingerprints.source_db_sha256,
            fingerprints.content_fingerprint,
            serde_json::to_string(report)?,
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

struct StagedCapture {
    legacy_id: i64,
    legacy_source_id: i64,
    source_path: String,
    external_session_id: Option<String>,
    source_modified_at: Option<String>,
    content: ContentRef,
    status: String,
}

struct StagedCaptures {
    items: Vec<StagedCapture>,
    created_blob_paths: Vec<std::path::PathBuf>,
}

fn stage_captures(
    source: &Connection,
    source_home: &Path,
    paths: &DistillPaths,
    skips: &mut Vec<LegacyImportSkip>,
    counts: &mut LegacyImportCounts,
) -> LibraryResult<StagedCaptures> {
    let mut stmt = source.prepare(
        "SELECT id, source_id, source_path, external_session_id, source_modified_at,
                raw_sha256, raw_blob_path, raw_payload_json, status
         FROM captures
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;

    let mut staged = Vec::new();
    let mut created_blob_paths = Vec::new();
    for row in rows {
        let (
            legacy_id,
            legacy_source_id,
            source_path,
            external_session_id,
            source_modified_at,
            raw_sha256,
            raw_blob_path,
            raw_payload_json,
            status,
        ) = row?;
        let Some(resolved) = resolve_legacy_capture_content(
            source_home,
            &raw_sha256,
            raw_blob_path.as_deref(),
            raw_payload_json.as_deref(),
            skips,
        )?
        else {
            counts.captures_skipped += 1;
            continue;
        };
        let (content, created_blob) = store_resolved_content(paths, &resolved)?;
        if created_blob {
            if let ContentRef::Blob { relative_path, .. } = &content {
                created_blob_paths.push(paths.home.join(relative_path));
            }
        }
        let source_path = source_path
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("legacy-capture:{legacy_id}"));
        staged.push(StagedCapture {
            legacy_id,
            legacy_source_id,
            source_path,
            external_session_id,
            source_modified_at,
            content,
            status,
        });
    }
    Ok(StagedCaptures {
        items: staged,
        created_blob_paths,
    })
}

fn import_sources(
    source: &Connection,
    dest: &Connection,
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<()> {
    let mut stmt = source.prepare(
        "SELECT id, kind, display_name, data_root, metadata_json, created_at, updated_at
         FROM sources ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    for row in rows {
        let (legacy_id, kind, display_name, data_root, metadata_json, created_at, updated_at) =
            row?;
        if !is_known_source_kind(&kind) {
            skips.push(LegacyImportSkip {
                category: "source".into(),
                reason: "unsupported_source_kind".into(),
                legacy_kind: Some(kind),
            });
            continue;
        }
        // Insert only when absent — never mutate an existing native Source row.
        let changed = dest.execute(
            "INSERT INTO sources (
                kind, display_name, data_root, metadata_json, created_at, updated_at,
                enabled, configured_root
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL)
             ON CONFLICT(kind) DO NOTHING",
            params![
                kind,
                display_name,
                data_root,
                metadata_json,
                created_at,
                updated_at
            ],
        )?;
        let dest_id: i64 =
            dest.query_row("SELECT id FROM sources WHERE kind = ?1", [&kind], |row| {
                row.get(0)
            })?;
        maps.sources.insert(legacy_id, dest_id);
        maps.source_kinds.insert(legacy_id, kind);
        if changed > 0 {
            counts.sources += 1;
        }
    }
    Ok(())
}

fn is_known_source_kind(kind: &str) -> bool {
    matches!(
        kind,
        "fixture" | "codex" | "claude_code" | "opencode" | "droid"
    )
}

fn import_staged_captures(
    dest: &Connection,
    staged: &[StagedCapture],
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for item in staged {
        let Some(&source_id) = maps.sources.get(&item.legacy_source_id) else {
            counts.captures_skipped += 1;
            skips.push(LegacyImportSkip {
                category: "capture".into(),
                reason: "source_not_imported".into(),
                legacy_kind: Some("unknown_source".into()),
            });
            continue;
        };
        let source_kind = maps
            .source_kinds
            .get(&item.legacy_source_id)
            .cloned()
            .unwrap_or_default();
        let (content_kind, inline_text, blob_path, sha256, byte_size, media_type) =
            content_columns(&item.content);

        // Skip when destination already owns this exact Capture key.
        let existing: Option<i64> = dest
            .query_row(
                "SELECT id FROM captures WHERE source_kind = ?1 AND source_path = ?2 AND sha256 = ?3",
                params![source_kind, item.source_path, sha256],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            maps.captures.insert(item.legacy_id, id);
            let existing_attempt: Option<(i64, String)> = dest
                .query_row(
                    "SELECT id, outcome FROM normalization_attempts
                     WHERE capture_id = ?1 ORDER BY id DESC LIMIT 1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((attempt_id, outcome)) = existing_attempt {
                maps.attempts.insert(item.legacy_id, attempt_id);
                maps.capture_attempt.insert(item.legacy_id, attempt_id);
                if outcome == "succeeded" {
                    if let Some(external) = item.external_session_id.as_ref() {
                        maps.session_attempt
                            .insert((source_kind, external.clone()), attempt_id);
                    }
                }
            }
            continue;
        }

        dest.execute(
            "INSERT INTO captures (
                source_id, source_kind, source_path, external_session_id,
                content_kind, media_type, sha256, byte_size, inline_text, blob_path,
                source_modified_at, accepted_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                source_id,
                source_kind,
                item.source_path,
                item.external_session_id,
                content_kind,
                media_type,
                sha256,
                byte_size as i64,
                inline_text,
                blob_path,
                item.source_modified_at,
                now,
            ],
        )?;
        let capture_id = dest.last_insert_rowid();
        maps.captures.insert(item.legacy_id, capture_id);
        counts.captures += 1;

        let outcome = if item.status == "failed_parse" {
            "failed"
        } else {
            "succeeded"
        };
        let (error_class, error_message) = if outcome == "failed" {
            (
                Some("legacy_failed_parse"),
                Some("imported legacy failed_parse capture"),
            )
        } else {
            (None, None)
        };
        dest.execute(
            "INSERT INTO normalization_attempts (
                capture_id, parser_id, parser_version, started_at, finished_at,
                outcome, error_class, error_message, projection_generation, metrics_json
             ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, '{}')",
            params![
                capture_id,
                LEGACY_IMPORT_PARSER_ID,
                LEGACY_IMPORT_PARSER_VERSION,
                now,
                outcome,
                error_class,
                error_message,
                if outcome == "succeeded" {
                    Some(1_i64)
                } else {
                    None
                },
            ],
        )?;
        let attempt_id = dest.last_insert_rowid();
        maps.attempts.insert(item.legacy_id, attempt_id);
        maps.capture_attempt.insert(item.legacy_id, attempt_id);
        counts.attempts += 1;

        dest.execute(
            "INSERT INTO activity_events (
                event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
             ) VALUES ('capture_recorded', ?1, ?2, NULL, ?3, ?4, ?5)",
            params![
                now,
                source_kind,
                capture_id,
                attempt_id,
                serde_json::json!({
                    "origin": "legacy_electron_import",
                    "legacy_capture_id": item.legacy_id
                })
                .to_string(),
            ],
        )?;

        if outcome == "succeeded" {
            if let Some(external) = item.external_session_id.as_ref() {
                maps.session_attempt
                    .insert((source_kind, external.clone()), attempt_id);
            }
        }
    }
    Ok(())
}

fn content_columns(
    content: &ContentRef,
) -> (&'static str, Option<&str>, Option<&str>, &str, u64, &str) {
    match content {
        ContentRef::Inline {
            text,
            sha256,
            byte_size,
            media_type,
        } => (
            "inline",
            Some(text.as_str()),
            None,
            sha256.as_str(),
            *byte_size,
            media_type.as_str(),
        ),
        ContentRef::Blob {
            relative_path,
            sha256,
            byte_size,
            media_type,
        } => (
            "blob",
            None,
            Some(relative_path.as_str()),
            sha256.as_str(),
            *byte_size,
            media_type.as_str(),
        ),
    }
}

fn import_capture_facts(
    source: &Connection,
    dest: &Connection,
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
) -> LibraryResult<()> {
    let mut stmt = source.prepare(
        "SELECT capture_id, line_no, record_type, role, is_meta, content_text, content_json, metadata_json
         FROM capture_records
         ORDER BY capture_id ASC, line_no ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    for row in rows {
        let (
            legacy_capture_id,
            line_no,
            record_type,
            role,
            is_meta,
            content_text,
            content_json,
            metadata_json,
        ) = row?;
        let Some(&attempt_id) = maps.capture_attempt.get(&legacy_capture_id) else {
            continue;
        };
        let changed = dest.execute(
            "INSERT INTO capture_facts (
                attempt_id, ordinal, record_type, role, is_meta, content_text, content_json, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(attempt_id, ordinal) DO NOTHING",
            params![
                attempt_id,
                line_no,
                record_type,
                role,
                if is_meta != 0 { 1 } else { 0 },
                content_text,
                content_json,
                metadata_json,
            ],
        )?;
        if changed > 0 {
            counts.facts += 1;
        }
    }
    Ok(())
}
