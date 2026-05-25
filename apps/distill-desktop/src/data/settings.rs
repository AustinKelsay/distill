use crate::config::SourceMode;
use crate::view_models::{KeyValueRowVm, SettingsSectionVm, SettingsSnapshotVm, SourceRowVm};

use super::DesktopDataSource;

impl DesktopDataSource {
    pub(super) fn build_settings_snapshot(&self, source_rows: &[SourceRowVm]) -> SettingsSnapshotVm {
        let mut sections = Vec::new();

        let storage_rows = vec![
            KeyValueRowVm {
                key: "Home".to_string(),
                value: self.runtime.app_paths.app_home.display().to_string(),
            },
            KeyValueRowVm {
                key: "Database".to_string(),
                value: self.runtime.app_paths.db_path.display().to_string(),
            },
            KeyValueRowVm {
                key: "Blobs".to_string(),
                value: self.runtime.app_paths.blobs_dir.display().to_string(),
            },
            KeyValueRowVm {
                key: "Preferences".to_string(),
                value: self.runtime.app_paths.prefs_path.display().to_string(),
            },
        ];
        sections.push(SettingsSectionVm {
            title: "Storage".to_string(),
            rows: storage_rows,
        });

        let mut source_section_rows = vec![KeyValueRowVm {
            key: "Mode".to_string(),
            value: self.runtime.source_mode.label().to_string(),
        }];
        if let Some(electron_home) = self.runtime.electron_home.as_ref() {
            source_section_rows.push(KeyValueRowVm {
                key: "Electron Home".to_string(),
                value: electron_home.display().to_string(),
            });
        }
        for row in source_rows {
            source_section_rows.push(KeyValueRowVm {
                key: row.display_name.clone(),
                value: format!(
                    "{} · {}",
                    row.status_text,
                    if row.data_root.is_empty() {
                        "not found".to_string()
                    } else {
                        row.data_root.clone()
                    }
                ),
            });
        }
        sections.push(SettingsSectionVm {
            title: "Sources".to_string(),
            rows: source_section_rows,
        });

        let sync_rows = vec![
            KeyValueRowVm {
                key: "Sync Button".to_string(),
                value: match self.runtime.source_mode {
                    SourceMode::RustOwned => "Enabled".to_string(),
                    SourceMode::ElectronCompatReadOnly => "Disabled in read-only mode".to_string(),
                },
            },
            KeyValueRowVm {
                key: "Background Sync".to_string(),
                value: "Not wired in Rust yet".to_string(),
            },
        ];
        sections.push(SettingsSectionVm {
            title: "Sync".to_string(),
            rows: sync_rows,
        });

        sections.push(SettingsSectionVm {
            title: "Curation".to_string(),
            rows: vec![KeyValueRowVm {
                key: "State".to_string(),
                value: "Read-only UI stubs only".to_string(),
            }],
        });

        SettingsSnapshotVm {
            sections,
            labels: vec![
                "train".to_string(),
                "holdout".to_string(),
                "exclude".to_string(),
                "sensitive".to_string(),
                "favorite".to_string(),
            ],
        }
    }
}
