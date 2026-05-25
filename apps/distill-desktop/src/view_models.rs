use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AppRoute {
    #[default]
    Sessions,
    Db,
    Logs,
}

impl AppRoute {
    pub fn as_index(self) -> i32 {
        match self {
            Self::Sessions => 0,
            Self::Db => 1,
            Self::Logs => 2,
        }
    }

    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Db,
            2 => Self::Logs,
            _ => Self::Sessions,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionLane {
    #[default]
    All,
    NeedsReview,
    TrainReady,
    HoldoutReady,
    Favorite,
}

impl SessionLane {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::NeedsReview,
        Self::TrainReady,
        Self::HoldoutReady,
        Self::Favorite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsReview => "Needs Review",
            Self::TrainReady => "Train Ready",
            Self::HoldoutReady => "Holdout Ready",
            Self::Favorite => "Favorites",
        }
    }

    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::NeedsReview,
            2 => Self::TrainReady,
            3 => Self::HoldoutReady,
            4 => Self::Favorite,
            _ => Self::All,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionWorkflowState {
    NeedsReview,
    TrainReady,
    HoldoutReady,
    Favorite,
    #[default]
    Neutral,
}

impl SessionWorkflowState {
    pub fn label(self) -> &'static str {
        match self {
            Self::NeedsReview => "review",
            Self::TrainReady => "train",
            Self::HoldoutReady => "holdout",
            Self::Favorite => "favorite",
            Self::Neutral => "",
        }
    }

    pub fn tone(self) -> &'static str {
        match self {
            Self::NeedsReview => "warning",
            Self::TrainReady => "ok",
            Self::HoldoutReady => "info",
            Self::Favorite => "favorite",
            Self::Neutral => "neutral",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LogFilter {
    #[default]
    All,
    Sync,
    Export,
    Errors,
}

impl LogFilter {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Sync,
            2 => Self::Export,
            3 => Self::Errors,
            _ => Self::All,
        }
    }

    pub fn as_index(self) -> i32 {
        match self {
            Self::All => 0,
            Self::Sync => 1,
            Self::Export => 2,
            Self::Errors => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DbTab {
    #[default]
    Browse,
    Query,
}

impl DbTab {
    pub fn as_index(self) -> i32 {
        match self {
            Self::Browse => 0,
            Self::Query => 1,
        }
    }

    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Query,
            _ => Self::Browse,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ShellStatVm {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct SourceCheckVm {
    pub label: String,
    pub state_text: String,
    pub exists: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SourceRowVm {
    pub kind: String,
    pub display_name: String,
    pub status_text: String,
    pub status_tone: String,
    pub data_root: String,
    pub checks: Vec<SourceCheckVm>,
    pub is_stub: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SyncStatusVm {
    pub text: String,
    pub tone: String,
    pub enabled: bool,
    pub button_label: String,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsSectionVm {
    pub title: String,
    pub rows: Vec<KeyValueRowVm>,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsSnapshotVm {
    pub sections: Vec<SettingsSectionVm>,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AppSnapshotVm {
    pub home_path: PathBuf,
    pub database_path: PathBuf,
    pub database_exists: bool,
    pub source_mode_label: String,
    pub source_badge_text: String,
    pub app_status_text: String,
    pub session_count: usize,
    pub message_count: usize,
    pub log_count: usize,
    pub table_count: usize,
    pub shell_stats: Vec<ShellStatVm>,
    pub sync_status: SyncStatusVm,
    pub source_rows: Vec<SourceRowVm>,
    pub settings: SettingsSnapshotVm,
    pub sidebar_count_label: String,
    pub scanned_at_label: String,
    pub show_onboarding: bool,
    pub onboarding_title: String,
    pub onboarding_message: String,
}

#[derive(Clone, Debug, Default)]
pub struct SessionsPageVm {
    pub rows: Vec<SessionListRowVm>,
    pub empty_title: String,
    pub empty_message: String,
}

#[derive(Clone, Debug, Default)]
pub struct SessionBadgeVm {
    pub text: String,
    pub tone: String,
}

#[derive(Clone, Debug, Default)]
pub struct SessionListRowVm {
    pub id: i64,
    pub title: String,
    pub preview: String,
    pub source_badge: Option<SessionBadgeVm>,
    pub workflow_badge: Option<SessionBadgeVm>,
    pub model_badge: Option<SessionBadgeVm>,
    pub message_count_text: String,
    pub updated_at_text: String,
    pub git_branch_text: String,
    pub selected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DetailContextRowVm {
    pub label: String,
    pub value: String,
    pub presentation: String,
}

#[derive(Clone, Debug, Default)]
pub struct TranscriptMessageVm {
    pub role: String,
    pub message_kind: String,
    pub ordinal_text: String,
    pub timestamp_text: String,
    pub body: String,
}

#[derive(Clone, Debug, Default)]
pub struct ArtifactCardVm {
    pub title: String,
    pub meta: String,
    pub preview: String,
    pub payload_json: String,
}

#[derive(Clone, Debug, Default)]
pub struct SessionDetailVm {
    pub id: Option<i64>,
    pub title: String,
    pub summary: String,
    pub secondary_badges: Vec<SessionBadgeVm>,
    pub labels: Vec<String>,
    pub tags: Vec<String>,
    pub context_rows: Vec<DetailContextRowVm>,
    pub provenance_json: String,
    pub messages: Vec<TranscriptMessageVm>,
    pub artifacts: Vec<ArtifactCardVm>,
    pub export_enabled: bool,
    pub curation_enabled: bool,
    pub empty_title: String,
    pub empty_message: String,
}

#[derive(Clone, Debug, Default)]
pub struct KeyValueRowVm {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default)]
pub struct LogsPageVm {
    pub entries: Vec<LogEntryVm>,
    pub summary_total_text: String,
    pub summary_error_text: String,
    pub summary_sync_text: String,
    pub empty_title: String,
    pub empty_message: String,
}

#[derive(Clone, Debug, Default)]
pub struct LogEntryVm {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub summary: String,
    pub status: String,
    pub level: String,
    pub metrics: String,
    pub raw_json: String,
    pub expanded: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DbExplorerVm {
    pub tables: Vec<DbTableVm>,
    pub selected_table_name: Option<String>,
    pub filter_column: String,
    pub filter_operator: String,
    pub sort_column: String,
    pub sort_direction: String,
    pub browse: DbBrowseVm,
    pub query: DbQueryVm,
}

#[derive(Clone, Debug, Default)]
pub struct DbTableVm {
    pub name: String,
    pub kind: String,
    pub selected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DbSchemaColumnVm {
    pub name: String,
    pub type_label: String,
    pub flags_text: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DbColumnHeaderVm {
    pub name: String,
    pub type_label: String,
}

#[derive(Clone, Debug, Default)]
pub struct DbGridRowVm {
    pub key: String,
    pub preview: String,
    pub detail: String,
    pub cells: Vec<String>,
    pub selected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DbBrowseVm {
    pub schema_columns: Vec<DbSchemaColumnVm>,
    pub result_columns: Vec<DbColumnHeaderVm>,
    pub result_rows: Vec<DbGridRowVm>,
    pub summary: String,
    pub error: String,
    pub page_label: String,
    pub filter_columns: Vec<String>,
    pub sort_columns: Vec<String>,
    pub row_detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct DbQueryVm {
    pub sql: String,
    pub summary: String,
    pub preview: String,
    pub error: String,
}
