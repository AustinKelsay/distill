use crate::data::{DbBrowseRequestVm, DesktopDataSource};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::view_models::{
    AppRoute, AppSnapshotVm, DbBrowseVm, DbExplorerVm, DbGridRowVm, DbQueryVm, DbTab, DbTableVm,
    LogEntryVm, LogFilter, SessionDetailVm, SessionLane, SessionListRowVm,
};
use crate::{
    AppWindow, ArtifactCardData, DbColumnHeaderData, DbGridRowData, DbSchemaColumnData, DbStore,
    DetailContextRowData, KeyValueRowData, LogEntryData, LogsStore, SessionBadgeData,
    SessionLaneData, SessionListRowData, SessionsStore, ShellStatData, ShellStore, SourceRowData,
    TableRowData, TranscriptMessageData, ViewTabData,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DesktopPreferences {
    pub route: AppRoute,
    pub sessions_width: f32,
    pub logs_width: f32,
    pub db_tables_width: f32,
    pub db_rows_width: f32,
    pub selected_session_id: Option<i64>,
    pub selected_log_id: Option<String>,
    pub selected_table_name: Option<String>,
    pub db_query_sql: String,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            route: AppRoute::Sessions,
            sessions_width: 340.0,
            logs_width: 420.0,
            db_tables_width: 240.0,
            db_rows_width: 420.0,
            selected_session_id: None,
            selected_log_id: None,
            selected_table_name: Some("sessions".to_string()),
            db_query_sql:
                "SELECT id, title, updated_at\nFROM sessions\nORDER BY updated_at DESC\nLIMIT 25;"
                    .to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct SessionsState {
    lane: SessionLane,
    query: String,
    rows: Vec<SessionListRowVm>,
    selected_id: Option<i64>,
    detail: SessionDetailVm,
    empty_title: String,
    empty_message: String,
}

impl Default for SessionsState {
    fn default() -> Self {
        Self {
            lane: SessionLane::All,
            query: String::new(),
            rows: Vec::new(),
            selected_id: None,
            detail: empty_session_detail(
                "Select a session",
                "Choose a conversation from the left to inspect the transcript, labels, tags, and artifacts.",
            ),
            empty_title: String::new(),
            empty_message: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct LogsState {
    filter: LogFilter,
    query: String,
    entries: Vec<LogEntryVm>,
    expanded_id: Option<String>,
    summary_total_text: String,
    summary_error_text: String,
    summary_sync_text: String,
    summary_sync_tone: String,
    empty_title: String,
    empty_message: String,
}

impl Default for LogsState {
    fn default() -> Self {
        Self {
            filter: LogFilter::All,
            query: String::new(),
            entries: Vec::new(),
            expanded_id: None,
            summary_total_text: "0 entries".to_string(),
            summary_error_text: "0 errors".to_string(),
            summary_sync_text: "idle".to_string(),
            summary_sync_tone: "idle".to_string(),
            empty_title: String::new(),
            empty_message: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SortDirection {
    Asc,
    #[default]
    Desc,
}

impl SortDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }

    fn from_value(value: &str) -> Self {
        if value.eq_ignore_ascii_case("asc") {
            Self::Asc
        } else {
            Self::Desc
        }
    }
}

#[derive(Clone, Debug, Default)]
struct DbFilterState {
    column: String,
    operator: String,
    value: String,
}

#[derive(Clone, Debug, Default)]
struct DbSortState {
    column: String,
    direction: SortDirection,
}

#[derive(Clone, Debug)]
struct DbState {
    tables: Vec<DbTableVm>,
    selected_table_name: Option<String>,
    browse: DbBrowseVm,
    selected_row_key: Option<String>,
    filter: DbFilterState,
    sort: DbSortState,
    page: usize,
    query: DbQueryVm,
    active_tab: DbTab,
}

impl Default for DbState {
    fn default() -> Self {
        Self {
            tables: Vec::new(),
            selected_table_name: Some("sessions".to_string()),
            browse: DbBrowseVm::default(),
            selected_row_key: None,
            filter: DbFilterState {
                operator: "contains".to_string(),
                ..DbFilterState::default()
            },
            sort: DbSortState::default(),
            page: 1,
            query: default_db_query(),
            active_tab: DbTab::Browse,
        }
    }
}

#[derive(Clone, Debug)]
struct AppState {
    route: AppRoute,
    snapshot: AppSnapshotVm,
    prefs: DesktopPreferences,
    sessions: SessionsState,
    logs: LogsState,
    db: DbState,
    sources_open: bool,
    settings_open: bool,
}

impl AppState {
    fn from_prefs(prefs: DesktopPreferences) -> Self {
        let mut db = DbState::default();
        db.selected_table_name = prefs.selected_table_name.clone();
        db.query.sql = prefs.db_query_sql.clone();

        Self {
            route: prefs.route,
            snapshot: AppSnapshotVm::default(),
            sessions: SessionsState {
                selected_id: prefs.selected_session_id,
                query: String::new(),
                ..SessionsState::default()
            },
            logs: LogsState {
                expanded_id: prefs.selected_log_id.clone(),
                ..LogsState::default()
            },
            db,
            prefs,
            sources_open: false,
            settings_open: false,
        }
    }
}

pub struct DesktopController {
    window: AppWindow,
    source: DesktopDataSource,
    state: AppState,
    prefs_path: PathBuf,
}

impl DesktopController {
    pub fn new(
        window: &AppWindow,
        source: DesktopDataSource,
        prefs_path: PathBuf,
    ) -> Rc<RefCell<Self>> {
        let prefs = load_preferences(&prefs_path).unwrap_or_default();
        let controller = Rc::new(RefCell::new(Self {
            window: window.clone_strong(),
            source,
            state: AppState::from_prefs(prefs),
            prefs_path,
        }));

        bind_callbacks(&controller);
        controller.borrow_mut().reload_all();
        controller
    }

    pub fn reload_all(&mut self) {
        self.load_snapshot();
        self.reload_sessions_state();
        self.reload_logs_state();
        self.reload_db_state();
        self.persist_preferences();
        self.render();
    }

    fn sync_or_reload(&mut self) {
        if !self.state.snapshot.sync_status.enabled {
            return;
        }

        match self.source.sync_now("manual_reload") {
            Ok(_) => self.reload_all(),
            Err(error) => {
                self.load_snapshot();
                self.state.snapshot.app_status_text = format!("sync: {error}");
                self.render();
            }
        }
    }

    fn switch_route(&mut self, route: AppRoute) {
        self.state.route = route;
        self.persist_preferences();
        self.render();
    }

    fn update_global_search(&mut self, text: String) {
        self.state.sessions.query = text;
        self.reload_sessions_state();
        self.persist_preferences();
        self.render();
    }

    fn set_session_lane(&mut self, lane: SessionLane) {
        self.state.sessions.lane = lane;
        self.reload_sessions_state();
        self.persist_preferences();
        self.render();
    }

    fn select_session(&mut self, session_id: i64) {
        if !self.state.sessions.rows.iter().any(|row| row.id == session_id) {
            return;
        }

        self.state.sessions.selected_id = Some(session_id);
        mark_selected_session_rows(&mut self.state.sessions.rows, self.state.sessions.selected_id);
        self.state.sessions.detail = self
            .source
            .load_session_detail(session_id)
            .unwrap_or_else(|error| error_session_detail(&error));
        self.persist_preferences();
        self.render();
    }

    fn toggle_sources_panel(&mut self) {
        self.state.sources_open = !self.state.sources_open;
        self.render();
    }

    fn open_settings(&mut self) {
        self.state.settings_open = true;
        self.render();
    }

    fn close_settings(&mut self) {
        self.state.settings_open = false;
        self.render();
    }

    fn set_log_filter(&mut self, filter: LogFilter) {
        self.state.logs.filter = filter;
        self.reload_logs_state();
        self.persist_preferences();
        self.render();
    }

    fn update_logs_search(&mut self, text: String) {
        self.state.logs.query = text;
        self.reload_logs_state();
        self.persist_preferences();
        self.render();
    }

    fn toggle_log_expanded(&mut self, log_id: String) {
        self.state.logs.expanded_id = if self.state.logs.expanded_id.as_deref() == Some(log_id.as_str()) {
            None
        } else {
            Some(log_id)
        };
        self.reload_logs_state();
        self.persist_preferences();
        self.render();
    }

    fn select_db_table(&mut self, table_name: String) {
        self.state.db.selected_table_name = Some(table_name);
        self.state.db.page = 1;
        self.reload_db_state();
        self.persist_preferences();
        self.render();
    }

    fn set_db_filter_column(&mut self, column: String) {
        self.state.db.filter.column = column;
        self.persist_preferences();
        self.render();
    }

    fn set_db_filter_operator(&mut self, operator: String) {
        self.state.db.filter.operator = operator;
        self.persist_preferences();
        self.render();
    }

    fn set_db_filter_value(&mut self, value: String) {
        self.state.db.filter.value = value;
        self.persist_preferences();
        self.render();
    }

    fn set_db_sort_column(&mut self, column: String) {
        self.state.db.sort.column = column;
        self.persist_preferences();
        self.render();
    }

    fn set_db_sort_direction(&mut self, direction: SortDirection) {
        self.state.db.sort.direction = direction;
        self.persist_preferences();
        self.render();
    }

    fn apply_db_browse(&mut self) {
        self.state.db.page = self.state.db.page.max(1);
        self.reload_db_browse();
        self.persist_preferences();
        self.render();
    }

    fn clear_db_filter(&mut self) {
        self.state.db.filter.value.clear();
        self.state.db.filter.operator = "contains".to_string();
        self.state.db.page = 1;
        self.reload_db_browse();
        self.persist_preferences();
        self.render();
    }

    fn change_db_page(&mut self, delta: PageDelta) {
        match delta {
            PageDelta::Prev => {
                if self.state.db.page > 1 {
                    self.state.db.page -= 1;
                }
            }
            PageDelta::Next => {
                self.state.db.page = self.state.db.page.max(1) + 1;
            }
        }

        self.reload_db_browse();
        self.persist_preferences();
        self.render();
    }

    fn select_db_row(&mut self, row_key: String) {
        if !self
            .state
            .db
            .browse
            .result_rows
            .iter()
            .any(|row| row.key == row_key)
        {
            return;
        }

        self.state.db.selected_row_key = Some(row_key);
        reconcile_db_row_selection(
            &mut self.state.db.browse.result_rows,
            &mut self.state.db.selected_row_key,
            &mut self.state.db.browse.row_detail,
        );
        self.persist_preferences();
        self.render();
    }

    fn update_db_query(&mut self, sql: String) {
        self.state.db.query.sql = sql;
        self.persist_preferences();
        self.render();
    }

    fn run_db_query(&mut self) {
        self.state.db.query = self
            .source
            .run_read_only_query(&self.state.db.query.sql)
            .unwrap_or_else(|error| DbQueryVm {
                sql: self.state.db.query.sql.clone(),
                summary: String::new(),
                preview: String::new(),
                error: error.to_string(),
            });
        self.persist_preferences();
        self.render();
    }

    fn switch_db_tab(&mut self, tab: DbTab) {
        self.state.db.active_tab = tab;
        self.render();
    }

    fn adjust_split(&mut self, pane: PaneWidth, delta: i32) {
        let delta = delta as f32;
        match pane {
            PaneWidth::Sessions => {
                self.state.prefs.sessions_width =
                    (self.state.prefs.sessions_width + delta).clamp(280.0, 520.0);
            }
            PaneWidth::DbTables => {
                self.state.prefs.db_tables_width =
                    (self.state.prefs.db_tables_width + delta).clamp(180.0, 360.0);
            }
            PaneWidth::DbRows => {
                self.state.prefs.db_rows_width =
                    (self.state.prefs.db_rows_width + delta).clamp(280.0, 680.0);
            }
        }

        self.persist_preferences();
        self.render();
    }

    fn load_snapshot(&mut self) {
        match self.source.app_snapshot() {
            Ok(snapshot) => {
                self.state.snapshot = snapshot;
            }
            Err(error) => {
                self.state.snapshot = AppSnapshotVm {
                    app_status_text: format!("snapshot: {error}"),
                    source_badge_text: "Rust Desktop".to_string(),
                    source_mode_label: self.source.source_mode().label().to_string(),
                    sync_status: crate::view_models::SyncStatusVm {
                        text: "unavailable".to_string(),
                        tone: "warning".to_string(),
                        enabled: false,
                        button_label: "Sync".to_string(),
                    },
                    ..AppSnapshotVm::default()
                };
            }
        }
    }

    fn reload_sessions_state(&mut self) {
        match self.source.load_sessions(
            self.state.sessions.lane,
            &self.state.sessions.query,
            self.state.sessions.selected_id,
        ) {
            Ok(page) => {
                self.state.sessions.rows = page.rows;
                self.state.sessions.empty_title = page.empty_title;
                self.state.sessions.empty_message = page.empty_message;
                self.state.sessions.selected_id = reconcile_session_selection(
                    &mut self.state.sessions.rows,
                    self.state.sessions.selected_id,
                );
                self.state.sessions.detail =
                    if let Some(session_id) = self.state.sessions.selected_id {
                        self.source
                            .load_session_detail(session_id)
                            .unwrap_or_else(|error| error_session_detail(&error))
                    } else {
                        empty_session_detail(
                            &self.state.sessions.empty_title,
                            &self.state.sessions.empty_message,
                        )
                    };
            }
            Err(error) => {
                self.state.snapshot.app_status_text = format!("sessions: {error}");
                self.state.sessions.rows.clear();
                self.state.sessions.selected_id = None;
                self.state.sessions.empty_title = "Sessions unavailable".to_string();
                self.state.sessions.empty_message = error.to_string();
                self.state.sessions.detail =
                    empty_session_detail("Sessions unavailable", &error.to_string());
            }
        }
    }

    fn reload_logs_state(&mut self) {
        match self.source.load_logs(
            self.state.logs.filter,
            &self.state.logs.query,
            self.state.logs.expanded_id.as_deref(),
        ) {
            Ok(page) => {
                self.state.logs.summary_total_text = page.summary_total_text;
                self.state.logs.summary_error_text = page.summary_error_text;
                self.state.logs.summary_sync_text = page.summary_sync_text;
                self.state.logs.summary_sync_tone = page.summary_sync_tone;
                self.state.logs.empty_title = page.empty_title;
                self.state.logs.empty_message = page.empty_message;
                self.state.logs.entries = page.entries;
                self.state.logs.expanded_id = self
                    .state
                    .logs
                    .entries
                    .iter()
                    .find(|entry| entry.expanded)
                    .map(|entry| entry.id.clone());
            }
            Err(error) => {
                self.state.snapshot.app_status_text = format!("logs: {error}");
                self.state.logs.entries.clear();
                self.state.logs.expanded_id = None;
                self.state.logs.summary_total_text = "0 entries".to_string();
                self.state.logs.summary_error_text = "0 errors".to_string();
                self.state.logs.summary_sync_text = "unavailable".to_string();
                self.state.logs.summary_sync_tone = "warning".to_string();
                self.state.logs.empty_title = "Logs unavailable".to_string();
                self.state.logs.empty_message = error.to_string();
            }
        }
    }

    fn reload_db_state(&mut self) {
        match self
            .source
            .load_db_snapshot(self.state.db.selected_table_name.as_deref())
        {
            Ok(snapshot) => self.apply_db_snapshot(snapshot),
            Err(error) => {
                self.state.snapshot.app_status_text = format!("db: {error}");
                self.state.db.tables.clear();
                self.state.db.selected_table_name = None;
                self.state.db.browse = DbBrowseVm {
                    error: error.to_string(),
                    ..DbBrowseVm::default()
                };
                self.state.db.selected_row_key = None;
            }
        }
    }

    fn apply_db_snapshot(&mut self, snapshot: DbExplorerVm) {
        self.state.db.tables = snapshot.tables;
        self.state.db.selected_table_name = snapshot.selected_table_name;
        if self.state.db.selected_table_name.is_none() {
            self.state.db.browse = DbBrowseVm::default();
            self.state.db.selected_row_key = None;
            return;
        }

        self.state.db.filter.column = choose_valid_value(
            self.state.db.filter.column.clone(),
            &snapshot.browse.filter_columns,
        );
        if self.state.db.filter.operator.is_empty() {
            self.state.db.filter.operator = "contains".to_string();
        }
        self.state.db.sort.column = choose_valid_value(
            self.state.db.sort.column.clone(),
            &snapshot.browse.sort_columns,
        );
        if self.state.db.sort.column.is_empty() {
            self.state.db.sort.column =
                choose_valid_value(snapshot.sort_column, &snapshot.browse.sort_columns);
        }
        self.state.db.sort.direction = if snapshot.sort_direction.is_empty() {
            SortDirection::Desc
        } else {
            SortDirection::from_value(&snapshot.sort_direction)
        };
        self.state.db.page = self.state.db.page.max(1);

        self.reload_db_browse();
    }

    fn reload_db_browse(&mut self) {
        let Some(table_name) = self.state.db.selected_table_name.clone() else {
            self.state.db.browse = DbBrowseVm::default();
            self.state.db.selected_row_key = None;
            return;
        };

        match self.source.browse_db_table(DbBrowseRequestVm {
            table_name,
            filter_column: self.state.db.filter.column.clone(),
            filter_operator: self.state.db.filter.operator.clone(),
            filter_value: self.state.db.filter.value.clone(),
            sort_column: self.state.db.sort.column.clone(),
            sort_direction: self.state.db.sort.direction.as_str().to_string(),
            page: self.state.db.page.max(1),
        }) {
            Ok(mut browse) => {
                self.state.db.filter.column = choose_valid_value(
                    self.state.db.filter.column.clone(),
                    &browse.filter_columns,
                );
                self.state.db.sort.column = choose_valid_value(
                    self.state.db.sort.column.clone(),
                    &browse.sort_columns,
                );
                reconcile_db_row_selection(
                    &mut browse.result_rows,
                    &mut self.state.db.selected_row_key,
                    &mut browse.row_detail,
                );
                self.state.db.browse = browse;
            }
            Err(error) => {
                self.state.snapshot.app_status_text = format!("db: {error}");
                self.state.db.browse = DbBrowseVm {
                    error: error.to_string(),
                    ..DbBrowseVm::default()
                };
                self.state.db.selected_row_key = None;
            }
        }
    }

    fn render(&self) {
        self.render_shell();
        self.render_sessions();
        self.render_logs();
        self.render_db();
    }

    fn render_shell(&self) {
        self.window.set_active_route(self.state.route.as_index());
        self.window
            .set_global_search(self.state.sessions.query.clone().into());
        self.window
            .set_sessions_list_width(self.state.prefs.sessions_width);
        self.window
            .set_db_tables_width(self.state.prefs.db_tables_width);
        self.window
            .set_db_rows_width(self.state.prefs.db_rows_width);

        let shell_store = self.window.global::<ShellStore>();
        let view_tabs = vec![
            ViewTabData {
                label: SharedString::from("Sessions"),
                selected: matches!(self.state.route, AppRoute::Sessions),
            },
            ViewTabData {
                label: SharedString::from("DB"),
                selected: matches!(self.state.route, AppRoute::Db),
            },
            ViewTabData {
                label: SharedString::from("Logs"),
                selected: matches!(self.state.route, AppRoute::Logs),
            },
        ];
        shell_store.set_view_tabs(ModelRc::new(VecModel::from(view_tabs)));

        let shell_stats = self
            .state
            .snapshot
            .shell_stats
            .iter()
            .map(|stat| ShellStatData {
                label: stat.label.clone().into(),
                value: stat.value.clone().into(),
            })
            .collect::<Vec<_>>();
        shell_store.set_shell_stats(ModelRc::new(VecModel::from(shell_stats)));

        let source_rows = self
            .state
            .snapshot
            .source_rows
            .iter()
            .map(|row| SourceRowData {
                display_name: row.display_name.clone().into(),
                status_text: row.status_text.clone().into(),
                status_tone: row.status_tone.clone().into(),
                data_root: row.data_root.clone().into(),
                checks_summary: summarize_source_checks(row).into(),
                is_stub: row.is_stub,
            })
            .collect::<Vec<_>>();
        shell_store.set_source_rows(ModelRc::new(VecModel::from(source_rows)));

        shell_store.set_show_onboarding(self.state.snapshot.show_onboarding);
        shell_store.set_onboarding_title(self.state.snapshot.onboarding_title.clone().into());
        shell_store.set_onboarding_message(self.state.snapshot.onboarding_message.clone().into());
        shell_store.set_sidebar_count_label(self.state.snapshot.sidebar_count_label.clone().into());
        shell_store.set_scanned_label(self.state.snapshot.scanned_at_label.clone().into());
        shell_store.set_source_mode_text(self.state.snapshot.source_mode_label.clone().into());
        shell_store.set_source_badge_text(self.state.snapshot.source_badge_text.clone().into());
        shell_store.set_sync_status_text(self.state.snapshot.sync_status.text.clone().into());
        shell_store.set_sync_status_tone(self.state.snapshot.sync_status.tone.clone().into());
        shell_store.set_sync_button_label(self.state.snapshot.sync_status.button_label.clone().into());
        shell_store.set_sync_enabled(self.state.snapshot.sync_status.enabled);
        shell_store.set_sources_open(self.state.sources_open);
        shell_store.set_settings_open(self.state.settings_open);

        let storage_rows = settings_section_rows(&self.state.snapshot, "Storage")
            .into_iter()
            .map(to_key_value_data)
            .collect::<Vec<_>>();
        shell_store.set_settings_storage_rows(ModelRc::new(VecModel::from(storage_rows)));
        let source_setting_rows = settings_section_rows(&self.state.snapshot, "Sources")
            .into_iter()
            .map(to_key_value_data)
            .collect::<Vec<_>>();
        shell_store.set_settings_source_rows(ModelRc::new(VecModel::from(source_setting_rows)));
        let sync_rows = settings_section_rows(&self.state.snapshot, "Sync")
            .into_iter()
            .map(to_key_value_data)
            .collect::<Vec<_>>();
        shell_store.set_settings_sync_rows(ModelRc::new(VecModel::from(sync_rows)));
        let curation_labels = self
            .state
            .snapshot
            .settings
            .labels
            .iter()
            .map(|label| SharedString::from(label.as_str()))
            .collect::<Vec<_>>();
        shell_store.set_settings_curation_labels(ModelRc::new(VecModel::from(curation_labels)));
    }

    fn render_sessions(&self) {
        let sessions_store = self.window.global::<SessionsStore>();
        let lanes = SessionLane::ALL
            .iter()
            .map(|lane| SessionLaneData {
                label: SharedString::from(lane.label()),
                selected: *lane == self.state.sessions.lane,
            })
            .collect::<Vec<_>>();
        sessions_store.set_session_lanes(ModelRc::new(VecModel::from(lanes)));

        let rows = self
            .state
            .sessions
            .rows
            .iter()
            .map(|row| SessionListRowData {
                id: row.id as i32,
                title: row.title.clone().into(),
                preview: row.preview.clone().into(),
                source_badge_text: row
                    .source_badge
                    .as_ref()
                    .map(|badge| badge.text.clone())
                    .unwrap_or_default()
                    .into(),
                source_badge_tone: row
                    .source_badge
                    .as_ref()
                    .map(|badge| badge.tone.clone())
                    .unwrap_or_default()
                    .into(),
                workflow_badge_text: row
                    .workflow_badge
                    .as_ref()
                    .map(|badge| badge.text.clone())
                    .unwrap_or_default()
                    .into(),
                workflow_badge_tone: row
                    .workflow_badge
                    .as_ref()
                    .map(|badge| badge.tone.clone())
                    .unwrap_or_default()
                    .into(),
                model_badge_text: row
                    .model_badge
                    .as_ref()
                    .map(|badge| badge.text.clone())
                    .unwrap_or_default()
                    .into(),
                message_count_text: row.message_count_text.clone().into(),
                updated_at_text: row.updated_at_text.clone().into(),
                git_branch_text: row.git_branch_text.clone().into(),
                selected: row.selected,
            })
            .collect::<Vec<_>>();
        sessions_store.set_session_rows(ModelRc::new(VecModel::from(rows)));

        let secondary_badges = self
            .state
            .sessions
            .detail
            .secondary_badges
            .iter()
            .map(|badge| SessionBadgeData {
                text: badge.text.clone().into(),
                tone: badge.tone.clone().into(),
            })
            .collect::<Vec<_>>();
        sessions_store.set_session_secondary_badges(ModelRc::new(VecModel::from(secondary_badges)));

        let labels = self
            .state
            .sessions
            .detail
            .labels
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>();
        sessions_store.set_session_labels(ModelRc::new(VecModel::from(labels)));

        let tags = self
            .state
            .sessions
            .detail
            .tags
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>();
        sessions_store.set_session_tags(ModelRc::new(VecModel::from(tags)));

        let context_rows = self
            .state
            .sessions
            .detail
            .context_rows
            .iter()
            .map(|row| DetailContextRowData {
                label: row.label.clone().into(),
                value: row.value.clone().into(),
                presentation: row.presentation.clone().into(),
            })
            .collect::<Vec<_>>();
        sessions_store.set_session_context_rows(ModelRc::new(VecModel::from(context_rows)));

        let transcript_rows = self
            .state
            .sessions
            .detail
            .messages
            .iter()
            .map(|row| TranscriptMessageData {
                role: row.role.clone().into(),
                message_kind: row.message_kind.clone().into(),
                ordinal_text: row.ordinal_text.clone().into(),
                timestamp_text: row.timestamp_text.clone().into(),
                body: row.body.clone().into(),
            })
            .collect::<Vec<_>>();
        sessions_store.set_transcript_rows(ModelRc::new(VecModel::from(transcript_rows)));

        let artifact_rows = self
            .state
            .sessions
            .detail
            .artifacts
            .iter()
            .map(|row| ArtifactCardData {
                title: row.title.clone().into(),
                meta: row.meta.clone().into(),
                preview: row.preview.clone().into(),
                payload_json: row.payload_json.clone().into(),
            })
            .collect::<Vec<_>>();
        sessions_store.set_artifact_rows(ModelRc::new(VecModel::from(artifact_rows)));

        sessions_store.set_sessions_empty_title(self.state.sessions.empty_title.clone().into());
        sessions_store.set_sessions_empty_message(self.state.sessions.empty_message.clone().into());
        sessions_store.set_session_detail_title(self.state.sessions.detail.title.clone().into());
        sessions_store
            .set_session_detail_summary(self.state.sessions.detail.summary.clone().into());
        sessions_store
            .set_session_provenance_json(self.state.sessions.detail.provenance_json.clone().into());
        sessions_store.set_export_enabled(self.state.sessions.detail.export_enabled);
        sessions_store.set_curation_enabled(self.state.sessions.detail.curation_enabled);
        sessions_store
            .set_session_detail_empty_title(self.state.sessions.detail.empty_title.clone().into());
        sessions_store.set_session_detail_empty_message(
            self.state.sessions.detail.empty_message.clone().into(),
        );
    }

    fn render_logs(&self) {
        let logs_store = self.window.global::<LogsStore>();
        let entries = self
            .state
            .logs
            .entries
            .iter()
            .map(|row| LogEntryData {
                id: row.id.clone().into(),
                title: row.title.clone().into(),
                subtitle: row.subtitle.clone().into(),
                summary: row.summary.clone().into(),
                status: row.status.clone().into(),
                level: row.level.clone().into(),
                metrics: row.metrics.clone().into(),
                raw_json: row.raw_json.clone().into(),
                expanded: row.expanded,
            })
            .collect::<Vec<_>>();
        logs_store.set_log_entries(ModelRc::new(VecModel::from(entries)));
        logs_store.set_logs_empty_title(self.state.logs.empty_title.clone().into());
        logs_store.set_logs_empty_message(self.state.logs.empty_message.clone().into());
        logs_store.set_summary_total_text(self.state.logs.summary_total_text.clone().into());
        logs_store.set_summary_error_text(self.state.logs.summary_error_text.clone().into());
        logs_store.set_summary_sync_text(self.state.logs.summary_sync_text.clone().into());
        logs_store.set_summary_sync_tone(self.state.logs.summary_sync_tone.clone().into());
        logs_store.set_active_log_filter(self.state.logs.filter.as_index());
        logs_store.set_logs_search_text(self.state.logs.query.clone().into());
    }

    fn render_db(&self) {
        let db_store = self.window.global::<DbStore>();
        let tables = self
            .state
            .db
            .tables
            .iter()
            .map(|row| TableRowData {
                name: row.name.clone().into(),
                kind: row.kind.clone().into(),
                selected: row.selected,
            })
            .collect::<Vec<_>>();
        db_store.set_db_tables(ModelRc::new(VecModel::from(tables)));

        let schema_columns = self
            .state
            .db
            .browse
            .schema_columns
            .iter()
            .map(|column| DbSchemaColumnData {
                name: column.name.clone().into(),
                type_label: column.type_label.clone().into(),
                flags_text: column.flags_text.clone().into(),
                hidden: column.hidden,
            })
            .collect::<Vec<_>>();
        db_store.set_db_schema_columns(ModelRc::new(VecModel::from(schema_columns)));

        let headers = self
            .state
            .db
            .browse
            .result_columns
            .iter()
            .map(|column| DbColumnHeaderData {
                name: column.name.clone().into(),
                type_label: column.type_label.clone().into(),
            })
            .collect::<Vec<_>>();
        db_store.set_db_column_headers(ModelRc::new(VecModel::from(headers)));

        let result_rows = self
            .state
            .db
            .browse
            .result_rows
            .iter()
            .map(|row| DbGridRowData {
                key: row.key.clone().into(),
                col1: row.cells.first().cloned().unwrap_or_default().into(),
                col2: row.cells.get(1).cloned().unwrap_or_default().into(),
                col3: row.cells.get(2).cloned().unwrap_or_default().into(),
                col4: row.cells.get(3).cloned().unwrap_or_default().into(),
                preview: row.preview.clone().into(),
                selected: row.selected,
            })
            .collect::<Vec<_>>();
        db_store.set_db_result_rows(ModelRc::new(VecModel::from(result_rows)));

        let filter_columns = self
            .state
            .db
            .browse
            .filter_columns
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>();
        db_store.set_db_filter_columns(ModelRc::new(VecModel::from(filter_columns)));

        let sort_columns = self
            .state
            .db
            .browse
            .sort_columns
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>();
        db_store.set_db_sort_columns(ModelRc::new(VecModel::from(sort_columns)));

        let filter_ops = [
            "contains",
            "equals",
            "not_equals",
            "starts_with",
            "ends_with",
            "eq",
            "neq",
            "gt",
            "gte",
            "lt",
            "lte",
            "is_null",
            "is_not_null",
        ]
        .into_iter()
        .map(SharedString::from)
        .collect::<Vec<_>>();
        db_store.set_db_filter_operator_options(ModelRc::new(VecModel::from(filter_ops)));

        let sort_dirs = ["desc", "asc"]
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>();
        db_store.set_db_sort_direction_options(ModelRc::new(VecModel::from(sort_dirs)));

        db_store.set_db_summary(self.state.db.browse.summary.clone().into());
        db_store.set_db_error(self.state.db.browse.error.clone().into());
        db_store.set_db_row_detail(self.state.db.browse.row_detail.clone().into());
        db_store.set_db_filter_column(self.state.db.filter.column.clone().into());
        db_store.set_db_filter_value(self.state.db.filter.value.clone().into());
        db_store.set_db_filter_operator(self.state.db.filter.operator.clone().into());
        db_store.set_db_sort_column(self.state.db.sort.column.clone().into());
        db_store.set_db_sort_direction(self.state.db.sort.direction.as_str().into());
        db_store.set_db_page_label(self.state.db.browse.page_label.clone().into());
        db_store.set_db_query_sql(self.state.db.query.sql.clone().into());
        db_store.set_db_query_summary(self.state.db.query.summary.clone().into());
        db_store.set_db_query_preview(self.state.db.query.preview.clone().into());
        db_store.set_db_query_error(self.state.db.query.error.clone().into());
        db_store.set_db_active_tab(self.state.db.active_tab.as_index());
    }

    fn persist_preferences(&mut self) {
        self.state.prefs.route = self.state.route;
        self.state.prefs.selected_session_id = self.state.sessions.selected_id;
        self.state.prefs.selected_log_id = self.state.logs.expanded_id.clone();
        self.state.prefs.selected_table_name = self.state.db.selected_table_name.clone();
        self.state.prefs.db_query_sql = self.state.db.query.sql.clone();
        let _ = save_preferences(&self.prefs_path, &self.state.prefs);
    }
}

#[derive(Clone, Copy, Debug)]
enum PaneWidth {
    Sessions,
    DbTables,
    DbRows,
}

#[derive(Clone, Copy, Debug)]
enum PageDelta {
    Prev,
    Next,
}

fn bind_callbacks(controller: &Rc<RefCell<DesktopController>>) {
    let window = controller.borrow().window.clone_strong();
    let shell_store = window.global::<ShellStore>();
    let sessions_store = window.global::<SessionsStore>();
    let logs_store = window.global::<LogsStore>();
    let db_store = window.global::<DbStore>();

    {
        let controller = Rc::downgrade(controller);
        shell_store.on_view_selected(move |index| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .switch_route(AppRoute::from_index(index));
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        shell_store.on_sync_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().sync_or_reload();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        shell_store.on_settings_open_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().open_settings();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        shell_store.on_settings_close_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().close_settings();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        shell_store.on_toggle_sources_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().toggle_sources_panel();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        window.on_global_search_edited(move |text| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .update_global_search(text.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        sessions_store.on_session_lane_selected(move |index| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_session_lane(SessionLane::from_index(index));
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        sessions_store.on_session_selected(move |session_id| {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().select_session(session_id.into());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        logs_store.on_logs_filter_selected(move |index| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_log_filter(LogFilter::from_index(index));
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        logs_store.on_logs_search_edited(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().update_logs_search(value.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        logs_store.on_log_toggled(move |id| {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().toggle_log_expanded(id.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_table_selected(move |index| {
            if let Some(controller) = controller.upgrade() {
                let selected = controller
                    .borrow()
                    .state
                    .db
                    .tables
                    .get(index as usize)
                    .map(|table| table.name.clone());
                if let Some(table_name) = selected {
                    controller.borrow_mut().select_db_table(table_name);
                }
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_result_row_selected(move |key| {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().select_db_row(key.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_filter_column_selected(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_db_filter_column(value.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_filter_operator_selected(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_db_filter_operator(value.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_filter_value_edited(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_db_filter_value(value.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_sort_column_selected(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_db_sort_column(value.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_sort_direction_selected(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .set_db_sort_direction(SortDirection::from_value(&value));
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_query_sql_edited(move |value| {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().update_db_query(value.to_string());
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_apply_filter_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().apply_db_browse();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_clear_filter_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().clear_db_filter();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_prev_page_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().change_db_page(PageDelta::Prev);
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_next_page_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().change_db_page(PageDelta::Next);
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_run_query_requested(move || {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().run_db_query();
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_db_tab_selected(move |index| {
            if let Some(controller) = controller.upgrade() {
                controller.borrow_mut().switch_db_tab(DbTab::from_index(index));
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        sessions_store.on_adjust_sessions_width(move |delta| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .adjust_split(PaneWidth::Sessions, delta);
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_adjust_db_tables_width(move |delta| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .adjust_split(PaneWidth::DbTables, delta);
            }
        });
    }

    {
        let controller = Rc::downgrade(controller);
        db_store.on_adjust_db_rows_width(move |delta| {
            if let Some(controller) = controller.upgrade() {
                controller
                    .borrow_mut()
                    .adjust_split(PaneWidth::DbRows, delta);
            }
        });
    }
}

fn reconcile_session_selection(rows: &mut [SessionListRowVm], current: Option<i64>) -> Option<i64> {
    let selected_id = current
        .filter(|candidate| rows.iter().any(|row| row.id == *candidate))
        .or_else(|| rows.first().map(|row| row.id));
    mark_selected_session_rows(rows, selected_id);
    selected_id
}

fn mark_selected_session_rows(rows: &mut [SessionListRowVm], selected_id: Option<i64>) {
    for row in rows {
        row.selected = Some(row.id) == selected_id;
    }
}

fn reconcile_db_row_selection(
    rows: &mut [DbGridRowVm],
    selected_row_key: &mut Option<String>,
    row_detail: &mut String,
) {
    *selected_row_key = selected_row_key
        .as_ref()
        .filter(|candidate| rows.iter().any(|row| row.key == **candidate))
        .cloned()
        .or_else(|| rows.first().map(|row| row.key.clone()));

    for row in rows.iter_mut() {
        row.selected = selected_row_key.as_deref() == Some(row.key.as_str());
    }

    *row_detail = rows
        .iter()
        .find(|row| selected_row_key.as_deref() == Some(row.key.as_str()))
        .map(|row| row.detail.clone())
        .unwrap_or_default();
}

fn choose_valid_value(current: String, options: &[String]) -> String {
    if options.is_empty() {
        return String::new();
    }

    if options.iter().any(|option| option == &current) {
        current
    } else {
        options[0].clone()
    }
}

fn default_db_query() -> DbQueryVm {
    DbQueryVm {
        sql: "SELECT id, title, updated_at\nFROM sessions\nORDER BY updated_at DESC\nLIMIT 25;"
            .to_string(),
        ..DbQueryVm::default()
    }
}

fn empty_session_detail(title: &str, message: &str) -> SessionDetailVm {
    SessionDetailVm {
        empty_title: title.to_string(),
        empty_message: message.to_string(),
        ..SessionDetailVm::default()
    }
}

fn error_session_detail(error: &anyhow::Error) -> SessionDetailVm {
    SessionDetailVm {
        empty_title: "Session detail unavailable".to_string(),
        empty_message: error.to_string(),
        ..SessionDetailVm::default()
    }
}

fn summarize_source_checks(row: &crate::view_models::SourceRowVm) -> String {
    if row.checks.is_empty() {
        return "No checks available".to_string();
    }

    row.checks
        .iter()
        .map(|check| format!("{}: {}", check.label, check.state_text))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn settings_section_rows(snapshot: &AppSnapshotVm, title: &str) -> Vec<crate::view_models::KeyValueRowVm> {
    snapshot
        .settings
        .sections
        .iter()
        .find(|section| section.title == title)
        .map(|section| section.rows.clone())
        .unwrap_or_default()
}

fn to_key_value_data(row: crate::view_models::KeyValueRowVm) -> KeyValueRowData {
    KeyValueRowData {
        key: row.key.into(),
        value: row.value.into(),
    }
}

fn load_preferences(path: &std::path::Path) -> Result<DesktopPreferences> {
    if !path.exists() {
        return Ok(DesktopPreferences::default());
    }

    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let prefs = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(prefs)
}

fn save_preferences(path: &std::path::Path, prefs: &DesktopPreferences) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(prefs)?;
    std::fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{reconcile_db_row_selection, reconcile_session_selection};
    use crate::view_models::{DbGridRowVm, SessionListRowVm};

    #[test]
    fn session_selection_falls_back_to_first_visible_row() {
        let mut rows = vec![
            SessionListRowVm {
                id: 7,
                ..SessionListRowVm::default()
            },
            SessionListRowVm {
                id: 8,
                ..SessionListRowVm::default()
            },
        ];

        let selected = reconcile_session_selection(&mut rows, Some(42));

        assert_eq!(selected, Some(7));
        assert!(rows[0].selected);
        assert!(!rows[1].selected);
    }

    #[test]
    fn db_row_selection_clears_stale_detail_when_rows_disappear() {
        let mut rows = Vec::<DbGridRowVm>::new();
        let mut selected_row_key = Some("row-4".to_string());
        let mut row_detail = "stale".to_string();

        reconcile_db_row_selection(&mut rows, &mut selected_row_key, &mut row_detail);

        assert_eq!(selected_row_key, None);
        assert!(row_detail.is_empty());
    }
}
