//! Shared authentication + authorization orchestration for both surfaces.
//!
//! The gate *sequence* lives here exactly once. temper-api and temper-mcp both
//! call these functions and map [`AuthzError`] to their own transport; neither
//! re-implements the ordering. Adding a future gate is one edit here, enforced
//! on every surface.
//!
//! Two levels form a typestate chain:
//! 1. [`authenticate`] — resolve the profile + `is_active`. Runs on every authed
//!    request on both surfaces. Yields [`AuthenticatedProfile`].
//! 2. [`require_system_access`] — consumes proof of Level 1, adds the access gate.
//!    Runs on the gated tier of both surfaces. Yields [`SystemAuthorized`].

use sqlx::PgPool;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, AuthenticatedProfile};

use crate::error::ApiError;
use crate::services::profile_service;

/// The reason an authn/authz gate refused a request. Each surface maps these to
/// its own transport (HTTP status / rmcp error); the variants are the shared
/// vocabulary of *why*, never the words on the wire.
#[derive(Debug)]
pub enum AuthzError {
    /// `resolve_from_claims` failed (DB error, missing link data, etc.).
    ProfileResolution(ApiError),
    /// The `has_system_access` gate check itself failed (DB error) — distinct
    /// from a clean `SystemAccessDenied`, so surfaces can keep the pre-seam
    /// "failed to check system access" diagnostic instead of collapsing it into
    /// the resolve-failure message.
    AccessCheck(ApiError),
    /// The resolved profile is soft-deleted (`is_active == false`).
    Deactivated { profile_id: uuid::Uuid },
    /// The profile is not an approved member of the gating team.
    /// Carries the id so a surface can build its own denial payload.
    SystemAccessDenied { profile_id: uuid::Uuid },
}

/// Level 1 — authentication. Verified+normalized claims → a resolved, active profile.
///
/// Runs on **every** authenticated request on **both** surfaces. Callers are
/// responsible for verifying the JWT and normalizing it into `claims` first
/// (each surface's audience differs); this function owns resolve + the
/// deactivation gate.
pub async fn authenticate(
    pool: &PgPool,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError> {
    let profile = profile_service::resolve_from_claims(pool, claims)
        .await
        .map_err(AuthzError::ProfileResolution)?;

    if !profile.is_active {
        return Err(AuthzError::Deactivated {
            profile_id: profile.id,
        });
    }

    Ok(AuthenticatedProfile {
        profile,
        claims: claims.clone(),
    })
}

/// Proof that a profile passed **both** levels: authenticated *and*
/// system-authorized. Only obtainable from [`require_system_access`], which
/// only accepts an [`AuthenticatedProfile`] — so the type makes it impossible
/// to run Level 2 without having passed Level 1.
#[derive(Debug)]
pub struct SystemAuthorized(pub AuthenticatedProfile);

/// Level 2 — system authorization. Consumes proof of Level 1, adds the
/// gating-team access gate. Runs on the gated tier of both surfaces.
pub async fn require_system_access(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError> {
    let has_access = crate::services::access_service::has_system_access(
        pool,
        ProfileId::from(authed.profile.id),
    )
    .await
    .map_err(AuthzError::AccessCheck)?;

    if !has_access {
        return Err(AuthzError::SystemAccessDenied {
            profile_id: authed.profile.id,
        });
    }

    Ok(SystemAuthorized(authed.clone()))
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    // Helper: build AuthClaims for a synthetic principal.
    fn claims(sub: &str, email: &str) -> AuthClaims {
        AuthClaims {
            principal_kind: temper_core::types::PrincipalKind::Human,
            provider: "test-provider".to_string(),
            external_user_id: sub.to_string(),
            email: email.to_string(),
            email_verified: Some(true),
            exp: 0,
            iat: 0,
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticate_returns_active_profile(pool: PgPool) {
        let c = claims("seam-active", "active@example.test");
        let authed = authenticate(&pool, &c).await.expect("should authenticate");
        assert!(authed.profile.is_active);
        assert_eq!(authed.claims.external_user_id, "seam-active");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticate_refuses_deactivated_profile(pool: PgPool) {
        // First resolve creates the profile.
        let c = claims("seam-deactivated", "deact@example.test");
        let authed = authenticate(&pool, &c).await.expect("first resolve");
        let id = authed.profile.id;

        // Soft-delete it (runtime query — test fixture, no macro cache needed).
        sqlx::query("UPDATE kb_profiles SET is_active = false WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("deactivate");

        let err = authenticate(&pool, &c).await.expect_err("should refuse");
        assert!(
            matches!(err, AuthzError::Deactivated { profile_id } if profile_id == id),
            "expected Deactivated, got {err:?}",
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_system_access_allows_approved_profile(pool: PgPool) {
        // Open-mode default: an authenticated profile has system access.
        let c = claims("seam-approved", "approved@example.test");
        let authed = authenticate(&pool, &c).await.expect("authenticate");
        let ok = require_system_access(&pool, &authed).await;
        assert!(
            ok.is_ok(),
            "open-mode profile should be system-authorized: {ok:?}"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_system_access_refuses_when_gated(pool: PgPool) {
        // Enable invite-only so a fresh profile is NOT an approved member.
        // enable_invite_only lives in the e2e harness; here we set the gate
        // directly: point kb_system_settings at a gating team the profile
        // does not belong to.
        let c = claims("seam-gated", "gated@example.test");
        let authed = authenticate(&pool, &c).await.expect("authenticate");
        let id = authed.profile.id;

        sqlx::query(
            "UPDATE kb_system_settings SET access_mode = 'invite_only', \
             gating_team_slug = 'nonexistent-gating-team'",
        )
        .execute(&pool)
        .await
        .expect("enable gate");

        let err = require_system_access(&pool, &authed)
            .await
            .expect_err("gated profile should be refused");
        assert!(
            matches!(err, AuthzError::SystemAccessDenied { profile_id } if profile_id == id),
            "expected SystemAccessDenied, got {err:?}",
        );
    }
}
