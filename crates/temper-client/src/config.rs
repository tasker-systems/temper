//! Cloud configuration loaded from `~/.config/temper/config.toml`.
//!
//! Uses the canonical [`TemperConfig`] from `temper-core` as the single source
//! of truth. Path resolution, default construction, and parsing all live in
//! `temper_core::types::config`. This module re-exports the core helpers and
//! adds client-specific conveniences (API URL resolution, OAuth config, and
//! building a fully-configured client).

use temper_core::types::config::{AuthProvider, TemperConfig};

// ---------------------------------------------------------------------------
// Load / resolve  (delegated to temper-core)
// ---------------------------------------------------------------------------

/// Load cloud config from `~/.config/temper/config.toml`.
///
/// Returns a [`TemperConfig`] with defaults when the file does not exist.
/// Returns an error only if the file exists but cannot be parsed.
pub fn load_cloud_config() -> crate::error::Result<TemperConfig> {
    temper_core::types::config::load_config().map_err(crate::error::ClientError::Other)
}

/// Load cloud config from an explicit path (used in tests).
pub fn load_cloud_config_from(path: &std::path::Path) -> crate::error::Result<TemperConfig> {
    temper_core::types::config::load_config_from(path).map_err(crate::error::ClientError::Other)
}

/// Return the API base URL, letting `TEMPER_API_URL` take priority.
pub fn api_url(config: &TemperConfig) -> String {
    std::env::var("TEMPER_API_URL").unwrap_or_else(|_| config.cloud.api_url.clone())
}

/// Build an [`OAuthConfig`](crate::login::OAuthConfig) from the active provider.
///
/// Returns an error when the named provider is not present in the config,
/// or when `auth.provider` is set to `"none"` (cloud sync disabled).
pub fn oauth_config(config: &TemperConfig) -> crate::error::Result<crate::login::OAuthConfig> {
    let provider: &AuthProvider = config
        .auth
        .providers
        .iter()
        .find(|p| p.name == config.auth.provider)
        .ok_or_else(|| {
            let msg = if config.auth.provider == "none" || config.auth.providers.is_empty() {
                "cloud sync is disabled for this vault — run `temper config edit` and set \
                 `auth.provider` to a configured provider, or re-run `temper init` and \
                 pick an auth provider"
                    .to_string()
            } else {
                format!(
                    "auth provider '{}' not found in [[auth.providers]] — run \
                     `temper config edit` to fix",
                    config.auth.provider
                )
            };
            crate::error::ClientError::Other(msg)
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

/// Build client from explicit config and optional stored auth (no disk reads).
///
/// When `auth` is provided, the access token is set as a token override on the
/// underlying `HttpClient`, bypassing `auth.json` reads in all sub-clients.
pub fn build_client_from(
    config: &TemperConfig,
    auth: Option<&crate::auth::StoredAuth>,
) -> crate::error::Result<crate::TemperClient> {
    let url = api_url(config);
    let device_id = auth.and_then(|a| a.device_id.clone());

    let client = if let Some(auth) = auth {
        crate::TemperClient::with_token(&url, device_id, auth.access_token.clone())
    } else {
        crate::TemperClient::new(&url, device_id)
    };

    let client = match oauth_config(config) {
        Ok(oauth) => client.with_oauth(oauth),
        Err(e) => {
            tracing::debug!("OAuth config not available: {e}");
            client
        }
    };

    Ok(client)
}

/// Convenience: load config and build a fully-configured [`TemperClient`](crate::TemperClient).
///
/// Reads `~/.config/temper/config.toml`, resolves the API URL (with env-var
/// override), loads the device UUID from `auth.json`, and attaches OAuth
/// config when a provider is configured.
pub fn build_client() -> crate::error::Result<crate::TemperClient> {
    let config = load_cloud_config()?;
    let auth = crate::auth::load_auth().ok().flatten();
    build_client_from(&config, auth.as_ref())
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

    use temper_core::types::config::{AuthConfig, CloudSection};

    /// Serialize tests that mutate `TEMPER_API_URL` to prevent races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Find a provider in the Vec by name (test convenience).
    fn find_provider<'a>(config: &'a TemperConfig, name: &str) -> &'a AuthProvider {
        config
            .auth
            .providers
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("provider {name} not found"))
    }

    // --- load_cloud_config ---

    #[test]
    fn returns_defaults_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "auth0");
        assert_eq!(config.cloud.api_url, "https://temperkb.io");
        let provider = find_provider(&config, "auth0");
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

[[auth.providers]]
name          = "my_provider"
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
        let p = find_provider(&config, "my_provider");
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
        let config = TemperConfig::default();
        std::env::remove_var("TEMPER_API_URL");
        let url = api_url(&config);
        assert_eq!(url, "https://temperkb.io");
    }

    #[test]
    fn api_url_env_var_takes_priority() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config = TemperConfig::default();
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

[[auth.providers]]
name          = "neon_auth"
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
            skill: Default::default(),
            auth: AuthConfig {
                provider: "nonexistent".to_string(),
                providers: Vec::new(),
            },
            cloud: CloudSection::default(),
            llm: Default::default(),
        };
        let err = oauth_config(&config).unwrap_err();
        let msg = err.to_string();
        // With empty providers, the helpful error guides the user
        assert!(
            msg.contains("cloud sync is disabled") || msg.contains("temper config edit"),
            "expected helpful guidance, got: {msg}"
        );
    }

    #[test]
    fn oauth_config_none_provider_returns_disabled_hint() {
        let config = TemperConfig {
            vault: temper_core::types::config::CloudVaultConfig {
                path: "~/vault".to_string(),
            },
            sync: Default::default(),
            skill: Default::default(),
            auth: AuthConfig {
                provider: "none".to_string(),
                providers: Vec::new(),
            },
            cloud: CloudSection::default(),
            llm: Default::default(),
        };
        let err = oauth_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cloud sync is disabled"),
            "expected disabled hint, got: {msg}"
        );
    }

    // --- default provider tests ---

    #[test]
    fn default_provider_is_auth0_with_config() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "auth0");
        let provider = find_provider(&config, "auth0");
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

[[auth.providers]]
name          = "keycloak"
authorize_url = "https://sso.example.com/auth"
token_url     = "https://sso.example.com/token"
client_id     = "custom-client"
audience      = "custom-api"
scopes        = ["openid", "profile"]
"#;
        fs::write(&path, toml).unwrap();
        let config = load_cloud_config_from(&path).unwrap();
        assert_eq!(config.auth.provider, "keycloak");
        let p = find_provider(&config, "keycloak");
        assert_eq!(p.audience, "custom-api");
    }

    // --- build_client smoke test ---

    #[test]
    fn build_client_succeeds_with_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Point TEMPER_GLOBAL_CONFIG at a non-existent path inside a temp dir
        // so load_config() falls back to TemperConfig::default() instead of
        // reading the developer's real ~/.config/temper/config.toml (which
        // might be in any format at any time).
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("no-such-config.toml");
        std::env::set_var("TEMPER_GLOBAL_CONFIG", &nonexistent);
        std::env::remove_var("TEMPER_API_URL");
        let result = build_client();
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
        assert!(result.is_ok(), "build_client failed: {:?}", result.err());
    }

    // --- build_client_from ---

    #[test]
    fn build_client_from_uses_config_api_url() {
        let config = TemperConfig {
            cloud: CloudSection {
                api_url: "https://test.example.com".to_string(),
            },
            ..TemperConfig::default()
        };
        let auth = crate::auth::StoredAuth {
            provider: "test".to_string(),
            access_token: "test-token".to_string(),
            refresh_token: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            profile_id: None,
            device_id: Some("test-device".to_string()),
        };
        let client = build_client_from(&config, Some(&auth)).unwrap();
        // Client was constructed without reading disk — verify it exists
        assert!(format!("{:?}", client).contains("test.example.com"));
    }
}
