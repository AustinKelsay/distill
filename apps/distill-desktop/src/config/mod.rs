use std::path::PathBuf;

use anyhow::{Result, bail};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPaths {
    pub app_home: PathBuf,
    pub db_path: PathBuf,
    pub blobs_dir: PathBuf,
    pub prefs_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceMode {
    #[default]
    RustOwned,
    ElectronCompatReadOnly,
}

impl SourceMode {
    pub fn parse_env(value: &str) -> Result<Self> {
        match value {
            "rust" => Ok(Self::RustOwned),
            "electron_compat" => Ok(Self::ElectronCompatReadOnly),
            other => bail!(
                "invalid DISTILL_SOURCE_MODE {other:?}; expected \"rust\" or \"electron_compat\""
            ),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RustOwned => "Rust-Owned / Writable",
            Self::ElectronCompatReadOnly => "Electron Compatibility / Read Only",
        }
    }

    pub fn badge_text(self) -> &'static str {
        match self {
            Self::RustOwned => "Rust Native Store",
            Self::ElectronCompatReadOnly => "Desktop Native Shell",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DesktopRuntimeConfig {
    pub app_paths: AppPaths,
    pub source_mode: SourceMode,
    pub electron_home: Option<PathBuf>,
}

pub fn resolve_runtime_config() -> Result<DesktopRuntimeConfig> {
    let data_local_dir = dirs::data_local_dir().or_else(dirs::data_dir);
    let home_dir = dirs::home_dir();

    resolve_runtime_config_from(
        std::env::var_os("DISTILL_DESKTOP_HOME").map(PathBuf::from),
        std::env::var("DISTILL_SOURCE_MODE").ok(),
        std::env::var_os("DISTILL_ELECTRON_HOME").map(PathBuf::from),
        data_local_dir,
        home_dir,
    )
}

pub fn resolve_runtime_config_from(
    desktop_home_override: Option<PathBuf>,
    source_mode_env: Option<String>,
    electron_home_override: Option<PathBuf>,
    data_local_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> Result<DesktopRuntimeConfig> {
    if desktop_home_override.is_none() && data_local_dir.is_none() {
        bail!("no app data directory is available");
    }

    let app_home = desktop_home_override.unwrap_or_else(|| {
        data_local_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("distill-desktop")
    });

    let source_mode = match source_mode_env {
        Some(value) => SourceMode::parse_env(value.trim())?,
        None => SourceMode::RustOwned,
    };

    let has_explicit_electron_home = electron_home_override.is_some();
    let electron_home = match source_mode {
        SourceMode::RustOwned => None,
        SourceMode::ElectronCompatReadOnly => Some(electron_home_override.unwrap_or_else(|| {
            home_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(".distill-electron")
        })),
    };

    if matches!(source_mode, SourceMode::ElectronCompatReadOnly)
        && home_dir.is_none()
        && !has_explicit_electron_home
    {
        bail!("home directory is unavailable");
    }

    Ok(DesktopRuntimeConfig {
        app_paths: AppPaths {
            db_path: app_home.join("distill.db"),
            blobs_dir: app_home.join("blobs"),
            prefs_path: app_home.join("preferences.json"),
            app_home,
        },
        source_mode,
        electron_home,
    })
}

#[cfg(test)]
mod tests {
    use super::{SourceMode, resolve_runtime_config_from};
    use std::path::PathBuf;

    #[test]
    fn defaults_to_rust_owned_home() {
        let runtime = resolve_runtime_config_from(
            None,
            None,
            None,
            Some(PathBuf::from("/tmp/data")),
            Some(PathBuf::from("/tmp/home")),
        )
        .unwrap();
        assert_eq!(runtime.source_mode, SourceMode::RustOwned);
        assert_eq!(
            runtime.app_paths.app_home,
            PathBuf::from("/tmp/data/distill-desktop")
        );
        assert_eq!(
            runtime.app_paths.db_path,
            PathBuf::from("/tmp/data/distill-desktop/distill.db")
        );
    }

    #[test]
    fn supports_electron_compat_resolution() {
        let runtime = resolve_runtime_config_from(
            None,
            Some("electron_compat".to_string()),
            None,
            Some(PathBuf::from("/tmp/data")),
            Some(PathBuf::from("/tmp/home")),
        )
        .unwrap();
        assert_eq!(runtime.source_mode, SourceMode::ElectronCompatReadOnly);
        assert_eq!(
            runtime.electron_home,
            Some(PathBuf::from("/tmp/home/.distill-electron"))
        );
    }

    #[test]
    fn desktop_home_override_resolves_when_data_local_dir_none() {
        let runtime = resolve_runtime_config_from(
            Some(PathBuf::from("./distill-desktop")),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            runtime.app_paths.app_home,
            PathBuf::from("./distill-desktop")
        );
    }

    #[test]
    fn rejects_invalid_source_mode() {
        let error = resolve_runtime_config_from(
            None,
            Some("bogus".to_string()),
            None,
            Some(PathBuf::from("/tmp/data")),
            Some(PathBuf::from("/tmp/home")),
        )
        .unwrap_err();
        assert!(error.to_string().contains("DISTILL_SOURCE_MODE"));
    }
}
