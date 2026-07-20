use crate::auth_config::{parse_auth_config, AuthConfig, ConfigError};
use crate::broker::VercelConnectConfig;
use crate::services::grant_crypto::VaultKey;
use std::env;

/// The instance's whole configuration.
///
/// `Debug` is hand-written to REDACT `internal_reconcile_secret`, `embed_dispatch_secret` and
/// `slack_mint_secret` — the three plaintext shared secrets behind three separate signature gates,
/// the last of which vends a token acting as any linked human. A derived `Debug` would print all
/// three verbatim wherever an `ApiConfig` is formatted. This is the same reasoning already spelled
/// out on [`SlackLinkConfig`] below ("would print it verbatim wherever this or the enclosing
/// `ApiConfig` is formatted") — the nested config got the treatment before its parent did.
///
/// Redaction is PRESENCE-PRESERVING: each secret prints as `Some("redacted")` or `None`, because
/// *whether* a secret is configured is exactly the operational fact a config dump is read for
/// (each `None` disables an endpoint), while its value is exactly the fact that must never reach
/// a log sink.
#[derive(Clone)]
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
    /// Shared secret gating the internal **mint** endpoint (`/internal/slack/mint`), which vends
    /// an act-as-the-human access token to the Slack mention agent. `None` disables that endpoint.
    ///
    /// DELIBERATELY NOT a field on [`SlackLinkConfig`], for two independent reasons:
    ///
    /// 1. **Privilege asymmetry.** `SLACK_LINK_SECRET` gates an endpoint that answers *"is this
    ///    principal linked?"*. This one gates an endpoint that hands back **a token acting as any
    ///    linked human, with that human's full reach**. The existing two-secret split
    ///    (`INTERNAL_RECONCILE_SECRET` vs `SLACK_LINK_SECRET`) exists so neither principal can
    ///    forge the other's calls; the same reasoning applies with far more force here, because
    ///    the endpoint that *confers reach* must not share a key with one that merely answers a
    ///    question. Compromising the low-privilege secret must not yield act-as-any-user.
    /// 2. **`parse_slack_link` is all-or-nothing.** Folding this in would mean a deploy that has
    ///    not yet set `SLACK_MINT_SECRET` silently disables the *entire* link flow — which is
    ///    live in production today. Additive and independent keeps that from being a cliff: an
    ///    unset mint secret disables minting only, and the link flow is untouched.
    pub slack_mint_secret: Option<String>,
}

/// Slack account-link configuration. `None` when the three values are not all present —
/// a partial set is treated as unconfigured, so the endpoints are disabled rather than
/// half-configured (the `parse_vercel_connect` precedent).
///
/// `Debug` is hand-written to REDACT `hmac_secret` (a plain `String` shared secret) — a derived
/// `Debug` would print it verbatim wherever this or the enclosing `ApiConfig` is formatted.
/// `vault_key` is already redacted by its own `Debug`.
#[derive(Clone)]
pub struct SlackLinkConfig {
    /// The OAuth client the link flow authorizes as. Its redirect_uri must be registered:
    /// Auth0's Allowed Callback URLs, or `AS_CLIENTS` on an AS instance.
    pub client_id: String,
    /// Shared with the mention agent; gates `POST /internal/slack/link-intents`.
    /// Distinct from `INTERNAL_RECONCILE_SECRET`: a different principal gets a different secret.
    pub hmac_secret: String,
    /// This instance's public origin, used to build the callback redirect_uri.
    pub public_base_url: String,
    /// The AEAD key the grant vault (T3) seals each per-user refresh token under. REQUIRED: an
    /// instance that can link accounts but cannot vault the grant is one whose links are inert
    /// (nothing can act as the human at mention time), so the flow is on only when the vault is
    /// too. Parsed once from `SLACK_VAULT_ENC_KEY` (32 bytes, base64) — a malformed key disables
    /// the whole link flow rather than half-configuring it.
    pub vault_key: VaultKey,
}

impl std::fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `.as_ref().map(|_| "redacted")` rather than a flat `&"redacted"`: it keeps the
        // Some/None distinction (which endpoint is enabled) while dropping the value.
        f.debug_struct("ApiConfig")
            .field("database_url", &self.database_url)
            .field("auth", &self.auth)
            .field("auth_provider_name", &self.auth_provider_name)
            .field("cors_origins", &self.cors_origins)
            .field("port", &self.port)
            .field("enable_swagger", &self.enable_swagger)
            .field(
                "internal_reconcile_secret",
                &self.internal_reconcile_secret.as_ref().map(|_| "redacted"),
            )
            .field(
                "embed_dispatch_secret",
                &self.embed_dispatch_secret.as_ref().map(|_| "redacted"),
            )
            .field("vercel_connect", &self.vercel_connect)
            .field("slack_link", &self.slack_link)
            .field(
                "slack_mint_secret",
                &self.slack_mint_secret.as_ref().map(|_| "redacted"),
            )
            .finish()
    }
}

impl std::fmt::Debug for SlackLinkConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackLinkConfig")
            .field("client_id", &self.client_id)
            .field("hmac_secret", &"redacted")
            .field("public_base_url", &self.public_base_url)
            .field("vault_key", &self.vault_key)
            .finish()
    }
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
            slack_mint_secret: lookup("SLACK_MINT_SECRET").filter(|s| !s.is_empty()),
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

/// Build the Slack link config from env — `Some` only when all FOUR values are present, non-empty,
/// AND the vault key parses (the `parse_vercel_connect` all-or-nothing precedent, extended to
/// T3's required key). A malformed `SLACK_VAULT_ENC_KEY` disables the flow with a loud error
/// rather than booting a link flow whose vault writes would fail at the callback seam.
fn parse_slack_link(lookup: impl Fn(&str) -> Option<String>) -> Option<SlackLinkConfig> {
    let get = |k| lookup(k).filter(|s: &String| !s.is_empty());

    let raw_key = get("SLACK_VAULT_ENC_KEY")?;
    let vault_key = match VaultKey::from_base64(&raw_key) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(
                "SLACK_VAULT_ENC_KEY is set but invalid ({e}); the Slack link flow is disabled. \
                 Expected 32 bytes, base64 (generate with `openssl rand -base64 32`)."
            );
            return None;
        }
    };

    Some(SlackLinkConfig {
        client_id: get("SLACK_LINK_CLIENT_ID")?,
        hmac_secret: get("SLACK_LINK_SECRET")?,
        public_base_url: get("PUBLIC_BASE_URL")?,
        vault_key,
    })
}
