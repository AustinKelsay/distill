//! Operations: Source preferences, detection, Sync Runs, and lease health.

mod detect;
mod paths;
mod prefs;
pub(crate) mod process;
mod sync;
mod sync_execute;
mod sync_lease;

pub use detect::detect_sources;
pub use paths::canonicalize_configured_root;
pub use prefs::{list_source_preferences, upsert_source_preference};
#[cfg(feature = "test-leases")]
pub use process::{
    enforce_output_bounds_for_test, run_bounded_command, BoundedProcessOutput,
    ProviderProcessLimits,
};
pub use sync::{
    active_sync_operations_status, fail_stale_active_runs, load_sync_run, request_cancel,
    start_sync, SYNC_LEASE_STALE_AFTER,
};

#[cfg(feature = "test-leases")]
pub use sync::test_leases;

use std::time::Duration;

/// Production stale threshold seconds used when no test override is armed.
pub const SYNC_LEASE_STALE_AFTER_SECS: u64 = 60;

/// Production background lease heartbeat interval seconds.
const SYNC_HEARTBEAT_INTERVAL_SECS: u64 = 15;

#[cfg(feature = "test-leases")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "test-leases")]
static TEST_LEASE_STALE_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "test-leases")]
static TEST_HEARTBEAT_INTERVAL_MS: AtomicU64 = AtomicU64::new(0);

/**
 * Resolve the active Sync lease stale duration.
 *
 * Production always returns 60 seconds. Under `test-leases`, tests may override.
 */
pub(crate) fn lease_stale_after() -> Duration {
    #[cfg(feature = "test-leases")]
    {
        let ms = TEST_LEASE_STALE_MS.load(Ordering::SeqCst);
        if ms > 0 {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(SYNC_LEASE_STALE_AFTER_SECS)
}

/**
 * Resolve the background lease heartbeat interval.
 *
 * Production uses 15 seconds. Under `test-leases`, tests may override.
 */
pub(crate) fn heartbeat_interval() -> Duration {
    #[cfg(feature = "test-leases")]
    {
        let ms = TEST_HEARTBEAT_INTERVAL_MS.load(Ordering::SeqCst);
        if ms > 0 {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(SYNC_HEARTBEAT_INTERVAL_SECS)
}

/**
 * Generate a durable Sync Run owner id for lease ownership.
 */
pub fn new_owner_id() -> String {
    format!("owner-{}", uuid::Uuid::new_v4())
}

#[cfg(feature = "test-leases")]
pub(crate) fn set_test_lease_stale_ms(ms: u64) {
    TEST_LEASE_STALE_MS.store(ms, Ordering::SeqCst);
}

#[cfg(feature = "test-leases")]
pub(crate) fn set_test_heartbeat_interval_ms(ms: u64) {
    TEST_HEARTBEAT_INTERVAL_MS.store(ms, Ordering::SeqCst);
}

#[cfg(feature = "test-leases")]
pub(crate) fn reset_test_lease_timing() {
    TEST_LEASE_STALE_MS.store(0, Ordering::SeqCst);
    TEST_HEARTBEAT_INTERVAL_MS.store(0, Ordering::SeqCst);
}
