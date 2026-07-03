//! Shared machine-token claim normalization. The single place that decides
//! whether a decoded JWT is a machine (M2M `client_credentials`) principal and,
//! if so, produces normalized `AuthClaims`. Both surfaces decode into
//! `RawJwtClaims` and call `normalize_machine`; the human branch stays
//! per-surface (email resolution differs by surface).

use serde::Deserialize;

use temper_core::types::{AuthClaims, PrincipalKind};

/// Link-namespace provider tag for Auth0 M2M agent principals. Distinct from the
/// human `auth0` namespace so `(auth0-m2m, client_id)` never collides with a
/// human `(auth0, sub)` under the `UNIQUE(auth_provider, auth_provider_user_id)`
/// constraint.
pub const MACHINE_PROVIDER_TAG: &str = "auth0-m2m";

/// Superset of JWT claims both surfaces decode into. Optional fields absorb the
/// human/machine shape difference in one struct.
#[derive(Debug, Clone, Deserialize)]
pub struct RawJwtClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
    /// Authorized party (the client id). Present on Auth0 human AND machine
    /// tokens — NOT a machine signal on its own.
    #[serde(default)]
    pub azp: Option<String>,
    /// Grant-type marker. `client-credentials` is the definitive machine signal.
    #[serde(default)]
    pub gty: Option<String>,
    pub exp: i64,
    #[serde(default)]
    pub iat: i64,
}

/// If `raw` is a machine (`client_credentials`) token, return normalized machine
/// `AuthClaims`; otherwise `None` (caller handles the human branch).
///
/// Detection is on `gty`, never `azp` presence. Client-id source: `azp` primary,
/// `sub` `@clients`-suffix strip as fallback.
pub fn normalize_machine(raw: &RawJwtClaims) -> Option<AuthClaims> {
    if raw.gty.as_deref() != Some("client-credentials") {
        return None;
    }
    let client_id = raw
        .azp
        .clone()
        .or_else(|| raw.sub.strip_suffix("@clients").map(str::to_string))?;
    Some(AuthClaims {
        principal_kind: PrincipalKind::Machine,
        provider: MACHINE_PROVIDER_TAG.to_string(),
        external_user_id: client_id,
        email: String::new(),
        email_verified: None,
        exp: raw.exp,
        iat: raw.iat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(gty: Option<&str>, azp: Option<&str>, sub: &str, email: Option<&str>) -> RawJwtClaims {
        RawJwtClaims {
            sub: sub.to_string(),
            email: email.map(str::to_string),
            email_verified: None,
            azp: azp.map(str::to_string),
            gty: gty.map(str::to_string),
            exp: 9999,
            iat: 1111,
        }
    }

    #[test]
    fn machine_token_via_azp() {
        let c = normalize_machine(&raw(
            Some("client-credentials"),
            Some("abc123"),
            "abc123@clients",
            None,
        ))
        .expect("should detect machine");
        assert_eq!(c.principal_kind, PrincipalKind::Machine);
        assert_eq!(c.provider, "auth0-m2m");
        assert_eq!(c.external_user_id, "abc123"); // azp preferred
        assert_eq!(c.exp, 9999);
        assert_eq!(c.iat, 1111);
    }

    #[test]
    fn machine_token_sub_strip_fallback_when_azp_absent() {
        let c = normalize_machine(&raw(
            Some("client-credentials"),
            None,
            "abc123@clients",
            None,
        ))
        .expect("should detect machine via sub strip");
        assert_eq!(c.external_user_id, "abc123");
    }

    #[test]
    fn human_token_with_azp_is_not_machine() {
        // The critical guard: a human authorization_code token also carries azp.
        assert!(normalize_machine(&raw(
            Some("authorization_code"),
            Some("abc123"),
            "auth0|user",
            Some("u@example.test"),
        ))
        .is_none());
    }

    #[test]
    fn human_token_without_gty_is_not_machine() {
        assert!(
            normalize_machine(&raw(None, Some("abc123"), "auth0|user", Some("u@x.test"))).is_none()
        );
    }

    /// Known-answer test pinning the *real* claim shape Auth0 mints for a
    /// `client_credentials` grant. Captured from a live token minted by the
    /// `Temper Steward M2M` app on `temperkb.us.auth0.com` (auth seam Stage 4
    /// validation, 2026-07-02). Guards against Auth0 silently changing its M2M
    /// token format under us.
    #[test]
    fn real_auth0_m2m_token_shape_is_detected() {
        let raw = RawJwtClaims {
            sub: "y23AQxuvzjYSb5n8lAUeuIgIXOftCWYu@clients".to_string(),
            email: None,
            email_verified: None,
            azp: Some("y23AQxuvzjYSb5n8lAUeuIgIXOftCWYu".to_string()),
            gty: Some("client-credentials".to_string()),
            exp: 1_783_126_372,
            iat: 1_783_039_972,
        };
        let c = normalize_machine(&raw).expect("real Auth0 M2M token must be detected as machine");
        assert_eq!(c.principal_kind, PrincipalKind::Machine);
        assert_eq!(c.provider, "auth0-m2m");
        assert_eq!(c.external_user_id, "y23AQxuvzjYSb5n8lAUeuIgIXOftCWYu");
    }
}
