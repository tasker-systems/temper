//! Shared JWT claim classification. The single place that decides *what kind of
//! principal* a decoded JWT is. Both surfaces decode into `RawJwtClaims` and call
//! [`classify`]; the human branch's email resolution stays per-surface (it differs
//! by surface), but the routing decision does not.
//!
//! The return type is a **closed sum** — [`Principal`] — precisely so that no
//! surface can express "unrecognized ⇒ human". An `Option<AuthClaims>` return let
//! each caller write `if let Some(machine) { … } else { …human… }`, and that `else`
//! is a *default arm*: every token the classifier failed to recognize — including a
//! token that declares itself a machine but whose client id we cannot derive, and a
//! machine-shaped `…@clients` token with no `gty` — silently became a human. On
//! temper-api that fell closed only by accident (the human path must resolve an
//! email, which an M2M token cannot supply); on temper-mcp there is no email step,
//! so the same token auto-provisioned a human profile and walked straight past the
//! `kb_machine_clients` registration gate. Same seam, opposite outcomes: exactly the
//! cross-surface drift a default arm produces.
//!
//! With a three-variant enum there is no default arm to fall into. Refusal is a
//! value the compiler forces every surface — including surfaces not yet written —
//! to handle, and each maps it to its own transport's unauthorized.

use serde::Deserialize;

use temper_core::types::{AuthClaims, PrincipalKind};

/// Link-namespace provider tag for Auth0 M2M agent principals. Distinct from the
/// human `auth0` namespace so `(auth0-m2m, client_id)` never collides with a
/// human `(auth0, sub)` under the `UNIQUE(auth_provider, auth_provider_user_id)`
/// constraint.
pub const MACHINE_PROVIDER_TAG: &str = "auth0-m2m";

/// The `sub` suffix Auth0 (and our own minting path) puts on `client_credentials`
/// subjects: `<client_id>@clients`. A *shape*, never the definitive signal — `gty`
/// is that — but a token wearing this shape without the signal is not something we
/// are willing to guess about.
const CLIENTS_SUB_SUFFIX: &str = "@clients";

/// The definitive machine grant-type marker.
const CLIENT_CREDENTIALS_GTY: &str = "client-credentials";

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

/// What kind of principal a verified token names. Total over `RawJwtClaims`: every
/// token is exactly one of these, and there is no "other" — see the module doc for
/// why that closure is the whole point.
#[derive(Debug, Clone)]
pub enum Principal {
    /// A registered-or-not machine (`client_credentials`) principal, normalized.
    /// Registration is checked downstream by the machine gate; classification only
    /// says *which* gate the token must face.
    Machine(AuthClaims),
    /// An ordinary human token. The surface completes the claims (email resolution
    /// differs per surface) and runs the human path.
    Human,
    /// The token is machine-shaped but not coherently so, and guessing either way
    /// would be a security decision. Carries the reason; each surface maps it to
    /// its transport's unauthorized.
    Refuse(&'static str),
}

/// Classify verified JWT claims into a [`Principal`].
///
/// `gty == "client-credentials"` is the definitive machine signal — never `azp`
/// presence, which human `authorization_code` tokens also carry. Client-id source:
/// `azp` primary, the `@clients`-suffix strip of `sub` as fallback.
///
/// The two refusals are the fall-open paths this function exists to close:
/// a token that *declares* itself a machine but from which no client id can be
/// derived, and a token wearing the `@clients` subject shape without declaring the
/// grant. Neither is mintable by Auth0 or by our own `mint.ts` today; both were
/// silently reclassified as human before, so the defense was coincidental rather
/// than designed.
pub fn classify(raw: &RawJwtClaims) -> Principal {
    let declares_machine_grant = raw.gty.as_deref() == Some(CLIENT_CREDENTIALS_GTY);
    let machine_shaped_sub = raw.sub.ends_with(CLIENTS_SUB_SUFFIX);

    if declares_machine_grant {
        let Some(client_id) = raw
            .azp
            .clone()
            .or_else(|| raw.sub.strip_suffix(CLIENTS_SUB_SUFFIX).map(str::to_string))
        else {
            return Principal::Refuse(
                "client_credentials token carries no derivable client id (no azp, and sub is not @clients-suffixed)",
            );
        };
        return Principal::Machine(AuthClaims {
            principal_kind: PrincipalKind::Machine,
            provider: MACHINE_PROVIDER_TAG.to_string(),
            external_user_id: client_id,
            email: String::new(),
            email_verified: None,
            exp: raw.exp,
            iat: raw.iat,
        });
    }

    if machine_shaped_sub {
        return Principal::Refuse(
            "token subject is machine-shaped (@clients) but does not declare the client_credentials grant",
        );
    }

    Principal::Human
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

    /// Unwrap a [`Principal::Machine`], panicking with the actual variant otherwise.
    fn expect_machine(p: Principal) -> AuthClaims {
        match p {
            Principal::Machine(c) => c,
            other => panic!("expected Machine, got {other:?}"),
        }
    }

    #[test]
    fn machine_token_via_azp() {
        let c = expect_machine(classify(&raw(
            Some("client-credentials"),
            Some("abc123"),
            "abc123@clients",
            None,
        )));
        assert_eq!(c.principal_kind, PrincipalKind::Machine);
        assert_eq!(c.provider, "auth0-m2m");
        assert_eq!(c.external_user_id, "abc123"); // azp preferred
        assert_eq!(c.exp, 9999);
        assert_eq!(c.iat, 1111);
    }

    #[test]
    fn machine_token_sub_strip_fallback_when_azp_absent() {
        let c = expect_machine(classify(&raw(
            Some("client-credentials"),
            None,
            "abc123@clients",
            None,
        )));
        assert_eq!(c.external_user_id, "abc123");
    }

    #[test]
    fn human_token_with_azp_is_not_machine() {
        // The critical guard: a human authorization_code token also carries azp.
        let p = classify(&raw(
            Some("authorization_code"),
            Some("abc123"),
            "auth0|user",
            Some("u@example.test"),
        ));
        assert!(
            matches!(p, Principal::Human),
            "a human authorization_code token carries azp too; it is not a machine, got {p:?}"
        );
    }

    #[test]
    fn human_token_without_gty_is_human() {
        let p = classify(&raw(None, Some("abc123"), "auth0|user", Some("u@x.test")));
        assert!(matches!(p, Principal::Human), "expected Human, got {p:?}");
    }

    /// Fall-open #1: machine-shaped subject, no grant declaration. Under the old
    /// `Option` contract this returned `None` and the surfaces' `else` arm made it a
    /// human — on temper-mcp, an auto-provisioned one.
    #[test]
    fn gty_less_clients_suffixed_sub_is_refused_not_human() {
        let p = classify(&raw(None, None, "abc123@clients", None));
        assert!(
            matches!(p, Principal::Refuse(why) if why.contains("machine-shaped")),
            "a machine-shaped subject without the grant must be refused, got {p:?}"
        );
    }

    /// The same shape with a *human* grant type is still a refusal: `gty` says
    /// authorization_code, `sub` says machine. Incoherent — we do not pick a side.
    #[test]
    fn clients_suffixed_sub_with_human_gty_is_refused() {
        let p = classify(&raw(
            Some("authorization_code"),
            Some("abc123"),
            "abc123@clients",
            None,
        ));
        assert!(
            matches!(p, Principal::Refuse(_)),
            "expected Refuse, got {p:?}"
        );
    }

    /// Fall-open #2: the token *declares* the machine grant, but no client id can be
    /// derived (no `azp`, and `sub` is not `@clients`-suffixed). It self-identifies as
    /// a machine; treating it as a human was the sharpest edge of the old default arm.
    #[test]
    fn client_credentials_without_derivable_client_id_is_refused() {
        let p = classify(&raw(Some("client-credentials"), None, "auth0|user", None));
        assert!(
            matches!(p, Principal::Refuse(why) if why.contains("client id")),
            "a machine token with no derivable client id must be refused, got {p:?}"
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
        let c = expect_machine(classify(&raw));
        assert_eq!(c.principal_kind, PrincipalKind::Machine);
        assert_eq!(c.provider, "auth0-m2m");
        assert_eq!(c.external_user_id, "y23AQxuvzjYSb5n8lAUeuIgIXOftCWYu");
    }
}
