//! Resource lookup primitives for CLI commands.
//!
//! Currently carries the process-wide profile-slug cache used by
//! `actions::runtime::ensure_profile`.

use std::sync::OnceLock;

/// Process-wide cache of the requester's profile slug (without leading
/// `@`). Populated by `actions::runtime::ensure_profile` on the first
/// authed CLI call that hits the API.
///
/// The cache is fire-and-forget: once set, never cleared. CLI processes
/// are short-lived, so this is sufficient.
static PROFILE_SLUG_CACHE: OnceLock<String> = OnceLock::new();

/// Read the cached profile slug. Returns `None` until
/// `set_cached_profile_slug` runs at least once in this process.
pub fn cached_profile_slug() -> Option<&'static str> {
    PROFILE_SLUG_CACHE.get().map(String::as_str)
}

/// Populate the profile-slug cache. Safe to call multiple times — the
/// `OnceLock` guarantees only the first call wins. Subsequent calls
/// with different values are silently ignored.
pub fn set_cached_profile_slug(slug: String) {
    let _ = PROFILE_SLUG_CACHE.set(slug);
}
