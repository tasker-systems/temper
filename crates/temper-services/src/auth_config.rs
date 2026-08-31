//! The one place an instance's auth identity is parsed.
//!
//! The environment variables that describe it — `AUTH_AUDIENCE`, `MCP_AUDIENCE`, `AS_AUDIENCE`,
//! `AS_ISSUER` (plus `AUTH_ISSUER` and `JWKS_URL`, which in AS mode must point at that same AS) —
//! used to be related only by convention, and before this module nothing made them agree. Worse,
//! `AUTH_AUDIENCE` resolved empty-or-unset to `None`, and `None` disabled JWT audience validation
//! outright.
//!
//! So `audience` here is a `String`, never an `Option<String>`. The fall-open branch has nothing
//! left to branch on: it is *unconstructible*, not merely forbidden. The same is true of
//! `mcp_audience`: `MCP_AUDIENCE` defaults to the API audience, so an instance that never sets it
//! behaves exactly as it did when the two were required to be equal.
//!
//! The coherence rules below are **not new policy**. The temper AS mints tokens with a single
//! `AS_AUDIENCE`, so a working AS instance already satisfies them: a divergent audience verifies
//! nothing, a divergent issuer trusts the wrong party, and a misdirected `JWKS_URL` checks no
//! signature. We name rules that were already true, and fail fast when they are not.

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
    /// The audience the REST API validates. Never optional.
    pub audience: String,
    /// The RFC 8707 resource indicator of the MCP surface: what its protected-resource metadata
    /// advertises, what conformant MCP clients request, and (alongside [`Self::audience`]) what
    /// the MCP middleware accepts. `MCP_AUDIENCE` when set, else [`Self::audience`] — resolved
    /// here so no caller re-derives the fallback. Never optional, for the same reason `audience`
    /// is not.
    pub mcp_audience: String,
    pub mode: AuthMode,
}

/// A boot-blocking configuration fault.
///
/// Every message names the offending environment variable and states the relation it must satisfy.
/// **No message ever prints a value** — anyone who can act on the error can already read them, and a
/// config value in a log is a liability with no upside.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set.")]
    Missing(&'static str),

    #[error(
        "AUTH_AUDIENCE is not set. Both surfaces validate the `aud` claim; set it to the API \
         identifier your IdP mints tokens for."
    )]
    MissingAudience,

    #[error(
        "MCP_AUDIENCE must be a URI. It is advertised as the `resource` indicator in the MCP \
         surface's RFC 9728 protected-resource metadata, which requires a URI — and conformant \
         MCP clients additionally refuse a resource that is neither the MCP server URL nor its \
         origin. Use an absolute URI such as https://<instance>/mcp."
    )]
    McpAudienceNotUri,

    #[error(
        "AS_ISSUER is set, so this instance mints its own tokens — but AS_AUDIENCE does not equal \
         AUTH_AUDIENCE. The authorization server mints every token with AS_AUDIENCE and the API \
         validates AUTH_AUDIENCE. Set them to the same value."
    )]
    AsAudienceMismatch,

    #[error(
        "AS_ISSUER is set, but AUTH_ISSUER does not equal it — byte for byte. The AS mints `iss` \
         from the raw AS_ISSUER and the API matches it exactly, so even a trailing-slash difference \
         means no token it mints will ever verify. Set AUTH_ISSUER to the same value as AS_ISSUER."
    )]
    AsIssuerMismatch,

    #[error(
        "AS_ISSUER is set, but JWKS_URL does not point at this instance's authorization server. \
         Set JWKS_URL to $AS_ISSUER/oauth/jwks."
    )]
    AsJwksMismatch,

    #[error(
        "AUTH_AUDIENCE must be a URI. It is matched byte-for-byte against the token's `aud` claim \
         AND advertised as the `resource` indicator in the RFC 9728 protected-resource metadata, \
         which requires a URI. Use an absolute URI such as https://<instance>/api."
    )]
    AudienceNotUri,

    #[error(
        "{0} and {1} hold the SAME value. Each of this instance's shared secrets gates a different \
         internal capability, and their being different values is the whole security property: \
         SLACK_LINK_SECRET answers \"is this principal linked?\", SLACK_MINT_SECRET vends a token \
         carrying that human's entire reach, and SLACK_VAULT_ENC_KEY decrypts every stored grant. \
         Two of them sharing a value means whoever holds the cheap capability already holds the \
         expensive one. Give each its own: `openssl rand -base64 32`."
    )]
    SecretCollision(&'static str, &'static str),
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

    // The audience must be a URI, and that became LOAD-BEARING the moment the protected-resource
    // metadata advertised it: RFC 7519 lets a JWT `aud` be a colon-free StringOrURI, so a plain
    // string was historically a legal audience — but the RFC 9728 `resource` MUST be a URI, and
    // every surface serves that document. There is no deployment left where a plain-string
    // audience is correct; an opaque scheme (`api://default`, Okta's convention) parses fine.
    // Validated, never normalized — the RAW value feeds `Validation::set_audience`, so applying
    // the URL crate's trailing-slash repair here would silently change what every token must
    // carry. The same value minted from AS_AUDIENCE is covered transitively by the equality
    // checks below, which the MCP_AUDIENCE check already models.
    url::Url::parse(&audience).map_err(|_| ConfigError::AudienceNotUri)?;

    // MCP_AUDIENCE is the MCP surface's own RFC 8707 resource indicator: what that surface's
    // protected-resource metadata advertises, what conformant MCP clients request as `resource`,
    // and the first audience the MCP middleware accepts. It defaults to the API audience — unset,
    // the instance behaves exactly as it did when this variable was required to restate
    // AUTH_AUDIENCE. When set it is independent, and that independence is the point: MCP clients
    // validate that the advertised resource equals the MCP server URL or its origin, so a
    // dedicated audience shaped like the MCP endpoint URL (e.g. `$INSTANCE/mcp`) is the only
    // value that satisfies both the client-side check and the `aud` the AS actually mints.
    // URI-validated for the same reason AUTH_AUDIENCE is: this value is served in a document
    // that requires a URI.
    let mcp_audience = get(&lookup, "MCP_AUDIENCE").unwrap_or_else(|| audience.clone());
    url::Url::parse(&mcp_audience).map_err(|_| ConfigError::McpAudienceNotUri)?;

    let mode = match get(&lookup, "AS_ISSUER") {
        None => AuthMode::ExternalIdp,
        Some(as_issuer) => {
            let as_audience =
                get(&lookup, "AS_AUDIENCE").ok_or(ConfigError::Missing("AS_AUDIENCE"))?;
            if as_audience != audience {
                return Err(ConfigError::AsAudienceMismatch);
            }

            // BYTE-EXACT, deliberately — do NOT normalize trailing slashes here.
            //
            // The AS mints `iss` from the RAW `AS_ISSUER` (`mint.ts`: `setIssuer(requireEnv(
            // "AS_ISSUER"))`, no trimming), and we hand the RAW `AUTH_ISSUER` to
            // `Validation::set_issuer`, which matches by exact string membership. So
            // `AS_ISSUER=https://x/` with `AUTH_ISSUER=https://x` is NOT a harmless cosmetic
            // difference: it would boot green and then reject every token the AS ever mints.
            // A tolerant comparison here would admit exactly the class of broken instance this
            // module exists to refuse.
            if as_issuer != issuer {
                return Err(ConfigError::AsIssuerMismatch);
            }

            // The JWKS URL is FETCHED verbatim, so it too must be exact. The issuer may legally
            // carry a trailing slash (it is just an identifier string) — the URL derived from it
            // must not double it.
            let expected_jwks = format!("{}/oauth/jwks", as_issuer.trim_end_matches('/'));
            if jwks_url != expected_jwks {
                return Err(ConfigError::AsJwksMismatch);
            }

            AuthMode::TemperAs
        }
    };

    Ok(AuthConfig {
        issuer,
        jwks_url,
        audience,
        mcp_audience,
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
        let cfg = parse_auth_config(env(&e)).expect("equal audiences remain valid");
        assert_eq!(cfg.mcp_audience, cfg.audience);
    }

    #[test]
    fn an_empty_mcp_audience_falls_back_to_the_api_audience() {
        // Empty means absent, uniformly. This is the value that used to mean two OPPOSITE things:
        // temper-api filtered it to None and disabled validation; temper-mcp did not filter it and
        // enforced `aud == ""`, rejecting every token. One typo, one variable, two failures.
        let e = with(external_idp(), "MCP_AUDIENCE", "   ");
        let cfg = parse_auth_config(env(&e)).expect("whitespace-only is absent, not a mismatch");
        assert_eq!(
            cfg.mcp_audience, cfg.audience,
            "absent MCP_AUDIENCE resolves to the API audience — the pre-split behavior"
        );
    }

    #[test]
    fn auth_issuer_and_jwks_url_are_required() {
        assert_eq!(
            parse_auth_config(env(&without(external_idp(), "AUTH_ISSUER"))),
            Err(ConfigError::Missing("AUTH_ISSUER"))
        );
        assert_eq!(
            parse_auth_config(env(&without(external_idp(), "JWKS_URL"))),
            Err(ConfigError::Missing("JWKS_URL"))
        );
    }

    #[test]
    fn values_are_trimmed_before_use() {
        // The audience is fed straight into `Validation::set_audience`, so a stray space would
        // silently never match.
        let e = with(
            external_idp(),
            "AUTH_AUDIENCE",
            "  https://temperkb.io/api  ",
        );
        let cfg = parse_auth_config(env(&e)).expect("valid");
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

    // The protected-resource metadata advertises the audience as the RFC 9728 `resource`, which
    // must be a URI — so a non-URI audience is a boot fault, not a value that "works until
    // someone reads the metadata".
    #[test]
    fn an_audience_that_is_not_a_uri_is_refused() {
        let e = with(external_idp(), "AUTH_AUDIENCE", "temper-api");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::AudienceNotUri));
        let msg = parse_auth_config(env(&e)).unwrap_err().to_string();
        assert!(
            msg.contains("AUTH_AUDIENCE"),
            "must name the variable: {msg}"
        );
        assert!(
            !msg.contains("temper-api"),
            "must never print the value: {msg}"
        );
    }

    #[test]
    fn an_opaque_scheme_uri_audience_is_accepted() {
        // Okta's convention (`api://default`) is a URI with a non-HTTP scheme. The check exists
        // for RFC 9728 conformance, not to impose https.
        let e = with(external_idp(), "AUTH_AUDIENCE", "api://default");
        parse_auth_config(env(&e)).expect("an opaque-scheme URI is still a URI");
    }

    #[test]
    fn the_audience_is_validated_but_never_normalized() {
        // `Url::parse("https://temperkb.io").to_string()` repairs the empty path to a trailing
        // slash — so if the parsed form were stored, cfg.audience would grow one and every
        // token's byte-for-byte `aud` match would break after an upgrade. The slashless input is
        // the discriminator: it is a valid URI (accepted) yet must survive verbatim.
        let e = with(external_idp(), "AUTH_AUDIENCE", "https://temperkb.io");
        let cfg = parse_auth_config(env(&e)).expect("a pathless URI audience is valid");
        assert_eq!(
            cfg.audience, "https://temperkb.io",
            "the RAW configured value feeds set_audience — never the URL crate's normalized form"
        );
    }

    // --- one bite test per rule: each violates exactly that rule and nothing else ---

    #[test]
    fn a_distinct_mcp_audience_becomes_the_mcp_resource_audience() {
        // The dedicated-MCP-resource shape: the MCP surface carries its own RFC 8707 resource
        // indicator, independent of the API audience. This is what lets the PRM advertise a
        // value MCP clients accept (the MCP server URL) while the API keeps its own audience —
        // and no CLI, M2M, or stored config anywhere has to change.
        let e = with(external_idp(), "MCP_AUDIENCE", "https://temperkb.io/mcp");
        let cfg = parse_auth_config(env(&e)).expect("a distinct MCP audience is the feature");
        assert_eq!(cfg.mcp_audience, "https://temperkb.io/mcp");
        assert_eq!(
            cfg.audience, "https://temperkb.io/api",
            "the API audience is untouched by MCP_AUDIENCE"
        );
    }

    #[test]
    fn a_non_uri_mcp_audience_is_refused() {
        // The PRM advertises this value as the RFC 9728 `resource`, which must be a URI — the
        // same boot fault a non-URI AUTH_AUDIENCE gets.
        let e = with(external_idp(), "MCP_AUDIENCE", "temper-mcp");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::McpAudienceNotUri)
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

    // --- exactness: a trailing slash is NOT cosmetic ---

    #[test]
    fn a_trailing_slash_on_as_issuer_alone_is_refused() {
        // The subtle one, and an earlier cut of this module got it WRONG: it normalized trailing
        // slashes before comparing, so this config booted green — and then rejected every token
        // the AS minted.
        //
        // The AS mints `iss` from the raw AS_ISSUER ("https://temper.acme.com/"), while the API
        // validates against the raw AUTH_ISSUER ("https://temper.acme.com"), matched by exact
        // string. Tolerating the difference admits precisely the broken instance this module
        // exists to refuse.
        let e = with(temper_as(), "AS_ISSUER", "https://temper.acme.com/");
        assert_eq!(
            parse_auth_config(env(&e)),
            Err(ConfigError::AsIssuerMismatch)
        );
    }

    #[test]
    fn a_trailing_slash_on_both_issuers_is_fine_and_does_not_double_the_jwks_url() {
        // An issuer carrying a trailing slash is legal — it is just an identifier, and here the AS
        // and the API agree on it byte for byte. The JWKS URL derived from it must not double up.
        let e = with(temper_as(), "AS_ISSUER", "https://temper.acme.com/");
        let e = with(e, "AUTH_ISSUER", "https://temper.acme.com/");
        let cfg = parse_auth_config(env(&e)).expect("issuers agreeing byte-for-byte must be valid");
        assert_eq!(cfg.mode, AuthMode::TemperAs);
        assert_eq!(cfg.issuer, "https://temper.acme.com/");
        assert_eq!(cfg.jwks_url, "https://temper.acme.com/oauth/jwks");
    }

    #[test]
    fn a_jwks_url_with_a_trailing_slash_is_refused() {
        // The JWKS URL is fetched verbatim; a trailing slash makes it a different URL.
        let e = with(
            temper_as(),
            "JWKS_URL",
            "https://temper.acme.com/oauth/jwks/",
        );
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::AsJwksMismatch));
    }

    // --- errors are actionable but leak nothing ---

    #[test]
    fn errors_name_the_variable_and_never_print_values() {
        let e = with(
            external_idp(),
            "MCP_AUDIENCE",
            "not-a-uri-secret-enterprise.internal",
        );
        let err = parse_auth_config(env(&e)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("MCP_AUDIENCE"), "must name the offending var");
        assert!(
            !msg.contains("secret-enterprise.internal"),
            "must NEVER print a config value: {msg}"
        );
    }
}
