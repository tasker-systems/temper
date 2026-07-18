//! Mode-aware OAuth endpoint derivation for the Slack account-link flow.
//!
//! Temper is an OAuth *client* here, of whichever issuer fronts this instance:
//! Auth0 on temperkb.io, the co-deployed AS on an enterprise install. `AuthConfig.mode`
//! already carries which, so this is derivation, not new configuration.

use crate::auth_config::{AuthConfig, AuthMode};
use crate::config::SlackLinkConfig;

/// The resolved endpoints for one link flow.
#[derive(Debug, Clone)]
pub struct LinkProvider {
    pub authorize_url: String,
    pub token_url: String,
    /// RFC 7009 token revocation. See [`fn@derive`] for what this means per mode.
    pub revoke_url: String,
    pub client_id: String,
    pub redirect_uri: String,
}

/// The callback path. Public (browser-facing), so it is served by the axum function via
/// vercel.json's `/(.*)` catch-all — the `filesystem` handler finds no file at this path.
pub const CALLBACK_PATH: &str = "/api/auth/slack/callback";

/// Derive the endpoints from the instance's auth identity.
///
/// AS mode mirrors `temper-cli`'s `Idp::TemperAs`: the endpoints live on the instance
/// itself rather than a separate auth host, so temper-api exchanges against its own
/// deployment's `/oauth/token`. That self-hop is not a wart — it is what any OAuth client
/// colocated with its AS does, and it keeps ONE code path across both modes.
pub fn derive(auth: &AuthConfig, cfg: &SlackLinkConfig) -> LinkProvider {
    let base = auth.issuer.trim_end_matches('/');

    let (authorize_url, token_url, revoke_url) = match auth.mode {
        AuthMode::ExternalIdp => (
            format!("{base}/authorize"),
            format!("{base}/oauth/token"),
            // Auth0's RFC 7009 endpoint.
            format!("{base}/oauth/revoke"),
        ),
        AuthMode::TemperAs => (
            format!("{base}/oauth/authorize"),
            format!("{base}/oauth/token"),
            // The AS has NO revocation endpoint today. This value is never
            // dialled: `slack_disconnect_service` revokes AS grants locally, as
            // a row update in its own transaction. We still produce a
            // well-formed URL on the same path shape rather than an `Option` the
            // caller would have to unwrap for a case that cannot arise. If the
            // AS ever grows a real `/oauth/revoke`, this is already pointing at
            // it — but until then, do NOT route an AS disconnect through HTTP.
            format!("{base}/oauth/revoke"),
        ),
    };

    LinkProvider {
        authorize_url,
        token_url,
        revoke_url,
        client_id: cfg.client_id.clone(),
        redirect_uri: format!(
            "{}{CALLBACK_PATH}",
            cfg.public_base_url.trim_end_matches('/')
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_config::{AuthConfig, AuthMode};

    fn slack_cfg() -> crate::config::SlackLinkConfig {
        use base64::Engine as _;
        crate::config::SlackLinkConfig {
            client_id: "slack-link-client".to_string(),
            hmac_secret: "s3cret".to_string(),
            public_base_url: "https://temperkb.io".to_string(),
            vault_key: crate::services::grant_crypto::VaultKey::from_base64(
                &base64::engine::general_purpose::STANDARD.encode([0u8; 32]),
            )
            .unwrap(),
        }
    }

    fn auth(issuer: &str, mode: AuthMode) -> AuthConfig {
        AuthConfig {
            issuer: issuer.to_string(),
            jwks_url: "https://unused/.well-known/jwks.json".to_string(),
            audience: "https://api.temperkb.io".to_string(),
            mode,
        }
    }

    #[test]
    fn external_idp_points_at_the_idp_domain() {
        let p = derive(
            &auth("https://temperkb.us.auth0.com/", AuthMode::ExternalIdp),
            &slack_cfg(),
        );
        assert_eq!(p.authorize_url, "https://temperkb.us.auth0.com/authorize");
        assert_eq!(p.token_url, "https://temperkb.us.auth0.com/oauth/token");
        assert_eq!(p.revoke_url, "https://temperkb.us.auth0.com/oauth/revoke");
    }

    /// A trailing slash on the issuer must not produce a doubled one.
    #[test]
    fn external_idp_tolerates_a_missing_trailing_slash() {
        let p = derive(
            &auth("https://temperkb.us.auth0.com", AuthMode::ExternalIdp),
            &slack_cfg(),
        );
        assert_eq!(p.authorize_url, "https://temperkb.us.auth0.com/authorize");
    }

    /// AS mode: the endpoints live on the instance itself, not a separate auth host.
    #[test]
    fn temper_as_points_at_the_instance_itself() {
        let p = derive(
            &auth("https://temper.acme.com", AuthMode::TemperAs),
            &slack_cfg(),
        );
        assert_eq!(p.authorize_url, "https://temper.acme.com/oauth/authorize");
        assert_eq!(p.token_url, "https://temper.acme.com/oauth/token");
        // Derived, but never dialled — AS mode revokes locally. See `derive`.
        assert_eq!(p.revoke_url, "https://temper.acme.com/oauth/revoke");
    }

    #[test]
    fn redirect_uri_is_the_public_callback() {
        let p = derive(&auth("https://x/", AuthMode::ExternalIdp), &slack_cfg());
        assert_eq!(
            p.redirect_uri,
            "https://temperkb.io/api/auth/slack/callback"
        );
    }
}
