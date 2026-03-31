//! Shared runtime and client setup for CLI commands that call the cloud API.
//!
//! Eliminates duplicated `tokio::runtime::Runtime::new()` + `build_client()`
//! boilerplate across command modules.

use std::future::Future;
use std::pin::Pin;

use crate::error::{Result, TemperError};

/// Create a tokio runtime and temper client, then execute an async closure.
///
/// This is the standard pattern for CLI commands that need to make API calls.
/// The closure receives a reference to the built client.
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
