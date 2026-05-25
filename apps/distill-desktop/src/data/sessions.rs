use std::fmt::Write as _;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::view_models::{
    ArtifactCardVm, DetailContextRowVm, SessionBadgeVm, SessionDetailVm, SessionLane,
    SessionListRowVm, SessionWorkflowState, SessionsPageVm, TranscriptMessageVm,
};

use super::{
    DesktopDataSource, derive_session_preview, derive_session_title, matches_query,
    parse_json_object, truncate_inline, uppercase_role,
};

#[derive(Clone, Debug)]
struct SessionListRow {
    id: i64,
    source_kind: String,
    title: Option<String>,
    project_path: Option<String>,
    updated_at: Option<String>,
    message_count: i64,
    model: Option<String>,
    git_branch: Option<String>,
    first_user_text: Option<String>,
    first_assistant_text: Option<String>,
    labels_summary: Option<String>,
}

#[derive(Clone, Debug)]
struct SessionDetailRow {
    id: i64,
    source_kind: String,
    external_session_id: String,
    title: Option<String>,
    project_path: Option<String>,
    source_url: Option<String>,
    started_at: Option<String>,
    updated_at: Option<String>,
    message_count: i64,
    raw_capture_count: i64,
    model: Option<String>,
    git_branch: Option<String>,
    summary: Option<String>,
    metadata_json: Option<String>,
    first_user_text: Option<String>,
    first_assistant_text: Option<String>,
    artifact_count: i64,
}

#[derive(Clone, Debug)]
struct MessageRow {
    ordinal: i64,
    role: String,
    text: String,
    created_at: Option<String>,
    message_kind: String,
}

#[derive(Clone, Debug)]
struct ArtifactRow {
    kind: String,
    mime_type: Option<String>,
    metadata_json: String,
    created_at: Option<String>,
    source_line_no: Option<i64>,
    message_ordinal: Option<i64>,
    message_role: Option<String>,
}

#[derive(Clone, Debug)]
struct TagRow {
    name: String,
}

#[derive(Clone, Debug)]
struct LabelRow {
    name: String,
}

impl DesktopDataSource {
    pub fn load_sessions(
        &self,
        lane: SessionLane,
        query: &str,
        selected_session_id: Option<i64>,
    ) -> Result<SessionsPageVm> {
        if !self.database_exists() {
            return Ok(SessionsPageVm {
                rows: Vec::new(),
                empty_title: match self.source_mode() {
                    crate::config::SourceMode::RustOwned => "No sessions yet".to_string(),
                    crate::config::SourceMode::ElectronCompatReadOnly => {
                        "No Distill data yet".to_string()
                    }
                },
                empty_message: match self.source_mode() {
                    crate::config::SourceMode::RustOwned => {
                        "Sync Codex CLI or Claude Code into the Rust-owned store to populate this view."
                            .to_string()
                    }
                    crate::config::SourceMode::ElectronCompatReadOnly => {
                        format!(
                            "Expected a read-only Distill Electron database at {}.",
                            self.database_path().display()
                        )
                    }
                },
            });
        }

        let conn = self.open_read_only()?;
        let rows = self.list_sessions_from_db(&conn)?;
        let filtered = rows
            .into_iter()
            .filter(|row| matches_session_lane(workflow_state_from_row(row), lane))
            .filter(|row| {
                matches_query(
                    query,
                    &[
                        &row.title,
                        &row.preview,
                        &row.message_count_text,
                        &row.updated_at_text,
                        &row.git_branch_text,
                    ],
                )
            })
            .map(|mut row| {
                row.selected = Some(row.id) == selected_session_id;
                row
            })
            .collect::<Vec<_>>();

        let (empty_title, empty_message) = if filtered.is_empty() {
            if query.trim().is_empty() {
                (
                    format!("No sessions in {}", lane.label()),
                    match self.source_mode() {
                        crate::config::SourceMode::RustOwned => {
                            "Sync local history into the Rust-owned Distill store to populate this workflow lane."
                                .to_string()
                        }
                        crate::config::SourceMode::ElectronCompatReadOnly => {
                            "Import data in Distill Electron, then reopen this desktop shell."
                                .to_string()
                        }
                    },
                )
            } else {
                (
                    "No sessions match the current search and workflow lane.".to_string(),
                    "Adjust the search query or switch workflow lanes.".to_string(),
                )
            }
        } else {
            (String::new(), String::new())
        };

        Ok(SessionsPageVm {
            rows: filtered,
            empty_title,
            empty_message,
        })
    }

    pub fn load_session_detail(&self, session_id: i64) -> Result<SessionDetailVm> {
        if !self.database_exists() {
            return Ok(SessionDetailVm {
                empty_title: "No session selected".to_string(),
                empty_message: match self.source_mode() {
                    crate::config::SourceMode::RustOwned => {
                        "The desktop shell has not initialized a Rust-owned Distill database yet."
                            .to_string()
                    }
                    crate::config::SourceMode::ElectronCompatReadOnly => {
                        "The desktop shell has not found a compatible Distill Electron database yet."
                            .to_string()
                    }
                },
                ..SessionDetailVm::default()
            });
        }

        let conn = self.open_read_only()?;
        let row = conn
            .query_row(
                r#"
                SELECT
                  s.id,
                  so.kind AS source_kind,
                  s.external_session_id,
                  s.title,
                  s.project_path,
                  s.source_url,
                  s.started_at,
                  s.updated_at,
                  s.message_count,
                  s.raw_capture_count,
                  s.model,
                  s.git_branch,
                  s.summary,
                  s.metadata_json,
                  (
                    SELECT m.text
                    FROM messages m
                    WHERE m.session_id = s.id AND m.role = 'user' AND m.message_kind = 'text'
                    ORDER BY m.ordinal ASC
                    LIMIT 1
                  ) AS first_user_text,
                  (
                    SELECT m.text
                    FROM messages m
                    WHERE m.session_id = s.id AND m.role = 'assistant' AND m.message_kind = 'text'
                    ORDER BY m.ordinal ASC
                    LIMIT 1
                  ) AS first_assistant_text,
                  (
                    SELECT COUNT(*)
                    FROM artifacts a
                    WHERE a.session_id = s.id
                  ) AS artifact_count
                FROM sessions s
                JOIN sources so ON so.id = s.source_id
                WHERE s.id = ?
                "#,
                [session_id],
                |row| {
                    Ok(SessionDetailRow {
                        id: row.get(0)?,
                        source_kind: row.get(1)?,
                        external_session_id: row.get(2)?,
                        title: row.get(3)?,
                        project_path: row.get(4)?,
                        source_url: row.get(5)?,
                        started_at: row.get(6)?,
                        updated_at: row.get(7)?,
                        message_count: row.get(8)?,
                        raw_capture_count: row.get(9)?,
                        model: row.get(10)?,
                        git_branch: row.get(11)?,
                        summary: row.get(12)?,
                        metadata_json: row.get(13)?,
                        first_user_text: row.get(14)?,
                        first_assistant_text: row.get(15)?,
                        artifact_count: row.get(16)?,
                    })
                },
            )
            .optional()?;

        let Some(row) = row else {
            return Ok(SessionDetailVm {
                empty_title: "Session missing".to_string(),
                empty_message:
                    "The selected session could not be loaded from the current projection."
                        .to_string(),
                ..SessionDetailVm::default()
            });
        };

        let metadata = parse_json_object(row.metadata_json.as_deref());
        let messages = self.load_messages(&conn, session_id)?;
        let artifacts = self.load_artifacts(&conn, session_id)?;
        let tags = self.load_tags(&conn, session_id)?;
        let labels = self.load_labels(&conn, session_id)?;

        let mut context_rows = vec![
            DetailContextRowVm {
                label: "External Session".to_string(),
                value: row.external_session_id.clone(),
                presentation: "copy".to_string(),
            },
            DetailContextRowVm {
                label: "Raw Captures".to_string(),
                value: row.raw_capture_count.to_string(),
                presentation: "value".to_string(),
            },
            DetailContextRowVm {
                label: "Messages".to_string(),
                value: row.message_count.to_string(),
                presentation: "value".to_string(),
            },
            DetailContextRowVm {
                label: "Artifacts".to_string(),
                value: row.artifact_count.to_string(),
                presentation: "value".to_string(),
            },
        ];
        push_context_if_some(&mut context_rows, "Project", row.project_path.as_deref());
        push_context_if_some(&mut context_rows, "Started", row.started_at.as_deref());
        push_context_if_some(&mut context_rows, "Updated", row.updated_at.as_deref());
        push_context_if_some(&mut context_rows, "Model", row.model.as_deref());
        push_context_if_some(&mut context_rows, "Git Branch", row.git_branch.as_deref());
        push_context_if_some(&mut context_rows, "Source URL", row.source_url.as_deref());

        let secondary_badges = detail_badges(&row);
        let transcript_rows = messages
            .into_iter()
            .map(|message| TranscriptMessageVm {
                role: uppercase_role(&message.role),
                message_kind: message.message_kind,
                ordinal_text: format!("#{}", message.ordinal),
                timestamp_text: message
                    .created_at
                    .unwrap_or_else(|| "undated".to_string()),
                body: message.text,
            })
            .collect::<Vec<_>>();
        let artifact_cards = artifacts
            .into_iter()
            .map(|artifact| ArtifactCardVm {
                title: artifact_summary(&artifact),
                meta: artifact_meta(&artifact),
                preview: artifact_preview(&artifact),
                payload_json: artifact_payload_json(&artifact),
            })
            .collect::<Vec<_>>();

        let empty_message = if transcript_rows.is_empty() {
            "No projected transcript messages were found for this session.".to_string()
        } else {
            String::new()
        };

        Ok(SessionDetailVm {
            id: Some(row.id),
            title: derive_session_title(row.title.as_deref(), row.first_user_text.as_deref()),
            summary: row
                .summary
                .or_else(|| {
                    derive_session_preview(
                        row.first_assistant_text.as_deref(),
                        row.first_user_text.as_deref(),
                    )
                })
                .unwrap_or_else(|| {
                    "No session summary is available for the current projection.".to_string()
                }),
            secondary_badges,
            labels: labels.into_iter().map(|label| label.name).collect(),
            tags: tags.into_iter().map(|tag| tag.name).collect(),
            context_rows,
            provenance_json: if metadata.is_empty() {
                String::new()
            } else {
                serde_json::to_string_pretty(&metadata).unwrap_or_default()
            },
            messages: transcript_rows,
            artifacts: artifact_cards,
            export_enabled: false,
            curation_enabled: false,
            empty_title: String::new(),
            empty_message,
        })
    }

    fn list_sessions_from_db(&self, conn: &Connection) -> Result<Vec<SessionListRowVm>> {
        let mut statement = conn.prepare(
            r#"
            WITH
            first_user_message AS (
              SELECT session_id, text
              FROM (
                SELECT
                  session_id,
                  text,
                  ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY ordinal ASC) AS row_no
                FROM messages
                WHERE role = 'user' AND message_kind = 'text'
              )
              WHERE row_no = 1
            ),
            first_assistant_message AS (
              SELECT session_id, text
              FROM (
                SELECT
                  session_id,
                  text,
                  ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY ordinal ASC) AS row_no
                FROM messages
                WHERE role = 'assistant' AND message_kind = 'text'
              )
              WHERE row_no = 1
            ),
            session_labels AS (
              SELECT
                session_id,
                GROUP_CONCAT(name, ', ') AS labels_summary
              FROM (
                SELECT
                  la.object_id AS session_id,
                  l.name AS name
                FROM label_assignments la
                JOIN labels l ON l.id = la.label_id
                WHERE la.object_type = 'session' AND la.origin = 'manual'
                ORDER BY la.object_id, l.name
              )
              GROUP BY session_id
            )
            SELECT
              s.id,
              so.kind AS source_kind,
              s.title,
              s.project_path,
              s.updated_at,
              s.message_count,
              s.model,
              s.git_branch,
              fu.text AS first_user_text,
              fa.text AS first_assistant_text,
              sl.labels_summary
            FROM sessions s
            JOIN sources so ON so.id = s.source_id
            LEFT JOIN first_user_message fu ON fu.session_id = s.id
            LEFT JOIN first_assistant_message fa ON fa.session_id = s.id
            LEFT JOIN session_labels sl ON sl.session_id = s.id
            ORDER BY COALESCE(s.updated_at, s.updated_recorded_at) DESC
            "#,
        )?;
        let mut rows = statement.query([])?;
        let mut sessions = Vec::new();
        while let Some(row) = rows.next()? {
            let item = SessionListRow {
                id: row.get(0)?,
                source_kind: row.get(1)?,
                title: row.get(2)?,
                project_path: row.get(3)?,
                updated_at: row.get(4)?,
                message_count: row.get(5)?,
                model: row.get(6)?,
                git_branch: row.get(7)?,
                first_user_text: row.get(8)?,
                first_assistant_text: row.get(9)?,
                labels_summary: row.get(10)?,
            };
            let labels = split_labels_summary(item.labels_summary.as_deref());
            let workflow_state = derive_workflow_state(&labels);
            let title =
                derive_session_title(item.title.as_deref(), item.first_user_text.as_deref());
            let preview = derive_session_preview(
                item.first_assistant_text.as_deref(),
                item.first_user_text.as_deref(),
            )
            .unwrap_or_else(|| "No assistant preview".to_string());

            sessions.push(SessionListRowVm {
                id: item.id,
                title,
                preview,
                source_badge: Some(source_badge_for_kind(&item.source_kind)),
                workflow_badge: workflow_badge_for_state(workflow_state),
                model_badge: item.model.as_ref().map(|model| SessionBadgeVm {
                    text: model.clone(),
                    tone: "muted".to_string(),
                }),
                message_count_text: format!("{} msgs", item.message_count),
                updated_at_text: item.updated_at.unwrap_or_else(|| "undated".to_string()),
                git_branch_text: item.git_branch.unwrap_or_default(),
                selected: false,
            });
        }
        Ok(sessions)
    }

    fn load_messages(&self, conn: &Connection, session_id: i64) -> Result<Vec<MessageRow>> {
        let mut statement = conn.prepare(
            r#"
            SELECT ordinal, role, text, created_at, message_kind
            FROM messages
            WHERE session_id = ?
            ORDER BY ordinal ASC
            "#,
        )?;
        let mut rows = statement.query([session_id])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            messages.push(MessageRow {
                ordinal: row.get(0)?,
                role: row.get(1)?,
                text: row.get(2)?,
                created_at: row.get(3)?,
                message_kind: row.get(4)?,
            });
        }
        Ok(messages)
    }

    fn load_artifacts(&self, conn: &Connection, session_id: i64) -> Result<Vec<ArtifactRow>> {
        let mut statement = conn.prepare(
            r#"
            SELECT
              a.kind,
              a.mime_type,
              a.metadata_json,
              a.created_at,
              cr.line_no AS source_line_no,
              m.ordinal AS message_ordinal,
              m.role AS message_role
            FROM artifacts a
            LEFT JOIN capture_records cr ON cr.id = a.capture_record_id
            LEFT JOIN messages m ON m.id = a.message_id
            WHERE a.session_id = ?
            ORDER BY COALESCE(m.ordinal, 999999), COALESCE(cr.line_no, 999999), a.id
            "#,
        )?;
        let mut rows = statement.query([session_id])?;
        let mut artifacts = Vec::new();
        while let Some(row) = rows.next()? {
            artifacts.push(ArtifactRow {
                kind: row.get(0)?,
                mime_type: row.get(1)?,
                metadata_json: row.get(2)?,
                created_at: row.get(3)?,
                source_line_no: row.get(4)?,
                message_ordinal: row.get(5)?,
                message_role: row.get(6)?,
            });
        }
        Ok(artifacts)
    }

    fn load_tags(&self, conn: &Connection, session_id: i64) -> Result<Vec<TagRow>> {
        let mut statement = conn.prepare(
            r#"
            SELECT t.name
            FROM tag_assignments ta
            JOIN tags t ON t.id = ta.tag_id
            WHERE ta.object_type = 'session'
              AND ta.object_id = ?
            ORDER BY t.name ASC
            "#,
        )?;
        let mut rows = statement.query([session_id])?;
        let mut tags = Vec::new();
        while let Some(row) = rows.next()? {
            tags.push(TagRow { name: row.get(0)? });
        }
        Ok(tags)
    }

    fn load_labels(&self, conn: &Connection, session_id: i64) -> Result<Vec<LabelRow>> {
        let mut statement = conn.prepare(
            r#"
            SELECT l.name
            FROM label_assignments la
            JOIN labels l ON l.id = la.label_id
            WHERE la.object_type = 'session'
              AND la.object_id = ?
              AND la.origin = 'manual'
            ORDER BY l.name ASC
            "#,
        )?;
        let mut rows = statement.query([session_id])?;
        let mut labels = Vec::new();
        while let Some(row) = rows.next()? {
            labels.push(LabelRow { name: row.get(0)? });
        }
        Ok(labels)
    }
}

fn workflow_state_from_row(row: &SessionListRowVm) -> SessionWorkflowState {
    match row.workflow_badge.as_ref().map(|badge| badge.text.as_str()) {
        Some("review") => SessionWorkflowState::NeedsReview,
        Some("train") => SessionWorkflowState::TrainReady,
        Some("holdout") => SessionWorkflowState::HoldoutReady,
        Some("favorite") => SessionWorkflowState::Favorite,
        _ => SessionWorkflowState::Neutral,
    }
}

fn split_labels_summary(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn derive_workflow_state(labels: &[String]) -> SessionWorkflowState {
    let has_train = labels.iter().any(|label| label == "train");
    let has_holdout = labels.iter().any(|label| label == "holdout");
    let has_exclude = labels.iter().any(|label| label == "exclude");
    let has_sensitive = labels.iter().any(|label| label == "sensitive");
    let has_favorite = labels.iter().any(|label| label == "favorite");

    if has_exclude || has_sensitive || (has_train && has_holdout) {
        SessionWorkflowState::NeedsReview
    } else if has_train {
        SessionWorkflowState::TrainReady
    } else if has_holdout {
        SessionWorkflowState::HoldoutReady
    } else if has_favorite {
        SessionWorkflowState::Favorite
    } else {
        SessionWorkflowState::Neutral
    }
}

fn matches_session_lane(workflow: SessionWorkflowState, lane: SessionLane) -> bool {
    match lane {
        SessionLane::All => true,
        SessionLane::NeedsReview => matches!(workflow, SessionWorkflowState::NeedsReview),
        SessionLane::TrainReady => matches!(workflow, SessionWorkflowState::TrainReady),
        SessionLane::HoldoutReady => matches!(workflow, SessionWorkflowState::HoldoutReady),
        SessionLane::Favorite => matches!(workflow, SessionWorkflowState::Favorite),
    }
}

fn source_badge_for_kind(source_kind: &str) -> SessionBadgeVm {
    SessionBadgeVm {
        text: match source_kind {
            "claude_code" => "claude".to_string(),
            "opencode" => "opencode".to_string(),
            _ => "codex".to_string(),
        },
        tone: match source_kind {
            "claude_code" => "source_claude".to_string(),
            "opencode" => "source_opencode".to_string(),
            _ => "source_codex".to_string(),
        },
    }
}

fn workflow_badge_for_state(workflow: SessionWorkflowState) -> Option<SessionBadgeVm> {
    let label = workflow.label();
    if label.is_empty() {
        None
    } else {
        Some(SessionBadgeVm {
            text: label.to_string(),
            tone: workflow.tone().to_string(),
        })
    }
}

fn detail_badges(row: &SessionDetailRow) -> Vec<SessionBadgeVm> {
    let mut badges = vec![source_badge_for_kind(&row.source_kind)];
    if let Some(model) = row.model.as_ref() {
        badges.push(SessionBadgeVm {
            text: model.clone(),
            tone: "muted".to_string(),
        });
    }
    badges.push(SessionBadgeVm {
        text: format!("{} msgs", row.message_count),
        tone: "muted".to_string(),
    });
    badges.push(SessionBadgeVm {
        text: format!("{} artifacts", row.artifact_count),
        tone: "muted".to_string(),
    });
    if let Some(branch) = row.git_branch.as_ref() {
        badges.push(SessionBadgeVm {
            text: format!("⌇ {branch}"),
            tone: "muted".to_string(),
        });
    }
    if let Some(project) = row.project_path.as_ref() {
        badges.push(SessionBadgeVm {
            text: truncate_inline(project, 32),
            tone: "muted".to_string(),
        });
    }
    if let Some(updated_at) = row.updated_at.as_ref() {
        badges.push(SessionBadgeVm {
            text: truncate_inline(updated_at, 24),
            tone: "muted".to_string(),
        });
    }
    badges
}

fn push_context_if_some(target: &mut Vec<DetailContextRowVm>, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        target.push(DetailContextRowVm {
            label: label.to_string(),
            value: value.to_string(),
            presentation: "value".to_string(),
        });
    }
}

fn artifact_summary(row: &ArtifactRow) -> String {
    let mut summary = row.kind.replace('_', " ");
    if let Some(mime_type) = row.mime_type.as_deref() {
        summary.push_str(" · ");
        summary.push_str(mime_type);
    }
    if let Some(message_ordinal) = row.message_ordinal {
        let _ = write!(summary, " · msg #{message_ordinal}");
    }
    summary
}

fn artifact_meta(row: &ArtifactRow) -> String {
    let mut parts = Vec::new();
    if let Some(created_at) = row.created_at.as_deref() {
        parts.push(created_at.to_string());
    }
    if let Some(source_line_no) = row.source_line_no {
        parts.push(format!("line {source_line_no}"));
    }
    if let Some(message_role) = row.message_role.as_deref() {
        parts.push(message_role.to_string());
    }
    parts.join(" · ")
}

fn artifact_payload_json(row: &ArtifactRow) -> String {
    let payload = parse_json_object(Some(&row.metadata_json));
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| row.metadata_json.clone())
}

fn artifact_preview(row: &ArtifactRow) -> String {
    truncate_inline(&artifact_payload_json(row).replace('\n', " "), 120)
}
