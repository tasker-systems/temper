use serde::{Deserialize, Serialize};

use super::profile::Profile;

/// Identity provider configuration — Neon Auth default, swappable for enterprise.
///
/// The provider is configuration, not code. The JWT verification middleware
/// is parameterized by `AuthProvider`, not specialized per provider.
/// Neon Auth uses EdDSA (Ed25519) with `sub`. Auth0/Okta use RS256 with `sub`.
#[derive(Debug, Clone)]
pub struct AuthProvider {
    /// Provider identifier: "neon_auth", "auth0", "okta", etc.
    pub name: String,
    /// JWKS endpoint for key discovery (e.g., `{base_url}/.well-known/jwks.json`)
    pub jwks_url: String,
    /// Expected `iss` claim in JWTs
    pub issuer: String,
    /// Expected `aud` claim, if the provider uses it
    pub audience: Option<String>,
    /// Which JWT claim holds the external user ID (usually "sub")
    pub user_id_claim: String,
}

/// JWT claims extracted from any supported identity provider.
///
/// Parsed during middleware verification. The `external_user_id` is the value
/// of the configured `user_id_claim` from the JWT, used to look up the
/// corresponding `ProfileAuthLink`.
#[derive(Debug, Clone)]
pub struct AuthClaims {
    /// Which provider issued this token
    pub provider: String,
    /// External user ID (value of the configured `user_id_claim`)
    pub external_user_id: String,
    /// User's email from token claims
    pub email: String,
    /// Whether the identity provider has verified the user's email.
    /// `None` means the provider didn't include the claim.
    pub email_verified: Option<bool>,
    /// Token expiry (Unix timestamp)
    pub exp: i64,
    /// Token issued-at (Unix timestamp)
    pub iat: i64,
}

/// The authenticated identity for the current request.
///
/// Extracted by axum middleware via JWT verification → auth link lookup → profile load.
/// Available to all route handlers as an axum extractor.
#[derive(Debug, Clone)]
pub struct AuthenticatedProfile {
    pub profile: Profile,
    pub claims: AuthClaims,
}

/// Wire payload for the internal SAML membership-reconcile call (AS → temper-api).
///
/// `provider` is advisory: the API derives the authoritative provider from its own
/// config (`auth_provider_name`) so the resolved profile matches the one the minted
/// token resolves to. `idp_key` selects the `kb_saml_group_mappings` rows to apply.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "ReconcileRequest.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileRequest {
    /// Advisory provider label (e.g. "saml:acme-okta"); the API ignores it for identity.
    pub provider: Option<String>,
    /// Stable NameID — the same value minted as the token `sub`.
    pub external_user_id: String,
    /// Email attribute from the assertion.
    pub email: String,
    /// Verified flag (a signed trusted-IdP assertion is treated as verified).
    pub email_verified: Option<bool>,
    /// Which IdP's group mappings to apply.
    pub idp_key: String,
    /// Asserted group values (possibly empty).
    pub groups: Vec<String>,
}
