//! Cloud configuration loaded from `~/.config/temper/config.toml`.
//!
//! Provides provider-agnostic auth and API URL resolution, shared by
//! `temper-cli`, `temper-mcp`, and any future client crate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

/// Top-level cloud configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub cloud: CloudSection,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            auth: AuthConfig::default(),
            cloud: CloudSection::default(),
        }
    }
}

/// Authentication configuration — which provider is active and how to reach it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Active provider name (e.g., `"neon_auth"`).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Provider-specific OAuth configurations, keyed by provider name.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            providers: HashMap::new(),
        }
    }
}

/// OAuth2 PKCE provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Cloud API section of the configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSection {
    /// API base URL (overridden by `TEMPER_API_URL` environment variable).
    #[serde(default = "default_api_url")]
    pub api_url: String,
}

impl Default for CloudSection {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
        }
    }
}

fn default_provider() -> String {
    "neon_auth".into()
}

fn default_api_url() -> String {
    "https://temperkb.io".into()
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns `~/.config/temper/config.toml` (or the platform equivalent).
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("temper")
        .join("config.toml")
}

// ---------------------------------------------------------------------------
// Load / resolve
// ---------------------------------------------------------------------------

/// Load cloud config from `~/.config/temper/config.toml`.
///
/// Returns a [`CloudConfig`] with all defaults applied when the file does not
/// exist. Returns an error only if the file exists but cannot be parsed.
pub fn load_cloud_config() -> crate::error::Result<CloudConfig> {
    load_cloud_config_from(&config_path())
}

/// Load cloud config from an explicit path (used in tests).
pub fn load_cloud_config_from(path: &std::path::Path) -> crate::error::Result<CloudConfig> {
    if !path.exists() {
        return Ok(CloudConfig::default());
    }
    let content = std::fs::read_to_string(path)?;
    let config: CloudConfig = toml::from_str(&content)
        .map_err(|e| crate::error::ClientError::Other(format!("config parse error: {e}")))?;
    Ok(config)
}

/// Return the API base URL, letting `TEMPER_API_URL` take priority.
pub fn api_url(config: &CloudConfig) -> String {
    std::env::var("TEMPER_API_URL").unwrap_or_else(|_| config.cloud.api_url.clone())
}

/// Build an [`OAuthConfig`](crate::login::OAuthConfig) from the active provider.
///
/// Returns an error when the named provider is not present in the config.
pub fn oauth_config(config: &CloudConfig) -> crate::error::Result<crate::login::OAuthConfig> {
    let provider = config
        .auth
        .providers
        .get(&config.auth.provider)
        .ok_or_else(|| {
            crate::error::ClientError::Other(format!(
                "auth provider '{}' not found in config",
                config.auth.provider
            ))
        })?;
    Ok(crate::login::OAuthConfig {
        authorize_url: provider.authorize_url.clone(),
        token_url: provider.token_url.clone(),
        client_id: provider.client_id.clone(),
        scopes: provider.scopes.clone(),
    })
}

/// Convenience: load config and build a fully-configured [`TemperClient`](crate::TemperClient).
///
/// Reads `~/.config/temper/config.toml`, resolves the API URL (with env-var
/// override), loads the device UUID from `~/.config/temper/device.json`, and
/// attaches OAuth config when a provider is configured.
pub fn build_client() -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    let url = api_url(&config);
    let device_id = load_device_id();
    let mut client = crate::TemperClient::new(&url, device_id);
    if let Ok(oauth) = oauth_config(&config) {
        client = client.with_oauth(oauth);
    }
    Ok(client)
}

/// Try to read the device UUID from `~/.config/temper/device.json`.
///
/// Returns `None` if the file is absent or cannot be parsed.
fn load_device_id() -> Option<String> {
    let path = dirs::config_dir()?.join("temper").join("device.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("client_id")?.as_str().map(String::from)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialize tests that mutate `TEMPER_API_URL` to prevent races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // --- load_cloud_config ---

    #[test]
    fn returns_defaults_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "neon_auth");
        assert_eq!(config.cloud.api_url, "https://temperkb.io");
        assert!(config.auth.providers.is_empty());
    }

    #[test]
    fn parses_valid_config_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let toml = r#"
[auth]
provider = "my_provider"

[auth.providers.my_provider]
authorize_url = "https://example.com/auth"
token_url     = "https://example.com/token"
client_id     = "abc123"
scopes        = ["openid", "profile"]

[cloud]
api_url = "https://api.example.com"
"#;
        fs::write(&path, toml).unwrap();
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "my_provider");
        assert_eq!(config.cloud.api_url, "https://api.example.com");
        let p = config.auth.providers.get("my_provider").unwrap();
        assert_eq!(p.client_id, "abc123");
        assert_eq!(p.scopes, vec!["openid", "profile"]);
    }

    #[test]
    fn returns_error_on_invalid_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[[[ not valid toml").unwrap();
        assert!(load_cloud_config_from(&path).is_err());
    }

    // --- api_url ---

    #[test]
    fn api_url_uses_config_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config = CloudConfig::default();
        // Remove any env var that may be set in the test environment.
        std::env::remove_var("TEMPER_API_URL");
        let url = api_url(&config);
        assert_eq!(url, "https://temperkb.io");
    }

    #[test]
    fn api_url_env_var_takes_priority() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config = CloudConfig::default();
        std::env::set_var("TEMPER_API_URL", "https://localhost:3000");
        let url = api_url(&config);
        std::env::remove_var("TEMPER_API_URL");
        assert_eq!(url, "https://localhost:3000");
    }

    // --- oauth_config ---

    #[test]
    fn oauth_config_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let toml = r#"
[auth]
provider = "neon_auth"

[auth.providers.neon_auth]
authorize_url = "https://id.example.com/auth"
token_url     = "https://id.example.com/token"
client_id     = "client_xyz"
scopes        = ["openid"]
"#;
        fs::write(&path, toml).unwrap();
        let config = load_cloud_config_from(&path).unwrap();
        let oauth = oauth_config(&config).unwrap();
        assert_eq!(oauth.client_id, "client_xyz");
        assert_eq!(oauth.scopes, vec!["openid"]);
    }

    #[test]
    fn oauth_config_missing_provider_returns_error() {
        let config = CloudConfig::default(); // no providers defined
        let err = oauth_config(&config).unwrap_err();
        assert!(err.to_string().contains("neon_auth"));
    }

    // --- build_client smoke test ---

    #[test]
    fn build_client_succeeds_with_defaults() {
        // Ensure env var doesn't interfere.
        std::env::remove_var("TEMPER_API_URL");
        // With no config file this should still produce a client (no oauth).
        let result = build_client();
        assert!(result.is_ok(), "build_client failed: {:?}", result.err());
    }
}
