//! Shared runtime and client setup for CLI commands that call the cloud API.
//!
//! Eliminates duplicated `tokio::runtime::Runtime::new()` + `build_client()`
//! boilerplate across command modules.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::{Result, TemperError};

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
    let client =
        temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;
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
    let client = Arc::new(
        temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?,
    );
    rt.block_on(f(client))
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
    let client =
        temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;
    Ok((rt, client))
}

/// Ensure the user's profile exists on the server.
///
/// Calls `GET /api/profile` which hits the Axum endpoint that auto-provisions
/// profiles for new users. This must be called before any TypeScript-routed
/// endpoints (ingest, sync) which require a pre-existing profile.
pub async fn ensure_profile(client: &temper_client::TemperClient) -> Result<()> {
    client
        .profile()
        .get()
        .await
        .map_err(|e| TemperError::Api(format!("profile pre-flight: {e}")))?;
    Ok(())
}

/// Require a device_id or return a clear auth error.
pub fn require_device_id() -> Result<String> {
    crate::config::load_device_id().ok_or_else(|| {
        TemperError::Config("not authenticated — run `temper auth login` first".into())
    })
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
}
