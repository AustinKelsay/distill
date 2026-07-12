//! Host hostile-input and capability-boundary contracts for issue #32.

use std::fs;
use std::path::{Path, PathBuf};

use distill_desktop_lib::{
    validate_fixture_journey_request, validate_home_request, validate_legacy_import_request,
    validate_source_preference_request, HostError,
};
use distill_library::{safe_caller_message, LibraryError};
use tempfile::TempDir;

/**
 * Capability file remains least-privilege: events only, no FS/shell/SQL plugins.
 */
#[test]
fn capability_file_denies_filesystem_shell_sql_and_process_plugins() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json");
    let raw = fs::read_to_string(&path).expect("read capabilities");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let permissions = value
        .get("permissions")
        .and_then(|item| item.as_array())
        .expect("permissions");
    assert_eq!(permissions.len(), 1);
    assert_eq!(
        permissions[0].as_str(),
        Some("core:event:default"),
        "renderer must not receive filesystem/shell/process/SQL plugin grants"
    );
    let denied = [
        "fs:", "shell:", "process:", "sql:", "dialog:", "http:", "os:",
    ];
    for needle in denied {
        assert!(
            !raw.contains(needle),
            "capability file unexpectedly grants {needle}"
        );
    }
    assert!(raw.contains("no application encryption"));
    assert!(raw.contains("secure-forget"));
}

#[test]
fn host_rejects_parent_traversal_and_nul_without_echoing_raw_paths() {
    let err = validate_home_request("/tmp/distill/../escape").expect_err("parent traversal");
    assert_eq!(err.code, "validation");
    assert!(err.message.contains("parent traversal"));
    assert!(!err.message.contains("escape"));

    let err = validate_home_request(" /tmp/home\0secret ").expect_err("nul");
    assert_eq!(err.code, "validation");
    assert!(err.message.contains("NUL"));
    assert!(!err.message.contains("secret"));

    let err = validate_legacy_import_request("/tmp/home", "/tmp/legacy/../other")
        .expect_err("legacy traversal");
    assert_eq!(err.code, "validation");

    let err = validate_source_preference_request(
        "/tmp/home",
        "fixture",
        true,
        Some("/tmp/fixture/../outside"),
    )
    .expect_err("configured root traversal");
    assert_eq!(err.code, "validation");
}

#[test]
fn host_fixture_validation_omits_missing_path_from_message() {
    let temp = TempDir::new().expect("temp");
    let missing = temp.path().join("missing-fixture-root");
    let err = validate_fixture_journey_request(
        temp.path().join("home").to_str().unwrap(),
        missing.to_str().unwrap(),
    )
    .expect_err("missing fixture");
    assert_eq!(err.code, "validation");
    assert_eq!(err.message, "fixture root is not an existing directory");
    assert!(!err.message.contains("missing-fixture-root"));
}

#[test]
fn host_error_from_library_uses_redacted_safe_message() {
    let err = HostError::from_library(LibraryError::PathOutsideConfiguredRoot {
        path: PathBuf::from("/Users/me/.codex/secret-session.jsonl"),
        root: PathBuf::from("/Users/me/.codex"),
    });
    assert_eq!(err.code, "path_outside_configured_root");
    assert_eq!(
        err.message,
        safe_caller_message(&LibraryError::PathOutsideConfiguredRoot {
            path: PathBuf::from("/Users/me/.codex/secret-session.jsonl"),
            root: PathBuf::from("/Users/me/.codex"),
        })
    );
    assert!(!err.message.contains("secret-session"));
    assert!(!err.message.contains("/Users/me"));
}
