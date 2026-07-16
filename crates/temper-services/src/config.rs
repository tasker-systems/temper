use crate::auth_config::{parse_auth_config, AuthConfig, ConfigError};
use crate::broker::VercelConnectConfig;
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
    /// Vercel Connect broker credentials. `None` when the four env vars are not all
    /// set — the deployment then has a `NullBroker` and mints fail clearly. Never
    /// hardcoded; a self-hosted operator sets their own.
    pub vercel_connect: Option<VercelConnectConfig>,
    /// Slack account-link configuration. `None` when the three env vars are not all
    /// set — the link flow's endpoints are then disabled rather than half-configured.
    pub slack_link: Option<SlackLinkConfig>,
}

/// Slack account-link configuration. `None` when the three values are not all present —
/// a partial set is treated as unconfigured, so the endpoints are disabled rather than
/// half-configured (the `parse_vercel_connect` precedent).
#[derive(Debug, Clone)]
pub struct SlackLinkConfig {
    /// The OAuth client the link flow authorizes as. Its redirect_uri must be registered:
    /// Auth0's Allowed Callback URLs, or `AS_CLIENTS` on an AS instance.
    pub client_id: String,
    /// Shared with the mention agent; gates `POST /internal/slack/link-intents`.
    /// Distinct from `INTERNAL_RECONCILE_SECRET`: a different principal gets a different secret.
    pub hmac_secret: String,
    /// This instance's public origin, used to build the callback redirect_uri.
    pub public_base_url: String,
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
            vercel_connect: parse_vercel_connect(&lookup),
            slack_link: parse_slack_link(&lookup),
        })
    }
}

/// Build the Vercel Connect config from env — `Some` only when all four values are
/// present and non-empty. A partial set is treated as unconfigured (the safer
/// default: a `NullBroker` that fails mints loudly, not a half-configured one that
/// fails obscurely at request time).
fn parse_vercel_connect(lookup: impl Fn(&str) -> Option<String>) -> Option<VercelConnectConfig> {
    let get = |k| lookup(k).filter(|s: &String| !s.is_empty());
    Some(VercelConnectConfig {
        access_token: get("VERCEL_CONNECT_ACCESS_TOKEN")?,
        project_id: get("VERCEL_CONNECT_PROJECT_ID")?,
        team_id: get("VERCEL_CONNECT_TEAM_ID")?,
        team_slug: get("VERCEL_CONNECT_TEAM_SLUG")?,
    })
}

/// Build the Slack link config from env — `Some` only when all three values are
/// present and non-empty (the `parse_vercel_connect` all-or-nothing precedent).
fn parse_slack_link(lookup: impl Fn(&str) -> Option<String>) -> Option<SlackLinkConfig> {
    let get = |k| lookup(k).filter(|s: &String| !s.is_empty());
    Some(SlackLinkConfig {
        client_id: get("SLACK_LINK_CLIENT_ID")?,
        hmac_secret: get("SLACK_LINK_SECRET")?,
        public_base_url: get("PUBLIC_BASE_URL")?,
    })
}
