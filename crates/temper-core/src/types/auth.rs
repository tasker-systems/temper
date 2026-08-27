use serde::{Deserialize, Serialize};

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

/// Whether the authenticated principal is a human (interactive OAuth) or a
/// machine (M2M `client_credentials`). The normalizer sets this; the profile
/// resolver branches on it. A typed discriminant — never a stringly-typed
/// provider-string match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    Human,
    Machine,
}

/// JWT claims extracted from any supported identity provider.
///
/// Parsed during middleware verification. The `external_user_id` is the value
/// of the configured `user_id_claim` from the JWT, used to look up the
/// corresponding `ProfileAuthLink`.
#[derive(Debug, Clone)]
pub struct AuthClaims {
    /// Human (interactive) vs machine (M2M) principal.
    pub principal_kind: PrincipalKind,
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

// `AuthenticatedProfile` used to live here, with public fields. It moved to
// `temper_services::auth` so it could be SEALED: its only legitimate constructor is the Level-1
// gate, which lives in temper-services, and a type cannot have private fields in one crate and be
// built in another. Keeping it here meant every crate in the workspace could forge proof of
// authentication by struct literal — the enforcement its doc comment claimed but did not have.
//
// `AuthClaims` and `Profile` stay here on purpose: they are shared data, not proof of anything.
// A forged `AuthClaims` is inert (see the seam's module docs) precisely because it has nowhere
// to go once the thing that *carries authority* is out of reach.

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
    /// The asserted group values, or `None` when the assertion carried **no group signal**.
    ///
    /// Three states, and the middle one is the one a `Vec<String>` could not express: `None` (the
    /// IdP has no `groups_attr` configured, or the attribute was absent from this assertion),
    /// `Some([])` (the provider named this principal's groups and there are none), and
    /// `Some([..])`. The first must never revoke reach; the second must. Mirrors the AS-side
    /// `extractGroups(): string[] | null` (`packages/temper-cloud/src/saml/sp.ts:63-77`) exactly,
    /// so the distinction survives the wire instead of being collapsed at it.
    ///
    /// NOTE on a serde subtlety worth knowing rather than rediscovering: an `Option` field that is
    /// **omitted** deserializes to `None`, so a caller that drops `groups` entirely gets the
    /// no-signal path rather than a rejection. That direction is the safe one — it declines to
    /// revoke — and it is no longer silent, because the no-signal path writes
    /// `kb_saml_principal_reconcile.last_skipped_at`. The AS always sends the key
    /// (`packages/temper-cloud/src/oauth/reconcile.ts`), pinned by the wire-contract test.
    pub groups: Option<Vec<String>>,
}

/// Wire payload for the internal principal-resolve call (AS → temper-api).
///
/// The AS holds a token `sub` and needs the profile it will resolve to, so it can stamp the
/// refresh-token row with an owner an administrator can later revoke through. It cannot do that
/// lookup itself: the authoritative provider is temper-api's `auth_provider_name`, and a second
/// copy of that value in the AS's environment would drift silently — no link matched, nothing
/// revoked, nothing said. So the resolution stays on the side that owns the config.
///
/// Deliberately NOT folded into [`ReconcileRequest`]. That call is skipped entirely when an
/// assertion carries no group signal, and its `groups: Vec<String>` is the wire form of the
/// absence-vs-empty distinction the whole SAML design turns on. Widening it to carry a second
/// purpose would put that distinction under pressure it does not need to be under.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "ResolvePrincipalRequest.ts")
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePrincipalRequest {
    /// Stable NameID — the same value minted as the token `sub`.
    pub external_user_id: String,
    /// Email attribute from the assertion.
    pub email: String,
    /// Verified flag (a signed trusted-IdP assertion is treated as verified).
    pub email_verified: Option<bool>,
}

/// The profile an [`ResolvePrincipalRequest`] resolved to.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "ResolvePrincipalResponse.ts")
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePrincipalResponse {
    /// The resolved (or just-provisioned) profile id.
    pub profile_id: uuid::Uuid,
}

#[cfg(test)]
mod tests {
    use super::ReconcileRequest;

    fn parse(groups_field: &str) -> ReconcileRequest {
        let json = format!(
            r#"{{"provider":"saml:acme","external_user_id":"nid","email":"a@b.io",
                 "email_verified":true,"idp_key":"acme"{groups_field}}}"#
        );
        serde_json::from_str(&json).expect("a well-formed reconcile payload")
    }

    /// The three group states survive the wire as three states.
    ///
    /// `[]` and `null` are the pair that matters: they are opposite instructions — "revoke what the
    /// provider no longer confers" and "the provider said nothing, act on nothing" — and a payload
    /// that collapsed them would revoke reach on a transient attribute drop.
    #[test]
    fn null_groups_and_empty_groups_are_different_payloads() {
        assert_eq!(parse(r#","groups":null"#).groups, None);
        assert_eq!(parse(r#","groups":[]"#).groups, Some(vec![]));
        assert_eq!(
            parse(r#","groups":["eng"]"#).groups,
            Some(vec!["eng".to_string()])
        );
    }

    /// An OMITTED `groups` key deserializes to `None`, not to an error.
    ///
    /// Asserted rather than assumed, because it is a genuine serde subtlety and the wrong belief
    /// about it is load-bearing: a reader who thinks a missing key is rejected would conclude that
    /// only an explicit `null` can reach the no-signal path. It cannot be rejected here without a
    /// custom deserializer, and it does not need to be — the direction is the safe one (decline to
    /// revoke) and it is no longer silent, since that path records `last_skipped_at`.
    #[test]
    fn an_omitted_groups_key_is_read_as_no_signal_rather_than_refused() {
        assert_eq!(parse("").groups, None);
    }
}
