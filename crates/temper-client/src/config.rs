//! Cloud configuration loaded from `~/.config/temper/config.toml`.
//!
//! Uses the canonical [`TemperConfig`] from `temper-core` as the single source
//! of truth. Provides helpers for loading the config, resolving the API URL,
//! extracting OAuth settings, and building a fully-configured client.

use std::path::PathBuf;

use temper_core::types::config::{AuthProviderConfig, CloudSection, TemperConfig};

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns `~/.config/temper/config.toml`.
///
/// We use `~/.config/temper/` explicitly (not the platform-specific config dir)
/// because the CLI and auth.json all live here.
pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config")
        .join("temper")
        .join("config.toml")
}

// ---------------------------------------------------------------------------
// Load / resolve
// ---------------------------------------------------------------------------

/// Load cloud config from `~/.config/temper/config.toml`.
///
/// Returns a [`TemperConfig`] with all defaults applied when the file does not
/// exist. Returns an error only if the file exists but cannot be parsed.
pub fn load_cloud_config() -> crate::error::Result<TemperConfig> {
    load_cloud_config_from(&config_path())
}

/// Load cloud config from an explicit path (used in tests).
pub fn load_cloud_config_from(path: &std::path::Path) -> crate::error::Result<TemperConfig> {
    if !path.exists() {
        return Ok(default_temper_config());
    }
    let content = std::fs::read_to_string(path)?;
    let config: TemperConfig = toml::from_str(&content)
        .map_err(|e| crate::error::ClientError::Other(format!("config parse error: {e}")))?;
    Ok(config)
}

/// Build a default `TemperConfig` suitable for when no config file exists.
///
/// `TemperConfig` requires a `vault` section, so we provide a sensible default
/// path. All other sections use their own `Default` impls.
fn default_temper_config() -> TemperConfig {
    TemperConfig {
        vault: temper_core::types::config::CloudVaultConfig {
            path: "~/projects/knowledge".to_string(),
        },
        sync: Default::default(),
        cli: Default::default(),
        skill: Default::default(),
        auth: Default::default(),
        cloud: CloudSection::default(),
    }
}

/// Return the API base URL, letting `TEMPER_API_URL` take priority.
pub fn api_url(config: &TemperConfig) -> String {
    std::env::var("TEMPER_API_URL").unwrap_or_else(|_| config.cloud.api_url.clone())
}

/// Build an [`OAuthConfig`](crate::login::OAuthConfig) from the active provider.
///
/// Returns an error when the named provider is not present in the config.
pub fn oauth_config(config: &TemperConfig) -> crate::error::Result<crate::login::OAuthConfig> {
    let provider: &AuthProviderConfig = config
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
        audience: Some(provider.audience.clone()),
        callback_url: provider.callback_url.clone(),
        scopes: provider.scopes.clone(),
    })
}

/// Convenience: load config and build a fully-configured [`TemperClient`](crate::TemperClient).
///
/// Reads `~/.config/temper/config.toml`, resolves the API URL (with env-var
/// override), loads the device UUID from `auth.json`, and attaches OAuth
/// config when a provider is configured.
pub fn build_client() -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    let url = api_url(&config);
    let device_id = load_device_id();
    let mut client = crate::TemperClient::new(&url, device_id);
    match oauth_config(&config) {
        Ok(oauth) => {
            client = client.with_oauth(oauth);
        }
        Err(e) => {
            tracing::debug!("OAuth config not available: {e}");
        }
    }
    Ok(client)
}

/// Load the device UUID from auth.json's `device_id` field.
///
/// Returns `None` if not authenticated or if the stored auth predates
/// the device_id field.
fn load_device_id() -> Option<String> {
    crate::auth::load_device_id()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    use temper_core::types::config::AuthConfig;

    /// Serialize tests that mutate `TEMPER_API_URL` to prevent races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // --- load_cloud_config ---

    #[test]
    fn returns_defaults_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "auth0");
        assert_eq!(config.cloud.api_url, "https://temperkb.io");
        let provider = config.auth.providers.get("auth0").unwrap();
        assert_eq!(
            provider.authorize_url,
            "https://temperkb.us.auth0.com/authorize"
        );
        assert_eq!(
            provider.token_url,
            "https://temperkb.us.auth0.com/oauth/token"
        );
        assert_eq!(
            provider.callback_url,
            "https://temperkb.io/api/auth/cli-callback"
        );
    }

    #[test]
    fn parses_valid_config_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let toml = r#"
[vault]
path = "~/projects/knowledge"

[auth]
provider = "my_provider"

[auth.providers.my_provider]
authorize_url = "https://example.com/auth"
token_url     = "https://example.com/token"
client_id     = "abc123"
audience      = "https://example.com/api"
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
        let config = default_temper_config();
        std::env::remove_var("TEMPER_API_URL");
        let url = api_url(&config);
        assert_eq!(url, "https://temperkb.io");
    }

    #[test]
    fn api_url_env_var_takes_priority() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config = default_temper_config();
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
[vault]
path = "~/vault"

[auth]
provider = "neon_auth"

[auth.providers.neon_auth]
authorize_url = "https://id.example.com/auth"
token_url     = "https://id.example.com/token"
client_id     = "client_xyz"
audience      = "https://id.example.com/api"
scopes        = ["openid"]
"#;
        fs::write(&path, toml).unwrap();
        let config = load_cloud_config_from(&path).unwrap();
        let oauth = oauth_config(&config).unwrap();
        assert_eq!(oauth.client_id, "client_xyz");
        assert_eq!(oauth.scopes, vec!["openid"]);
        assert_eq!(
            oauth.audience,
            Some("https://id.example.com/api".to_string())
        );
    }

    #[test]
    fn oauth_config_missing_provider_returns_error() {
        let config = TemperConfig {
            vault: temper_core::types::config::CloudVaultConfig {
                path: "~/vault".to_string(),
            },
            sync: Default::default(),
            cli: Default::default(),
            skill: Default::default(),
            auth: AuthConfig {
                provider: "nonexistent".to_string(),
                providers: HashMap::new(),
            },
            cloud: CloudSection::default(),
        };
        let err = oauth_config(&config).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    // --- default provider tests ---

    #[test]
    fn default_provider_is_auth0_with_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "auth0");
        let provider = config.auth.providers.get("auth0").unwrap();
        assert_eq!(
            provider.authorize_url,
            "https://temperkb.us.auth0.com/authorize"
        );
        assert_eq!(
            provider.token_url,
            "https://temperkb.us.auth0.com/oauth/token"
        );
        assert_eq!(provider.client_id, "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF");
        assert_eq!(provider.audience, "https://temperkb.io/api");
        assert!(provider.scopes.contains(&"offline_access".to_string()));
    }

    #[test]
    fn config_file_overrides_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let toml = r#"
[vault]
path = "~/vault"

[auth]
provider = "keycloak"

[auth.providers.keycloak]
authorize_url = "https://sso.example.com/auth"
token_url     = "https://sso.example.com/token"
client_id     = "custom-client"
audience      = "custom-api"
scopes        = ["openid", "profile"]
"#;
        fs::write(&path, toml).unwrap();
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "keycloak");
        let p = config.auth.providers.get("keycloak").unwrap();
        assert_eq!(p.audience, "custom-api");
    }

    // --- build_client smoke test ---

    #[test]
    fn build_client_succeeds_with_defaults() {
        std::env::remove_var("TEMPER_API_URL");
        let result = build_client();
        assert!(result.is_ok(), "build_client failed: {:?}", result.err());
    }
}
