//! The one place an instance's auth identity is parsed.
//!
//! Four environment variables have to agree — `AUTH_AUDIENCE`, `MCP_AUDIENCE`, `AS_AUDIENCE`,
//! `AS_ISSUER` (plus `AUTH_ISSUER` and `JWKS_URL`, which in AS mode must point at that same AS) —
//! and before this module nothing made them. Worse, `AUTH_AUDIENCE` resolved empty-or-unset to
//! `None`, and `None` disabled JWT audience validation outright.
//!
//! So `audience` here is a `String`, never an `Option<String>`. The fall-open branch has nothing
//! left to branch on: it is *unconstructible*, not merely forbidden.
//!
//! The coherence rules below are **not new policy**. The temper AS mints every token — human and
//! machine — with a single `AS_AUDIENCE`, so a working AS instance already satisfies all of them:
//! a divergent audience verifies nothing, a divergent issuer trusts the wrong party, and a
//! misdirected `JWKS_URL` checks no signature. We name rules that were already true, and fail fast
//! when they are not.

use std::fmt;

/// Which issuer fronts this instance. Derived from `AS_ISSUER`'s presence — that variable being set
/// *is* the AS-mode signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// An external IdP (Auth0) mints tokens; temper is a pure resource server.
    ExternalIdp,
    /// Temper's own authorization server mints tokens.
    TemperAs,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExternalIdp => write!(f, "external-IdP"),
            Self::TemperAs => write!(f, "temper-AS"),
        }
    }
}

/// An instance's verified auth identity. Both surfaces read this same value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub issuer: String,
    pub jwks_url: String,
    /// The one audience this instance validates, on both surfaces. Never optional.
    pub audience: String,
    pub mode: AuthMode,
}

/// A boot-blocking configuration fault.
///
/// Every message names the offending environment variable and states the relation it must satisfy.
/// **No message ever prints a value** — anyone who can act on the error can already read them, and a
/// config value in a log is a liability with no upside.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Missing(&'static str),
    MissingAudience,
    McpAudienceMismatch,
    AsAudienceMismatch,
    AsIssuerMismatch,
    AsJwksMismatch,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(var) => write!(f, "{var} is not set."),
            Self::MissingAudience => write!(
                f,
                "AUTH_AUDIENCE is not set. Both surfaces validate the `aud` claim; set it to the \
                 API identifier your IdP mints tokens for."
            ),
            Self::McpAudienceMismatch => write!(
                f,
                "MCP_AUDIENCE is set but does not equal AUTH_AUDIENCE. This instance validates one \
                 audience on both surfaces. Set them to the same value, or unset MCP_AUDIENCE."
            ),
            Self::AsAudienceMismatch => write!(
                f,
                "AS_ISSUER is set, so this instance mints its own tokens — but AS_AUDIENCE does not \
                 equal AUTH_AUDIENCE. The authorization server mints every token with AS_AUDIENCE \
                 and the API validates AUTH_AUDIENCE. Set them to the same value."
            ),
            Self::AsIssuerMismatch => write!(
                f,
                "AS_ISSUER is set, but AUTH_ISSUER does not equal it. The API must trust the \
                 authorization server it fronts. Set AUTH_ISSUER to the same value as AS_ISSUER."
            ),
            Self::AsJwksMismatch => write!(
                f,
                "AS_ISSUER is set, but JWKS_URL does not point at this instance's authorization \
                 server. Set JWKS_URL to $AS_ISSUER/oauth/jwks."
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Strip trailing slashes. Auth0 issuers conventionally end in `/` and the AS's own metadata
/// already strips them, so a raw string compare would false-positive.
fn norm(s: &str) -> &str {
    s.trim_end_matches('/')
}

/// Read a variable, treating whitespace-only and empty as absent — uniformly, for every variable.
/// Before this, an empty `AUTH_AUDIENCE` disabled validation on temper-api while an empty
/// `MCP_AUDIENCE` made temper-mcp reject every token. One typo, two opposite failures.
fn get(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Parse and verify an instance's auth identity, or refuse to produce one.
pub fn parse_auth_config(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<AuthConfig, ConfigError> {
    let issuer = get(&lookup, "AUTH_ISSUER").ok_or(ConfigError::Missing("AUTH_ISSUER"))?;
    let jwks_url = get(&lookup, "JWKS_URL").ok_or(ConfigError::Missing("JWKS_URL"))?;
    let audience = get(&lookup, "AUTH_AUDIENCE").ok_or(ConfigError::MissingAudience)?;

    // MCP_AUDIENCE is no longer a *value* — it is an agreement assertion. An instance has exactly
    // one audience; this variable may only restate it.
    if let Some(mcp_audience) = get(&lookup, "MCP_AUDIENCE") {
        if mcp_audience != audience {
            return Err(ConfigError::McpAudienceMismatch);
        }
    }

    let mode = match get(&lookup, "AS_ISSUER") {
        None => AuthMode::ExternalIdp,
        Some(as_issuer) => {
            let as_audience =
                get(&lookup, "AS_AUDIENCE").ok_or(ConfigError::Missing("AS_AUDIENCE"))?;
            if as_audience != audience {
                return Err(ConfigError::AsAudienceMismatch);
            }
            if norm(&as_issuer) != norm(&issuer) {
                return Err(ConfigError::AsIssuerMismatch);
            }
            if norm(&jwks_url) != format!("{}/oauth/jwks", norm(&as_issuer)) {
                return Err(ConfigError::AsJwksMismatch);
            }
            AuthMode::TemperAs
        }
    };

    Ok(AuthConfig {
        issuer,
        jwks_url,
        audience,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a lookup from pairs. Absent keys return None.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    /// A valid external-IdP instance: exactly today's production shape.
    fn external_idp() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AUTH_ISSUER", "https://tenant.auth0.com/"),
            ("JWKS_URL", "https://tenant.auth0.com/.well-known/jwks.json"),
            ("AUTH_AUDIENCE", "https://temperkb.io/api"),
        ]
    }

    /// A valid AS-mode instance: every audience collapses to one value.
    fn temper_as() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AUTH_ISSUER", "https://temper.acme.com"),
            ("JWKS_URL", "https://temper.acme.com/oauth/jwks"),
            ("AUTH_AUDIENCE", "https://temper.acme.com/api"),
            ("AS_ISSUER", "https://temper.acme.com"),
            ("AS_AUDIENCE", "https://temper.acme.com/api"),
        ]
    }

    fn with(
        base: Vec<(&'static str, &'static str)>,
        k: &'static str,
        v: &'static str,
    ) -> Vec<(&'static str, &'static str)> {
        let mut out: Vec<_> = base.into_iter().filter(|(key, _)| *key != k).collect();
        out.push((k, v));
        out
    }

    fn without(
        base: Vec<(&'static str, &'static str)>,
        k: &'static str,
    ) -> Vec<(&'static str, &'static str)> {
        base.into_iter().filter(|(key, _)| *key != k).collect()
    }

    // --- the happy paths, which pin the two live deployments ---

    #[test]
    fn external_idp_instance_parses_and_is_external_mode() {
        let cfg = parse_auth_config(env(&external_idp())).expect("valid external-IdP config");
        assert_eq!(cfg.mode, AuthMode::ExternalIdp);
        assert_eq!(cfg.audience, "https://temperkb.io/api");
    }

    #[test]
    fn as_instance_parses_and_is_as_mode() {
        let cfg = parse_auth_config(env(&temper_as())).expect("valid AS config");
        assert_eq!(cfg.mode, AuthMode::TemperAs);
    }

    #[test]
    fn mcp_audience_equal_to_auth_audience_is_accepted() {
        // The current live shape on BOTH deployments.
        let e = with(external_idp(), "MCP_AUDIENCE", "https://temperkb.io/api");
        assert!(parse_auth_config(env(&e)).is_ok());
    }

    #[test]
    fn mcp_audience_unset_is_normal_not_a_fallback() {
        // The instance simply has its one audience. Absence is correct, not degraded.
        let cfg = parse_auth_config(env(&external_idp())).expect("valid");
        assert_eq!(cfg.audience, "https://temperkb.io/api");
    }

    // --- the security regression: this is the bug being closed ---

    #[test]
    fn missing_audience_is_refused() {
        let e = without(external_idp(), "AUTH_AUDIENCE");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::MissingAudience)
        );
    }

    #[test]
    fn empty_audience_is_refused_not_treated_as_disabled() {
        // Before this change, an empty value resolved to None and DISABLED audience validation.
        let e = with(external_idp(), "AUTH_AUDIENCE", "");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::MissingAudience)
        );
    }

    // --- one bite test per rule: each violates exactly that rule and nothing else ---

    #[test]
    fn divergent_mcp_audience_is_refused() {
        let e = with(external_idp(), "MCP_AUDIENCE", "https://other.example/api");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::McpAudienceMismatch)
        );
    }

    #[test]
    fn as_mode_divergent_as_audience_is_refused() {
        let e = with(temper_as(), "AS_AUDIENCE", "https://other.example/api");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::AsAudienceMismatch)
        );
    }

    #[test]
    fn as_mode_divergent_auth_issuer_is_refused() {
        let e = with(temper_as(), "AUTH_ISSUER", "https://tenant.auth0.com/");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::AsIssuerMismatch)
        );
    }

    #[test]
    fn as_mode_jwks_pointing_elsewhere_is_refused() {
        let e = with(
            temper_as(),
            "JWKS_URL",
            "https://tenant.auth0.com/.well-known/jwks.json",
        );
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::AsJwksMismatch));
    }

    #[test]
    fn as_mode_requires_as_audience() {
        let e = without(temper_as(), "AS_AUDIENCE");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::Missing("AS_AUDIENCE"))
        );
    }

    // --- normalization ---

    #[test]
    fn trailing_slashes_are_normalized_before_comparison() {
        // AS_ISSUER with a trailing slash, AUTH_ISSUER without: the same instance.
        let e = with(temper_as(), "AS_ISSUER", "https://temper.acme.com/");
        let cfg = parse_auth_config(env(&e)).expect("trailing slash must not be a mismatch");
        assert_eq!(cfg.mode, AuthMode::TemperAs);
    }

    #[test]
    fn a_jwks_url_with_a_trailing_slash_still_matches() {
        let e = with(
            temper_as(),
            "JWKS_URL",
            "https://temper.acme.com/oauth/jwks/",
        );
        assert!(parse_auth_config(env(&e)).is_ok());
    }

    // --- errors are actionable but leak nothing ---

    #[test]
    fn errors_name_the_variable_and_never_print_values() {
        let e = with(
            external_idp(),
            "MCP_AUDIENCE",
            "https://secret-enterprise.internal/api",
        );
        let err = parse_auth_config(env(&e)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("MCP_AUDIENCE"), "must name the offending var");
        assert!(
            msg.contains("AUTH_AUDIENCE"),
            "must name the relation's other side"
        );
        assert!(
            !msg.contains("secret-enterprise.internal"),
            "must NEVER print a config value: {msg}"
        );
    }
}
