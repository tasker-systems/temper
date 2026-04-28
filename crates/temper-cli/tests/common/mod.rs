//! Shared test helpers for `temper-cli` integration tests.
//!
//! Lives at `tests/common/mod.rs` (rather than `common.rs`) so cargo treats
//! it as a module rather than a stand-alone test binary.

use std::sync::Once;

static AUTH_INIT: Once = Once::new();

/// Point `TEMPER_AUTH_PATH` at a non-existent path so the publish-tail helper
/// finds no token and no-ops without touching `~/.config/temper/auth.json`
/// or making any network calls.
///
/// Idempotent — runs at most once per test process. The value is a constant
/// (not per-test) so parallel set_var calls converge on the same final state
/// even if the OS sees the writes in any order. Tests that need a different
/// auth path can override via `temp_env::with_var` for their scope.
///
/// Without this, integration tests on a developer machine with a real
/// `~/.config/temper/auth.json` would silently publish test fixtures to the
/// production server.
pub fn init_isolated_auth() {
    AUTH_INIT.call_once(|| {
        // SAFETY: Once ensures this closure runs at most once. The value
        // written is constant across the entire test process, so no test
        // observes a torn or shifting value.
        unsafe {
            std::env::set_var(
                "TEMPER_AUTH_PATH",
                "/tmp/temper-cli-tests-no-such-auth-path/auth.json",
            );
        }
    });
}
