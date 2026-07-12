//! Independent per-Source detection through the production adapter seam.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::adapter::{
    find_executable, ClaudeAdapter, CodexAdapter, FixtureAdapter, OpenCodeAdapter, SourceAdapter,
    SourceKind,
};
use crate::error::LibraryResult;
use crate::ops::paths::canonicalize_configured_root;
use crate::ops::prefs::list_source_preferences;
use crate::types::{SourceDetectRequest, SourceDetectResult};

/**
 * Detect each requested Source independently.
 *
 * A failing Source never aborts sibling results. Fixture, Codex, Claude Code,
 * and OpenCode use concrete adapters; Droid returns typed `unavailable` until #29.
 */
pub fn detect_sources(
    conn: &Connection,
    requests: &[SourceDetectRequest],
    fixture_parser_version: &str,
) -> LibraryResult<Vec<SourceDetectResult>> {
    let prefs = list_source_preferences(conn)?;
    let mut results = Vec::with_capacity(requests.len());
    for request in requests {
        results.push(detect_one(conn, request, &prefs, fixture_parser_version));
    }
    Ok(results)
}

fn detect_one(
    _conn: &Connection,
    request: &SourceDetectRequest,
    prefs: &[crate::types::SourcePreference],
    fixture_parser_version: &str,
) -> SourceDetectResult {
    let Some(kind) = SourceKind::parse(&request.kind) else {
        return SourceDetectResult {
            kind: request.kind.clone(),
            status: "unhealthy".into(),
            executable: None,
            effective_data_root: None,
            display_name: None,
            error_class: Some("unknown_source_kind".into()),
            error_message: Some("unknown source kind".into()),
        };
    };

    let pref = prefs.iter().find(|pref| pref.kind == kind.as_str());

    match kind {
        SourceKind::Fixture => {
            if pref.is_some_and(|pref| !pref.enabled) && request.configured_root.is_none() {
                return SourceDetectResult {
                    kind: kind.as_str().into(),
                    status: "disabled".into(),
                    executable: None,
                    effective_data_root: pref.and_then(|p| p.configured_root.clone()),
                    display_name: pref.and_then(|p| p.display_name.clone()),
                    error_class: None,
                    error_message: None,
                };
            }
            detect_fixture(request, pref, fixture_parser_version)
        }
        SourceKind::Codex => {
            if pref.is_some_and(|pref| !pref.enabled) && request.configured_root.is_none() {
                return SourceDetectResult {
                    kind: kind.as_str().into(),
                    status: "disabled".into(),
                    executable: find_executable("codex").map(|path| path.display().to_string()),
                    effective_data_root: pref.and_then(|p| p.configured_root.clone()),
                    display_name: Some("Codex".into()),
                    error_class: None,
                    error_message: None,
                };
            }
            detect_codex(request, pref)
        }
        SourceKind::ClaudeCode => {
            if pref.is_some_and(|pref| !pref.enabled) && request.configured_root.is_none() {
                return SourceDetectResult {
                    kind: kind.as_str().into(),
                    status: "disabled".into(),
                    executable: find_executable("claude").map(|path| path.display().to_string()),
                    effective_data_root: pref.and_then(|p| p.configured_root.clone()),
                    display_name: Some("Claude Code".into()),
                    error_class: None,
                    error_message: None,
                };
            }
            detect_claude(request, pref)
        }
        SourceKind::OpenCode => {
            if pref.is_some_and(|pref| !pref.enabled) && request.configured_root.is_none() {
                return SourceDetectResult {
                    kind: kind.as_str().into(),
                    status: "disabled".into(),
                    executable: find_executable("opencode").map(|path| path.display().to_string()),
                    effective_data_root: pref.and_then(|p| p.configured_root.clone()),
                    display_name: Some("OpenCode".into()),
                    error_class: None,
                    error_message: None,
                };
            }
            detect_opencode(request, pref)
        }
        SourceKind::Droid => {
            let effective_data_root = match resolve_root_text(request, pref) {
                Ok(root) => root,
                Err(err) => {
                    return SourceDetectResult {
                        kind: kind.as_str().into(),
                        status: "unhealthy".into(),
                        executable: None,
                        effective_data_root: None,
                        display_name: Some(kind.as_str().into()),
                        error_class: Some(err.code().into()),
                        error_message: Some(stable_detect_message(err.code())),
                    };
                }
            };
            SourceDetectResult {
                kind: kind.as_str().into(),
                status: "unavailable".into(),
                executable: None,
                effective_data_root,
                display_name: Some(kind.as_str().into()),
                error_class: Some("adapter_not_registered".into()),
                error_message: Some("source adapter is not registered in this build".into()),
            }
        }
    }
}

fn detect_fixture(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
    fixture_parser_version: &str,
) -> SourceDetectResult {
    let root = match resolve_root_path(request, pref) {
        Ok(Some(root)) => root,
        Ok(None) => {
            return SourceDetectResult {
                kind: SourceKind::Fixture.as_str().into(),
                status: "missing".into(),
                executable: None,
                effective_data_root: None,
                display_name: Some("Fixture".into()),
                error_class: Some("configured_root_required".into()),
                error_message: Some(stable_detect_message("configured_root_required")),
            };
        }
        Err(err) => {
            return SourceDetectResult {
                kind: SourceKind::Fixture.as_str().into(),
                status: "unhealthy".into(),
                executable: None,
                effective_data_root: None,
                display_name: Some("Fixture".into()),
                error_class: Some(err.code().into()),
                error_message: Some(stable_detect_message(err.code())),
            };
        }
    };

    let adapter = FixtureAdapter::with_parser(
        root.clone(),
        crate::adapter::ParserIdentity {
            id: crate::adapter::FIXTURE_PARSER_ID.to_string(),
            version: fixture_parser_version.to_string(),
        },
    );
    match adapter.detect() {
        Ok(discovered) => SourceDetectResult {
            kind: SourceKind::Fixture.as_str().into(),
            status: "ok".into(),
            executable: None,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: None,
            error_message: None,
        },
        Err(err) => SourceDetectResult {
            kind: SourceKind::Fixture.as_str().into(),
            status: "unhealthy".into(),
            executable: None,
            effective_data_root: None,
            display_name: Some("Fixture".into()),
            error_class: Some(err.code().into()),
            error_message: Some(stable_detect_message(err.code())),
        },
    }
}

fn detect_codex(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
) -> SourceDetectResult {
    detect_codex_with_executable(
        request,
        pref,
        find_executable("codex").map(|path| path.display().to_string()),
    )
}

fn detect_claude(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
) -> SourceDetectResult {
    detect_claude_with_executable(
        request,
        pref,
        find_executable("claude").map(|path| path.display().to_string()),
    )
}

fn detect_opencode(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
) -> SourceDetectResult {
    detect_opencode_with_executable(
        request,
        pref,
        find_executable("opencode").map(|path| path.display().to_string()),
    )
}

fn detect_codex_with_executable(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
    executable: Option<String>,
) -> SourceDetectResult {
    let root = match resolve_root_path(request, pref) {
        Ok(Some(root)) => root,
        Ok(None) => {
            return SourceDetectResult {
                kind: SourceKind::Codex.as_str().into(),
                status: "missing".into(),
                executable,
                effective_data_root: None,
                display_name: Some("Codex".into()),
                error_class: Some("configured_root_required".into()),
                error_message: Some(stable_detect_message("configured_root_required")),
            };
        }
        Err(err) => {
            return SourceDetectResult {
                kind: SourceKind::Codex.as_str().into(),
                status: "unhealthy".into(),
                executable,
                effective_data_root: None,
                display_name: Some("Codex".into()),
                error_class: Some(err.code().into()),
                error_message: Some(stable_detect_message(err.code())),
            };
        }
    };

    let adapter = CodexAdapter::new(root);
    match adapter.detect() {
        Ok(discovered) if executable.is_some() => SourceDetectResult {
            kind: SourceKind::Codex.as_str().into(),
            status: "ok".into(),
            executable,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: None,
            error_message: None,
        },
        Ok(discovered) => SourceDetectResult {
            kind: SourceKind::Codex.as_str().into(),
            status: "unavailable".into(),
            executable,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: Some("executable_not_found".into()),
            error_message: Some(stable_detect_message("executable_not_found")),
        },
        Err(err) => SourceDetectResult {
            kind: SourceKind::Codex.as_str().into(),
            status: "unhealthy".into(),
            executable,
            effective_data_root: None,
            display_name: Some("Codex".into()),
            error_class: Some(err.code().into()),
            error_message: Some(stable_detect_message(err.code())),
        },
    }
}

fn detect_claude_with_executable(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
    executable: Option<String>,
) -> SourceDetectResult {
    let root = match resolve_root_path(request, pref) {
        Ok(Some(root)) => root,
        Ok(None) => {
            return SourceDetectResult {
                kind: SourceKind::ClaudeCode.as_str().into(),
                status: "missing".into(),
                executable,
                effective_data_root: None,
                display_name: Some("Claude Code".into()),
                error_class: Some("configured_root_required".into()),
                error_message: Some(stable_detect_message("configured_root_required")),
            };
        }
        Err(err) => {
            return SourceDetectResult {
                kind: SourceKind::ClaudeCode.as_str().into(),
                status: "unhealthy".into(),
                executable,
                effective_data_root: None,
                display_name: Some("Claude Code".into()),
                error_class: Some(err.code().into()),
                error_message: Some(stable_detect_message(err.code())),
            };
        }
    };

    let adapter = ClaudeAdapter::new(root);
    match adapter.detect() {
        Ok(discovered) if executable.is_some() => SourceDetectResult {
            kind: SourceKind::ClaudeCode.as_str().into(),
            status: "ok".into(),
            executable,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: None,
            error_message: None,
        },
        Ok(discovered) => SourceDetectResult {
            kind: SourceKind::ClaudeCode.as_str().into(),
            status: "unavailable".into(),
            executable,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: Some("executable_not_found".into()),
            error_message: Some(stable_detect_message("executable_not_found")),
        },
        Err(err) => SourceDetectResult {
            kind: SourceKind::ClaudeCode.as_str().into(),
            status: "unhealthy".into(),
            executable,
            effective_data_root: None,
            display_name: Some("Claude Code".into()),
            error_class: Some(err.code().into()),
            error_message: Some(stable_detect_message(err.code())),
        },
    }
}

fn detect_opencode_with_executable(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
    executable: Option<String>,
) -> SourceDetectResult {
    let root = match resolve_root_path(request, pref) {
        Ok(Some(root)) => root,
        Ok(None) => {
            return SourceDetectResult {
                kind: SourceKind::OpenCode.as_str().into(),
                status: "missing".into(),
                executable,
                effective_data_root: None,
                display_name: Some("OpenCode".into()),
                error_class: Some("configured_root_required".into()),
                error_message: Some(stable_detect_message("configured_root_required")),
            };
        }
        Err(err) => {
            return SourceDetectResult {
                kind: SourceKind::OpenCode.as_str().into(),
                status: "unhealthy".into(),
                executable,
                effective_data_root: None,
                display_name: Some("OpenCode".into()),
                error_class: Some(err.code().into()),
                error_message: Some(stable_detect_message(err.code())),
            };
        }
    };

    // Prefer a root-local harness binary when present so detect matches discover/snapshot.
    let executable = if root.join("bin").join("opencode").is_file() {
        Some(root.join("bin").join("opencode").display().to_string())
    } else {
        executable
    };

    let adapter = OpenCodeAdapter::new(root);
    match adapter.detect() {
        Ok(discovered) if executable.is_some() => SourceDetectResult {
            kind: SourceKind::OpenCode.as_str().into(),
            status: "ok".into(),
            executable,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: None,
            error_message: None,
        },
        Ok(discovered) => SourceDetectResult {
            kind: SourceKind::OpenCode.as_str().into(),
            status: "unavailable".into(),
            executable,
            effective_data_root: Some(discovered.data_root.display().to_string()),
            display_name: Some(discovered.display_name),
            error_class: Some("executable_not_found".into()),
            error_message: Some(stable_detect_message("executable_not_found")),
        },
        Err(err) => SourceDetectResult {
            kind: SourceKind::OpenCode.as_str().into(),
            status: "unhealthy".into(),
            executable,
            effective_data_root: None,
            display_name: Some("OpenCode".into()),
            error_class: Some(err.code().into()),
            error_message: Some(stable_detect_message(err.code())),
        },
    }
}

fn resolve_root_path(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
) -> LibraryResult<Option<PathBuf>> {
    if let Some(root) = request.configured_root.as_ref() {
        return Ok(Some(canonicalize_configured_root(Path::new(root))?));
    }
    if let Some(root) = pref.and_then(|pref| pref.configured_root.as_ref()) {
        return Ok(Some(canonicalize_configured_root(Path::new(root))?));
    }
    Ok(None)
}

fn resolve_root_text(
    request: &SourceDetectRequest,
    pref: Option<&crate::types::SourcePreference>,
) -> LibraryResult<Option<String>> {
    Ok(resolve_root_path(request, pref)?.map(|path| path.to_string_lossy().into_owned()))
}

/**
 * Stable generic detection diagnostic. Never includes paths or provider payloads.
 */
fn stable_detect_message(error_class: &str) -> String {
    match error_class {
        "invalid_configured_root" => "configured root is invalid".into(),
        "source_adapter" => "source detection failed".into(),
        "configured_root_required" => "source detection requires a configured root".into(),
        "unknown_source_kind" => "unknown source kind".into(),
        "adapter_not_registered" => "source adapter is not registered in this build".into(),
        "executable_not_found" => "source executable is unavailable".into(),
        _ => "source detection failed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn codex_detection_classifies_missing_executable_without_provider_text() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("codex-home");
        std::fs::create_dir_all(&root).expect("root");
        let result = detect_codex_with_executable(
            &SourceDetectRequest {
                kind: "codex".into(),
                configured_root: Some(root.display().to_string()),
            },
            None,
            None,
        );
        assert_eq!(result.status, "unavailable");
        assert_eq!(result.error_class.as_deref(), Some("executable_not_found"));
        let message = result.error_message.as_deref().unwrap_or_default();
        assert!(!message.contains("codex"));
        assert!(!message.contains('/'));
    }

    #[test]
    fn claude_detection_classifies_missing_executable_without_provider_text() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("claude-home");
        std::fs::create_dir_all(&root).expect("root");
        let result = detect_claude_with_executable(
            &SourceDetectRequest {
                kind: "claude_code".into(),
                configured_root: Some(root.display().to_string()),
            },
            None,
            None,
        );
        assert_eq!(result.status, "unavailable");
        assert_eq!(result.error_class.as_deref(), Some("executable_not_found"));
        let message = result.error_message.as_deref().unwrap_or_default();
        assert!(!message.to_ascii_lowercase().contains("claude"));
        assert!(!message.contains('/'));
    }

    #[test]
    fn opencode_detection_classifies_missing_executable_without_provider_text() {
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("opencode-home");
        std::fs::create_dir_all(&root).expect("root");
        let result = detect_opencode_with_executable(
            &SourceDetectRequest {
                kind: "opencode".into(),
                configured_root: Some(root.display().to_string()),
            },
            None,
            None,
        );
        assert_eq!(result.status, "unavailable");
        assert_eq!(result.error_class.as_deref(), Some("executable_not_found"));
        let message = result.error_message.as_deref().unwrap_or_default();
        assert!(!message.to_ascii_lowercase().contains("opencode"));
        assert!(!message.contains('/'));
    }
}
