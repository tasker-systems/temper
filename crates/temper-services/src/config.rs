use crate::auth_config::{parse_auth_config, AuthConfig, ConfigError};
use std::env;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub database_url: String,
    /// This instance's verified auth identity — issuer, JWKS, the one audience, and the mode.
    ///
    /// Replaces the old `auth_issuer` / `auth_audience` / `jwks_url` trio. That `auth_audience` was
    /// an `Option<String>`, and a `None` reached `JwksKeyStore::validation`, which answered it by
    /// setting `validate_aud = false` — so an unset or empty `AUTH_AUDIENCE` silently switched
    /// audience validation off. There is no `None` to hand it any more.
    pub auth: AuthConfig,
    pub auth_provider_name: String,
    pub cors_origins: Vec<String>,
    pub port: u16,
    pub enable_swagger: bool,
    /// Shared secret gating the internal SAML reconcile endpoint. `None` disables the endpoint.
    pub internal_reconcile_secret: Option<String>,
    /// Shared secret gating the internal embed-dispatch drain endpoint (issue #299), called by the
    /// Vercel cron. `None` disables the endpoint (a deployment with no drain configured).
    pub embed_dispatch_secret: Option<String>,
}

impl ApiConfig {
    /// Load from the process environment. Refuses to produce a config an instance cannot serve on.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Load from an arbitrary lookup rather than the process environment.
    ///
    /// Private: the injectable seam that tests actually use is [`parse_auth_config`], which owns
    /// every rule worth testing. Exposing this would be test-only machinery in the public API that
    /// no test even calls.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let auth = parse_auth_config(&lookup)?;

        // The mode is not consulted after parsing. It is logged because an operator who cannot tell
        // which mode their instance is in is exactly the operator who mis-sets these variables.
        tracing::info!(mode = %auth.mode, "auth configured");

        let cors_origins: Vec<String> = lookup("CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if cors_origins.is_empty() {
            tracing::info!(
                "CORS_ORIGINS is not set — cross-origin requests will be denied. \
                 Set CORS_ORIGINS=* for permissive mode in development."
            );
        }

        let enable_swagger = lookup("ENABLE_SWAGGER")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        if enable_swagger {
            tracing::info!("Swagger UI enabled at /api-docs/ui");
        }

        Ok(Self {
            database_url: lookup("DATABASE_URL").ok_or(ConfigError::Missing("DATABASE_URL"))?,
            auth,
            auth_provider_name: lookup("AUTH_PROVIDER_NAME").unwrap_or_else(|| "auth0".to_string()),
            cors_origins,
            port: lookup("PORT").and_then(|p| p.parse().ok()).unwrap_or(3000),
            enable_swagger,
            internal_reconcile_secret: lookup("INTERNAL_RECONCILE_SECRET")
                .filter(|s| !s.is_empty()),
            embed_dispatch_secret: lookup("EMBED_DISPATCH_SECRET").filter(|s| !s.is_empty()),
        })
    }
}
