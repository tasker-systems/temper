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

        // Auth identity first, then secret hygiene, then everything that merely has to be present.
        check_secret_distinctness(&lookup)?;

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

/// Every variable whose plaintext value is a standalone credential: hold the string, exercise the
/// capability. Four gate an internal surface; the fifth decrypts what one of them protects.
///
/// | Variable                    | Capability it confers                                          |
/// | --------------------------- | -------------------------------------------------------------- |
/// | `INTERNAL_RECONCILE_SECRET` | call `/internal/saml/reconcile`                                  |
/// | `EMBED_DISPATCH_SECRET`     | call the embed drain crons                                       |
/// | `SLACK_LINK_SECRET`         | ask `/internal/slack/link-state` *"is this principal linked?"*    |
/// | `SLACK_MINT_SECRET`         | mint a token acting as **any linked human, with their full reach**|
/// | `SLACK_VAULT_ENC_KEY`       | decrypt **every** vaulted refresh token                           |
///
/// The order is load-bearing only in that it fixes which pair a multi-way collision reports, so the
/// error is deterministic rather than dependent on iteration order.
///
/// `SLACK_VAULT_ENC_KEY` is in this list even though it is an AEAD key rather than a gate secret,
/// and it is the most dangerous member. `SLACK_LINK_SECRET` and `SLACK_MINT_SECRET` are **shared
/// with the Slack mention agent**, which runs as a separate Vercel deployment; the vault key never
/// leaves temper-api. If the vault key equals either of them, the agent holds the key to every
/// stored grant. And `openssl rand -base64 32` is the documented generator for the vault key
/// (`parse_slack_link` above says so), which makes "generate once, paste everywhere" the exact
/// operator error this guards.
const SHARED_SECRET_VARS: [&str; 5] = [
    "INTERNAL_RECONCILE_SECRET",
    "EMBED_DISPATCH_SECRET",
    "SLACK_LINK_SECRET",
    "SLACK_MINT_SECRET",
    "SLACK_VAULT_ENC_KEY",
];

/// Refuse to boot when two shared secrets hold the same value.
///
/// **This is the value-level twin of a structural check that already exists.**
/// `.github/scripts/audit-signature-secrets.sh` asserts each signature gate reads a distinct config
/// *field* — a property of the source. It is satisfied by two gates reading two differently-named
/// env vars, whatever those vars happen to contain. Nothing looked at the deployed *values*, so an
/// operator wiring a new instance by copy-paste could collapse the privilege split silently: every
/// gate still passes, nothing logs, and the security property the split exists for is simply untrue
/// of that deployment. A test cannot catch it either — a test can only tell two secrets apart in an
/// environment that already has them distinct.
///
/// Checked over the **raw environment**, not over the parsed [`ApiConfig`], and that is deliberate.
/// `parse_slack_link` is all-or-nothing, so a `SLACK_LINK_SECRET` colliding with the mint secret
/// would be invisible to a config-level check whenever some *other* Slack variable happens to be
/// unset. That collision is latent, not absent: it goes live the moment the operator fills in the
/// missing variable, and boot is the last moment anyone is looking. The property being asserted is
/// about the values an operator sets, so the environment is the honest place to assert it.
///
/// Only [`ApiConfig::from_lookup`] runs this, so the in-process test harnesses that build an
/// `ApiConfig` by struct literal are unaffected — correctly, since they are not deployments.
fn check_secret_distinctness(lookup: impl Fn(&str) -> Option<String>) -> Result<(), ConfigError> {
    // Empty is absent (the `.filter(|s| !s.is_empty())` convention every field above uses). Two
    // unset variables both reading "" are not a collision — they are two disabled endpoints.
    let present: Vec<(&'static str, String)> = SHARED_SECRET_VARS
        .iter()
        .filter_map(|&name| lookup(name).filter(|v| !v.is_empty()).map(|v| (name, v)))
        .collect();

    for (i, (a_name, a_value)) in present.iter().enumerate() {
        for (b_name, b_value) in &present[i + 1..] {
            if a_value == b_value {
                return Err(ConfigError::SecretCollision(a_name, b_name));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a lookup from pairs. Absent keys return None. (The `auth_config::tests` helper,
    /// which this module cannot reach across the `mod` boundary.)
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    /// The minimum a real instance needs to get past every *other* check in `from_lookup` — so a
    /// failure in these tests is the distinctness check and nothing else.
    fn bootable() -> Vec<(&'static str, &'static str)> {
        vec![
            ("DATABASE_URL", "postgresql://localhost/temper_test"),
            ("AUTH_ISSUER", "https://tenant.auth0.com/"),
            ("JWKS_URL", "https://tenant.auth0.com/.well-known/jwks.json"),
            ("AUTH_AUDIENCE", "https://temperkb.io/api"),
        ]
    }

    /// A distinct value per secret — the correct configuration.
    fn distinct_secrets() -> Vec<(&'static str, String)> {
        SHARED_SECRET_VARS
            .iter()
            .enumerate()
            .map(|(i, &name)| (name, format!("secret-value-number-{i}")))
            .collect()
    }

    fn with_secrets(secrets: &[(&'static str, String)]) -> Vec<(String, String)> {
        bootable()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .chain(secrets.iter().map(|(k, v)| ((*k).to_string(), v.clone())))
            .collect()
    }

    fn lookup_of(pairs: &[(String, String)]) -> impl Fn(&str) -> Option<String> + '_ {
        let refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        env(&refs)
    }

    // --- the invariant, over the whole pair space ---

    // FAILS IF: any two shared secrets may hold the same value. Exhaustive over the pairs rather
    // than hand-listing them, so a SIXTH entry in SHARED_SECRET_VARS is covered against all five
    // incumbents the moment it is added — with no test edit, which is exactly the edit that would
    // be forgotten. (A hand-written pair list is also how you end up asserting the pair you already
    // thought of and none of the ones you didn't.)
    #[test]
    fn every_pair_of_shared_secrets_must_differ() {
        for (i, &a) in SHARED_SECRET_VARS.iter().enumerate() {
            for &b in &SHARED_SECRET_VARS[i + 1..] {
                let mut secrets = distinct_secrets();
                // Collapse b onto a's value — the copy-paste an operator actually makes.
                let a_value = secrets
                    .iter()
                    .find(|(n, _)| *n == a)
                    .map(|(_, v)| v.clone())
                    .expect("a is in SHARED_SECRET_VARS");
                for entry in secrets.iter_mut() {
                    if entry.0 == b {
                        entry.1 = a_value.clone();
                    }
                }

                assert_eq!(
                    check_secret_distinctness(env(&secrets
                        .iter()
                        .map(|(k, v)| (*k, v.as_str()))
                        .collect::<Vec<_>>())),
                    Err(ConfigError::SecretCollision(a, b)),
                    "{a} and {b} were allowed to hold the same value",
                );
            }
        }
    }

    // FAILS IF: the check fires on a correct configuration. The other half of the pair — a guard
    // that rejects everything is not a guard, and this is the assertion that would catch it.
    #[test]
    fn all_distinct_secrets_are_accepted() {
        let secrets = distinct_secrets();
        let pairs: Vec<(&str, &str)> = secrets.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(check_secret_distinctness(env(&pairs)), Ok(()));
    }

    // FAILS IF: absence is read as collision. Two unset secrets both look like "" and are NOT the
    // same credential — they are two disabled endpoints, which is a supported deployment (every
    // e2e harness but one leaves `embed_dispatch_secret: None`).
    #[test]
    fn unset_and_empty_secrets_do_not_collide() {
        assert_eq!(check_secret_distinctness(env(&[])), Ok(()));
        assert_eq!(
            check_secret_distinctness(env(&[
                ("SLACK_LINK_SECRET", ""),
                ("SLACK_MINT_SECRET", ""),
                ("EMBED_DISPATCH_SECRET", ""),
            ])),
            Ok(())
        );
    }

    // FAILS IF: only ONE of a colliding pair is set — that is a single secret, not a shared one.
    #[test]
    fn a_lone_secret_never_collides() {
        assert_eq!(
            check_secret_distinctness(env(&[("SLACK_MINT_SECRET", "only-one-set")])),
            Ok(())
        );
    }

    // FAILS IF: the collision error names the wrong pair, or prints the colliding secret.
    //
    // The sibling assertion on `ConfigError::McpAudienceMismatch`
    // (`auth_config::tests::errors_name_the_variable_and_never_print_values`) carries the same
    // obligation for a URL. Here the leaked value would be an actual credential, and the boot
    // failure is loud by design — panicked straight to the deployment log by all four entrypoints
    // (`api/axum.rs`, `api/mcp.rs`, `api/internal.rs`, `temper-api/src/main.rs`, each
    // `unwrap_or_else(|e| panic!("refusing to start: {e}"))`), which is precisely a log sink the
    // ApiConfig `Debug` impl above goes to some length to keep secrets out of.
    //
    // The collided pair is deliberately the two vars the message's *prose* never mentions, so this
    // asserts the `{0}`/`{1}` interpolation and cannot be satisfied by the fixed explanatory text.
    #[test]
    fn a_collision_error_names_both_variables_and_never_the_value() {
        let leaked = "s3cret-shared-by-copy-paste";
        let err = check_secret_distinctness(env(&[
            ("INTERNAL_RECONCILE_SECRET", leaked),
            ("EMBED_DISPATCH_SECRET", leaked),
        ]))
        .unwrap_err();
        let msg = err.to_string();

        assert!(
            msg.contains("INTERNAL_RECONCILE_SECRET"),
            "must name the offending var: {msg}"
        );
        assert!(
            msg.contains("EMBED_DISPATCH_SECRET"),
            "must name the relation's other side: {msg}"
        );
        assert!(!msg.contains(leaked), "must NEVER print a secret: {msg}");
    }

    // --- and that it is actually WIRED into the boot path ---
    //
    // The three above test a pure function; these two prove `from_lookup` calls it. A correct
    // predicate nothing invokes is the failure mode that motivated this task in the first place.

    // FAILS IF: `from_lookup` builds a config for a deployment whose privilege split has collapsed.
    #[test]
    fn from_lookup_refuses_a_deployment_with_colliding_secrets() {
        let mut secrets = distinct_secrets();
        for entry in secrets.iter_mut() {
            if entry.0 == "SLACK_MINT_SECRET" {
                entry.1 = "shared-by-copy-paste".to_string();
            }
            if entry.0 == "SLACK_LINK_SECRET" {
                entry.1 = "shared-by-copy-paste".to_string();
            }
        }
        let pairs = with_secrets(&secrets);

        assert_eq!(
            ApiConfig::from_lookup(lookup_of(&pairs)).map(|_| ()),
            Err(ConfigError::SecretCollision(
                "SLACK_LINK_SECRET",
                "SLACK_MINT_SECRET"
            )),
        );
    }

    // FAILS IF: the check turned a correctly-configured deployment into a boot failure.
    #[test]
    fn from_lookup_accepts_a_deployment_with_distinct_secrets() {
        let pairs = with_secrets(&distinct_secrets());
        assert!(ApiConfig::from_lookup(lookup_of(&pairs)).is_ok());
    }
}
