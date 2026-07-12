//! Session projection, curation, Activity, and export mapping for legacy import.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::fingerprint::resolve_in_root_regular_file;
use super::import::IdMaps;
use super::redact::redact_legacy_activity_payload;
use crate::error::LibraryResult;
use crate::storage::{set_file_mode_600, DistillPaths};
use crate::types::{LegacyImportCounts, LegacyImportSkip, EXPORT_FORMAT_ID};

/**
 * Import legacy sessions and current message/artifact projections at generation 1.
 */
pub(super) fn import_sessions_and_projections(
    source: &Connection,
    dest: &Connection,
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<()> {
    let mut stmt = source.prepare(
        "SELECT s.id, src.kind, s.external_session_id, s.title, s.project_path, s.source_url,
                s.summary, s.started_at, s.updated_at, s.metadata_json, s.raw_capture_count
         FROM sessions s
         JOIN sources src ON src.id = s.source_id
         ORDER BY s.id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, i64>(10)?,
        ))
    })?;

    for row in rows {
        let (
            legacy_session_id,
            source_kind,
            external_session_id,
            title,
            project_path,
            source_url,
            summary,
            started_at,
            updated_at,
            metadata_json,
            raw_capture_count,
        ) = row?;

        if !maps.source_kinds.values().any(|k| k == &source_kind) {
            skips.push(LegacyImportSkip {
                category: "session".into(),
                reason: "source_not_imported".into(),
                legacy_kind: Some(source_kind),
            });
            continue;
        }

        let key = (source_kind.clone(), external_session_id.clone());
        let existing: Option<i64> = dest
            .query_row(
                "SELECT id FROM sessions WHERE source_kind = ?1 AND external_session_id = ?2",
                params![source_kind, external_session_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            maps.sessions.insert(key.clone(), id);
            maps.session_by_legacy.insert(legacy_session_id, id);
            continue;
        }

        let attempt_id = maps.session_attempt.get(&key).copied();
        let accepted = if raw_capture_count > 0 {
            raw_capture_count
        } else if attempt_id.is_some() {
            1
        } else {
            0
        };
        let attempt_count = if attempt_id.is_some() { 1 } else { 0 };
        let generation = if attempt_id.is_some() { 1_i64 } else { 0_i64 };

        dest.execute(
            "INSERT INTO sessions (
                source_kind, external_session_id, title, project_path, source_url, summary,
                started_at, updated_at, metadata_json, accepted_capture_count,
                normalization_attempt_count, successful_projection_generation, current_attempt_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                source_kind,
                external_session_id,
                title,
                project_path,
                source_url,
                summary,
                started_at,
                updated_at,
                metadata_json,
                accepted,
                attempt_count,
                generation,
                attempt_id,
            ],
        )?;
        let session_id = dest.last_insert_rowid();
        maps.sessions.insert(key, session_id);
        maps.session_by_legacy.insert(legacy_session_id, session_id);
        counts.sessions += 1;

        if generation == 1 {
            import_messages(source, dest, legacy_session_id, session_id, counts)?;
            import_artifacts(source, dest, legacy_session_id, session_id, counts, skips)?;
            let now = chrono::Utc::now().to_rfc3339();
            dest.execute(
                "INSERT INTO activity_events (
                    event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
                 ) VALUES ('projection_replaced', ?1, ?2, ?3, NULL, ?4, ?5)",
                params![
                    now,
                    source_kind,
                    session_id,
                    attempt_id,
                    serde_json::json!({
                        "origin": "legacy_electron_import",
                        "projection_generation": 1
                    })
                    .to_string(),
                ],
            )?;
        } else {
            skips.push(LegacyImportSkip {
                category: "session".into(),
                reason: "projection_without_imported_capture".into(),
                legacy_kind: Some(source_kind),
            });
        }
    }
    Ok(())
}

fn import_messages(
    source: &Connection,
    dest: &Connection,
    legacy_session_id: i64,
    session_id: i64,
    counts: &mut LegacyImportCounts,
) -> LibraryResult<()> {
    let mut stmt = source.prepare(
        "SELECT ordinal, role, text, external_message_id, created_at, message_kind, metadata_json,
                (SELECT title FROM sessions WHERE id = ?1),
                (SELECT project_path FROM sessions WHERE id = ?1)
         FROM messages
         WHERE session_id = ?1
         ORDER BY ordinal ASC",
    )?;
    let rows = stmt.query_map([legacy_session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
        ))
    })?;
    for row in rows {
        let (
            ordinal,
            role,
            text,
            external_message_id,
            created_at,
            message_kind,
            metadata_json,
            title,
            project_path,
        ) = row?;
        let kind = if message_kind == "meta" {
            "meta"
        } else {
            "text"
        };
        dest.execute(
            "INSERT INTO projection_messages (
                session_id, projection_generation, ordinal, role, message_kind, text,
                external_message_id, created_at, metadata_json
             ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                ordinal,
                role,
                kind,
                text,
                external_message_id,
                created_at,
                metadata_json,
            ],
        )?;
        let message_id = dest.last_insert_rowid();
        dest.execute(
            "INSERT INTO projection_fts (session_id, message_id, title, project_path, role, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                message_id,
                title.unwrap_or_default(),
                project_path.unwrap_or_default(),
                role,
                text,
            ],
        )?;
        counts.messages += 1;
    }
    Ok(())
}

fn import_artifacts(
    source: &Connection,
    dest: &Connection,
    legacy_session_id: i64,
    session_id: i64,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<()> {
    let mut stmt = source.prepare(
        "SELECT kind, mime_type, metadata_json
         FROM artifacts
         WHERE session_id = ?1
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([legacy_session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (kind, mime_type, metadata_json) = row?;
        dest.execute(
            "INSERT INTO projection_artifacts (
                session_id, projection_generation, message_id, capture_fact_id,
                artifact_type, media_type, text_preview, content_json, metadata_json
             ) VALUES (?1, 1, NULL, NULL, ?2, ?3, NULL, '{}', ?4)",
            params![session_id, kind, mime_type, metadata_json],
        )?;
        counts.artifacts += 1;
        skips.push(LegacyImportSkip {
            category: "projection".into(),
            reason: "artifact_links_unmapped".into(),
            legacy_kind: Some(kind),
        });
    }
    Ok(())
}

/**
 * Import tag/label descriptors and session-scoped assignments.
 */
pub(super) fn import_curation(
    source: &Connection,
    dest: &Connection,
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<()> {
    let mut tag_stmt =
        source.prepare("SELECT id, name, kind, created_at FROM tags ORDER BY id ASC")?;
    let tag_rows = tag_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in tag_rows {
        let (legacy_id, name, kind, created_at) = row?;
        let changed = dest.execute(
            "INSERT INTO tags (name, kind, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO NOTHING",
            params![name, kind, created_at],
        )?;
        let dest_id: i64 =
            dest.query_row("SELECT id FROM tags WHERE name = ?1", [&name], |row| {
                row.get(0)
            })?;
        maps.tags.insert(legacy_id, dest_id);
        if changed > 0 {
            counts.tags += 1;
        }
    }

    let mut label_stmt =
        source.prepare("SELECT id, name, scope, created_at FROM labels ORDER BY id ASC")?;
    let label_rows = label_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in label_rows {
        let (legacy_id, name, scope, created_at) = row?;
        let changed = dest.execute(
            "INSERT INTO labels (name, scope, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO NOTHING",
            params![name, scope, created_at],
        )?;
        let dest_id: i64 =
            dest.query_row("SELECT id FROM labels WHERE name = ?1", [&name], |row| {
                row.get(0)
            })?;
        maps.labels.insert(legacy_id, dest_id);
        if changed > 0 {
            counts.labels += 1;
        }
    }

    let mut ta_stmt = source.prepare(
        "SELECT object_type, object_id, tag_id, origin, confidence, created_at
         FROM tag_assignments ORDER BY id ASC",
    )?;
    let ta_rows = ta_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<f64>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in ta_rows {
        let (object_type, object_id, tag_id, origin, confidence, created_at) = row?;
        if object_type != "session" {
            skips.push(LegacyImportSkip {
                category: "curation".into(),
                reason: "unsupported_tag_object_type".into(),
                legacy_kind: Some(object_type),
            });
            continue;
        }
        let Some(&session_id) = maps.session_by_legacy.get(&object_id) else {
            skips.push(LegacyImportSkip {
                category: "curation".into(),
                reason: "tag_assignment_session_missing".into(),
                legacy_kind: None,
            });
            continue;
        };
        let Some(&dest_tag) = maps.tags.get(&tag_id) else {
            continue;
        };
        let changed = dest.execute(
            "INSERT INTO tag_assignments (
                object_type, object_id, tag_id, origin, confidence, created_at
             ) VALUES ('session', ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(object_type, object_id, tag_id, origin) DO NOTHING",
            params![session_id, dest_tag, origin, confidence, created_at],
        )?;
        if changed > 0 {
            counts.tag_assignments += 1;
        }
    }

    let mut la_stmt = source.prepare(
        "SELECT object_type, object_id, label_id, origin, created_at
         FROM label_assignments ORDER BY id ASC",
    )?;
    let la_rows = la_stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in la_rows {
        let (object_type, object_id, label_id, origin, created_at) = row?;
        if object_type != "session" {
            skips.push(LegacyImportSkip {
                category: "curation".into(),
                reason: "unsupported_label_object_type".into(),
                legacy_kind: Some(object_type),
            });
            continue;
        }
        let Some(&session_id) = maps.session_by_legacy.get(&object_id) else {
            skips.push(LegacyImportSkip {
                category: "curation".into(),
                reason: "label_assignment_session_missing".into(),
                legacy_kind: None,
            });
            continue;
        };
        let Some(&dest_label) = maps.labels.get(&label_id) else {
            continue;
        };
        let changed = dest.execute(
            "INSERT INTO label_assignments (
                object_type, object_id, label_id, origin, created_at
             ) VALUES ('session', ?1, ?2, ?3, ?4)
             ON CONFLICT(object_type, object_id, label_id) DO NOTHING",
            params![session_id, dest_label, origin, created_at],
        )?;
        if changed > 0 {
            counts.label_assignments += 1;
        }
    }
    Ok(())
}

/**
 * Import Activity-compatible events with remapped ids and redacted payloads.
 */
pub(super) fn import_activity(
    source: &Connection,
    dest: &Connection,
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<()> {
    let mut stmt = source.prepare(
        "SELECT event_type, object_type, object_id, session_id, payload_json, created_at
         FROM activity_events
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;

    const ALLOWED: &[&str] = &[
        "capture_recorded",
        "capture_failed",
        "projection_replaced",
        "tag_added",
        "tag_removed",
        "label_toggled",
        "export_written",
        "sync_queued",
        "sync_started",
        "sync_completed",
        "sync_failed",
    ];

    for row in rows {
        let (event_type, object_type, object_id, session_id, payload_json, created_at) = row?;
        if !ALLOWED.contains(&event_type.as_str()) {
            skips.push(LegacyImportSkip {
                category: "activity".into(),
                reason: "unsupported_event_type".into(),
                legacy_kind: Some(event_type),
            });
            continue;
        }
        // Skip events already synthesized during capture/session import.
        if matches!(
            event_type.as_str(),
            "capture_recorded" | "projection_replaced"
        ) {
            continue;
        }

        let mut capture_id = None;
        let mut mapped_session = session_id.and_then(|id| maps.session_by_legacy.get(&id).copied());
        let mut payload = redact_legacy_activity_payload(&payload_json);
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "origin".into(),
                serde_json::Value::String("legacy_electron_import".into()),
            );
        }

        match object_type.as_str() {
            "capture" => {
                if let Some(legacy_capture) = object_id {
                    capture_id = maps.captures.get(&legacy_capture).copied();
                    if capture_id.is_none() {
                        skips.push(LegacyImportSkip {
                            category: "activity".into(),
                            reason: "capture_not_imported".into(),
                            legacy_kind: Some(event_type),
                        });
                        continue;
                    }
                }
            }
            "session" => {
                if let Some(legacy_session) = object_id {
                    mapped_session = maps.session_by_legacy.get(&legacy_session).copied();
                    if mapped_session.is_none() {
                        skips.push(LegacyImportSkip {
                            category: "activity".into(),
                            reason: "session_not_imported".into(),
                            legacy_kind: Some(event_type),
                        });
                        continue;
                    }
                }
            }
            "export" => {
                if let Some(legacy_export) = object_id {
                    if let Some(dest_export) = maps.exports.get(&legacy_export) {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert(
                                "export_id".into(),
                                serde_json::Value::Number((*dest_export).into()),
                            );
                        }
                    }
                }
            }
            "sync_job" => {}
            other => {
                skips.push(LegacyImportSkip {
                    category: "activity".into(),
                    reason: "unsupported_object_type".into(),
                    legacy_kind: Some(other.to_string()),
                });
                continue;
            }
        }

        let source_kind = payload
            .get("sourceKind")
            .or_else(|| payload.get("source_kind"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        dest.execute(
            "INSERT INTO activity_events (
                event_type, occurred_at, source_kind, session_id, capture_id, attempt_id, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
            params![
                event_type,
                created_at,
                source_kind,
                mapped_session,
                capture_id,
                payload.to_string(),
            ],
        )?;
        counts.activity_events += 1;
    }
    Ok(())
}

/**
 * Map train/holdout export metadata and safely copy existing output bytes.
 */
pub(super) fn import_exports(
    source: &Connection,
    dest: &Connection,
    source_home: &Path,
    paths: &DistillPaths,
    maps: &mut IdMaps,
    counts: &mut LegacyImportCounts,
    skips: &mut Vec<LegacyImportSkip>,
) -> LibraryResult<Vec<std::path::PathBuf>> {
    let mut created_paths = Vec::new();
    let mut stmt = source.prepare(
        "SELECT id, export_type, label_filter, output_path, record_count, metadata_json, created_at
         FROM exports
         ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    for row in rows {
        let (
            legacy_id,
            export_type,
            label_filter,
            output_path,
            record_count,
            metadata_json,
            created_at,
        ) = row?;
        let dataset = match label_filter.as_deref() {
            Some("train") => "train",
            Some("holdout") => "holdout",
            other => {
                counts.exports_skipped += 1;
                skips.push(LegacyImportSkip {
                    category: "export".into(),
                    reason: "unsupported_dataset".into(),
                    legacy_kind: other.map(str::to_string).or(Some(export_type)),
                });
                continue;
            }
        };

        // Prefer path relative to source home / exports; also accept absolute-under-home.
        let relative_hint = Path::new(&output_path)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|name| format!("exports/{name}"))
            .unwrap_or_default();

        let resolved = match resolve_in_root_regular_file(source_home, &output_path, false)? {
            Some(path) => Some(path),
            None => resolve_in_root_regular_file(source_home, &relative_hint, false)?,
        };

        let Some(source_file) = resolved else {
            counts.exports_skipped += 1;
            skips.push(LegacyImportSkip {
                category: "export".into(),
                reason: "missing_or_unsafe_output".into(),
                legacy_kind: Some(dataset.into()),
            });
            continue;
        };

        let bytes = fs::read(&source_file)?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let byte_size = bytes.len() as u64;
        let dest_name = format!(
            "legacy-{}-{}-{}.jsonl",
            dataset,
            &sha256[..12.min(sha256.len())],
            legacy_id
        );
        let dest_rel = format!("exports/{dest_name}");
        let dest_abs = paths.home.join(&dest_rel);
        if let Some(parent) = dest_abs.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::symlink_metadata(&dest_abs) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                counts.exports_skipped += 1;
                skips.push(LegacyImportSkip {
                    category: "export".into(),
                    reason: "destination_output_unsafe".into(),
                    legacy_kind: Some(dataset.into()),
                });
                continue;
            }
            Ok(_) => {
                let existing = fs::read(&dest_abs)?;
                let existing_sha = hex::encode(Sha256::digest(&existing));
                if existing_sha != sha256 || existing.len() as u64 != byte_size {
                    counts.exports_skipped += 1;
                    skips.push(LegacyImportSkip {
                        category: "export".into(),
                        reason: "destination_output_conflict".into(),
                        legacy_kind: Some(dataset.into()),
                    });
                    continue;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                write_export_atomically(&dest_abs, &bytes)?;
                created_paths.push(dest_abs.clone());
            }
            Err(err) => return Err(err.into()),
        }

        let eligibility = serde_json::json!({
            "origin": "legacy_electron_import",
            "legacy_export_id": legacy_id,
            "metadata": redact_legacy_activity_payload(&metadata_json)
        });

        dest.execute(
            "INSERT INTO exports (
                format_id, dataset, status, created_at, updated_at, temp_path, output_path,
                sha256, byte_size, record_count, eligibility_snapshot_json, error_class, error_message
             ) VALUES (?1, ?2, 'published', ?3, ?3, NULL, ?4, ?5, ?6, ?7, ?8, NULL, NULL)",
            params![
                EXPORT_FORMAT_ID,
                dataset,
                created_at,
                dest_rel,
                sha256,
                byte_size as i64,
                record_count,
                eligibility.to_string(),
            ],
        )?;
        let export_id = dest.last_insert_rowid();
        maps.exports.insert(legacy_id, export_id);
        counts.exports += 1;
    }
    Ok(created_paths)
}

/** Write a Library-owned legacy export through a same-volume temp+rename. */
fn write_export_atomically(path: &Path, bytes: &[u8]) -> LibraryResult<()> {
    let temp_path = path.with_extension("jsonl.tmp");
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        set_file_mode_600(&temp_path)?;
        fs::rename(&temp_path, path)?;
        set_file_mode_600(path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}
