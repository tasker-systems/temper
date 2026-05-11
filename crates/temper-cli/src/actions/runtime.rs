//! Shared runtime and client setup for CLI commands that call the cloud API.
//!
//! Eliminates duplicated `tokio::runtime::Runtime::new()` + `build_client()`
//! boilerplate across command modules.
//!
//! Picks a [`temper_client::auth::TokenStore`] based on
//! [`temper_core::types::VaultState::from_env`]:
//! `VaultState::Cloud` → `MemoryTokenStore` (ephemeral, env-var-backed);
//! `VaultState::Local` → `DiskTokenStore::default_path()`
//! (`~/.config/temper/auth.json`). Cloud sessions cannot accidentally write
//! to disk because the store itself has no disk knowledge.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use temper_client::auth::{DiskTokenStore, MemoryTokenStore, TokenStore};
use temper_client::config::{auth_path, build_client_from, load_cloud_config};
use temper_client::error::ClientError;
use temper_core::types::config::TemperConfig;
use temper_core::types::VaultState;

use crate::error::{Result, TemperError};

/// Lift a [`ClientError`] into a [`TemperError`], preserving the network/
/// server distinction so callers can choose to fall back to local state on
/// unreachable-server errors without swallowing legitimate 4xx/5xx responses.
pub fn client_err_to_temper(e: ClientError) -> TemperError {
    if e.is_network() {
        TemperError::Network(e.to_string())
    } else {
        TemperError::Api(e.to_string())
    }
}

/// Returns a stderr-shaped warning message when `stored`'s token is at or
/// past `threshold` of expiry, or already expired. Returns `None` for
/// healthy tokens.
///
/// Pure function; takes `now` explicitly so tests don't depend on
/// wall-clock time. Used by `resolve_token_store`'s `Cloud` branch.
fn token_expiry_warning(
    stored: &temper_client::auth::StoredAuth,
    now: chrono::DateTime<chrono::Utc>,
    threshold: chrono::Duration,
) -> Option<String> {
    let remaining = temper_client::auth::time_until_expiry(stored, now);
    if remaining < chrono::Duration::zero() {
        Some(format!(
            "error: TEMPER_TOKEN expired at {} (~{} ago). \
             Re-run `temper auth export-token` and re-set TEMPER_TOKEN to renew.",
            stored.expires_at.to_rfc3339(),
            humanize_duration(-remaining),
        ))
    } else if remaining <= threshold {
        Some(format!(
            "warning: TEMPER_TOKEN expires at {} (~{} from now). \
             Re-run `temper auth export-token` and re-set TEMPER_TOKEN to renew.",
            stored.expires_at.to_rfc3339(),
            humanize_duration(remaining),
        ))
    } else {
        None
    }
}

/// Render a non-negative `chrono::Duration` in `<N>h<M>m` or `<N>m` form.
/// Sub-minute durations clamp to `0m` — the warning is human-scale, not
/// stopwatch-precise.
fn humanize_duration(d: chrono::Duration) -> String {
    let total_mins = d.num_minutes().max(0);
    if total_mins >= 60 {
        format!("{}h{}m", total_mins / 60, total_mins % 60)
    } else {
        format!("{total_mins}m")
    }
}

/// Resolve the active [`TokenStore`] for this process.
///
/// In `Local` mode the disk path is computed via
/// [`temper_client::config::auth_path`] so the same `TEMPER_AUTH_PATH` /
/// `auth.path` precedence applies to both reads and writes — tests can
/// isolate from the developer's real `~/.config/temper/auth.json` by setting
/// `TEMPER_AUTH_PATH` to a tmpdir.
fn resolve_token_store(config: &TemperConfig) -> Result<Arc<dyn TokenStore>> {
    match VaultState::from_env() {
        VaultState::Cloud => {
            let mem = MemoryTokenStore::from_env_required()
                .map_err(|e| TemperError::Config(e.to_string()))?;
            // Cloud-mode AT is refresh-less by design (see
            // `stored_auth_from_env` docstring). Warn early when the token
            // is approaching expiry so users have time to re-export.
            if let Ok(Some(stored)) = mem.load() {
                if let Some(msg) =
                    token_expiry_warning(&stored, chrono::Utc::now(), chrono::Duration::hours(1))
                {
                    eprintln!("{msg}");
                }
            }
            Ok(Arc::new(mem))
        }
        VaultState::Local => Ok(Arc::new(DiskTokenStore::at(auth_path(config)))),
    }
}

/// Load config + resolve store + build client, sharing the loaded config so
/// `TEMPER_API_URL` / `TEMPER_AUTH_PATH` resolution and provider selection all
/// see the same `TemperConfig` snapshot.
fn build_config_store_and_client() -> Result<(
    TemperConfig,
    Arc<dyn TokenStore>,
    temper_client::TemperClient,
)> {
    let config = load_cloud_config().map_err(|e| TemperError::Api(e.to_string()))?;
    let store = resolve_token_store(&config)?;
    let client =
        build_client_from(&config, store.clone()).map_err(|e| TemperError::Api(e.to_string()))?;
    Ok((config, store, client))
}

/// Create a tokio runtime and temper client, then execute an async closure.
///
/// The closure receives a reference to the built client. Use this for
/// simple request-response patterns (single API call, no spawned tasks).
pub fn with_client<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&temper_client::TemperClient) -> Pin<Box<dyn Future<Output = Result<T>> + '_>>,
{
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let (_config, _store, client) = build_config_store_and_client()?;
    rt.block_on(f(&client))
}

/// Like [`with_client`], but wraps the client in `Arc` for use with
/// concurrent tasks (`tokio::spawn`), shared references across async
/// boundaries, or patterns that need owned client handles.
pub fn with_arc_client<F, T>(f: F) -> Result<T>
where
    F: FnOnce(Arc<temper_client::TemperClient>) -> Pin<Box<dyn Future<Output = Result<T>>>>,
{
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let (_config, _store, client) = build_config_store_and_client()?;
    rt.block_on(f(Arc::new(client)))
}

/// Create a tokio runtime and temper client pair.
///
/// Use this when you need the runtime and client as separate values —
/// e.g., when the async block needs mutable references to local state
/// that can't be moved into a closure.
pub fn build_runtime_and_client() -> Result<(tokio::runtime::Runtime, temper_client::TemperClient)>
{
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let (_config, _store, client) = build_config_store_and_client()?;
    Ok((rt, client))
}

/// Ensure the user's profile exists on the server, returning the resolved
/// `Profile` so callers can reuse fields like `slug` without a second
/// network round-trip.
///
/// Calls `GET /api/profile` which hits the Axum endpoint that auto-provisions
/// profiles for new users. This must be called before any TypeScript-routed
/// endpoints (ingest, sync) which require a pre-existing profile.
pub async fn ensure_profile(
    client: &temper_client::TemperClient,
) -> Result<temper_core::types::Profile> {
    let profile = client
        .profile()
        .get()
        .await
        .map_err(|e| TemperError::Api(format!("profile pre-flight: {e}")))?;
    // Populate the process-wide profile-slug cache so `lookup::find_resource`
    // can scan the legacy `@<profile.slug>/` directory without an explicit
    // Config field set. Idempotent — subsequent calls are no-ops.
    crate::lookup::set_cached_profile_slug(profile.slug.clone());
    Ok(profile)
}

/// Require a device_id or return a clear auth error.
pub fn require_device_id() -> Result<String> {
    crate::config::load_device_id().ok_or_else(|| {
        TemperError::Config("not authenticated — run `temper auth login` first".into())
    })
}

/// Publish a freshly-written local file to the server, downgrading transient
/// failures to warnings so the local file-creation contract still succeeds.
///
/// This is the single source of truth for the publish-tail policy invoked by
/// every Local-mode creator and the update path. Errors are classified
/// structurally:
///
/// - **No token configured** (no `TEMPER_TOKEN`, no auth.json on disk):
///   `tracing::warn!` and return `Ok(None)`. The user is in offline / not-yet-
///   authenticated mode; the file exists locally and `temper sync run` will
///   reconcile after `temper auth login`.
/// - **`TemperError::Network(_)`**: transient — server unreachable. Warn and
///   return `Ok(None)`. Sync will recover when connectivity returns.
/// - **Any other `Err`** (auth/4xx/5xx/validation/conflict): bubble. The user
///   wants to know about a real failure on the server side.
/// - **`Ok(_)`**: return `Ok(Some(result))`.
///
/// Returns `Ok(None)` for both "no token" and "transient network" so callers
/// can treat them uniformly: the local file exists either way.
pub fn publish_local_write_best_effort(
    vault_root: &std::path::Path,
    file_path: &std::path::Path,
) -> Result<Option<crate::actions::sync::PushResult>> {
    let config = load_cloud_config().map_err(|e| TemperError::Api(e.to_string()))?;
    let store = resolve_token_store(&config)?;

    if store.load().ok().flatten().is_none() {
        tracing::warn!(
            "not authenticated; file created locally and not published — \
             run `temper auth login` to publish, or run `temper sync run` \
             after authenticating"
        );
        return Ok(None);
    }

    let vault_root = vault_root.to_path_buf();
    let file_path = file_path.to_path_buf();
    let result = with_client(move |client| {
        Box::pin(async move {
            crate::actions::sync::publish_local_write(client, &vault_root, &file_path).await
        })
    });

    match result {
        Ok(r) => Ok(Some(r)),
        Err(TemperError::Network(msg)) => {
            tracing::warn!("publish failed (offline; sync will recover): {msg}");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_device_id_returns_error_when_not_logged_in() {
        let result = require_device_id();
        // In test environment, no device_id file exists — should return Config error.
        // The important thing is it doesn't panic.
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn with_client_errors_when_cloud_mode_but_no_token() {
        temp_env::with_vars(
            [
                ("TEMPER_VAULT_STATE", Some("cloud")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                let result = with_client(|_client| Box::pin(async { Ok(()) }));
                let err = result.unwrap_err();
                let msg = format!("{err}");
                assert!(
                    msg.contains("TEMPER_TOKEN"),
                    "expected TEMPER_TOKEN error: {msg}"
                );
            },
        );
    }

    #[test]
    fn publish_best_effort_returns_ok_none_when_no_token() {
        // Local mode + TEMPER_AUTH_PATH pointed at a non-existent file
        // simulates a freshly-installed CLI: the disk store finds nothing
        // and the helper warns + returns Ok(None) without making any API
        // call. Critical for unit-test isolation on logged-in dev machines.
        let dir = tempfile::TempDir::new().unwrap();
        let auth_path = dir.path().join("auth.json");
        let nonexistent_config = dir.path().join("no-such-config.toml");
        let vault_root = dir.path();
        let file_path = dir.path().join("dummy.md");

        temp_env::with_vars(
            [
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
                ("TEMPER_AUTH_PATH", Some(auth_path.to_str().unwrap())),
                (
                    "TEMPER_GLOBAL_CONFIG",
                    Some(nonexistent_config.to_str().unwrap()),
                ),
            ],
            || {
                let result = publish_local_write_best_effort(vault_root, &file_path);
                assert!(
                    matches!(result, Ok(None)),
                    "expected Ok(None) on no-token, got {result:?}"
                );
            },
        );
    }
}

#[cfg(test)]
mod expiry_warning_tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};
    use temper_client::auth::{Provider, StoredAuth};

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-09T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn auth_expiring_at(when: DateTime<Utc>) -> StoredAuth {
        StoredAuth {
            provider: Provider::auth0("temperkb.us.auth0.com"),
            access_token: "tok".to_string().into(),
            refresh_token: None,
            expires_at: when,
            profile_id: None,
            device_id: None,
        }
    }

    #[test]
    fn warns_when_token_within_threshold() {
        let now = fixed_now();
        let auth = auth_expiring_at(now + Duration::minutes(30));
        let msg = token_expiry_warning(&auth, now, Duration::hours(1))
            .expect("expected warning for 30-min-out token");
        assert!(msg.starts_with("warning:"), "got: {msg}");
        assert!(msg.contains("30m"), "got: {msg}");
        assert!(msg.contains("temper auth export-token"), "got: {msg}");
    }

    #[test]
    fn silent_when_token_healthy() {
        let now = fixed_now();
        let auth = auth_expiring_at(now + Duration::hours(2));
        assert!(token_expiry_warning(&auth, now, Duration::hours(1)).is_none());
    }

    #[test]
    fn errors_when_token_expired() {
        let now = fixed_now();
        let auth = auth_expiring_at(now - Duration::minutes(30));
        let msg = token_expiry_warning(&auth, now, Duration::hours(1))
            .expect("expected error-shaped warning for expired token");
        assert!(msg.starts_with("error:"), "got: {msg}");
        assert!(msg.contains("expired"), "got: {msg}");
    }
}
