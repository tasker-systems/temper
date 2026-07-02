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
//! 2. `require_system_access` — consumes proof of Level 1, adds the access gate.
//!    Runs on the gated tier of both surfaces. Yields `SystemAuthorized`.

use sqlx::PgPool;

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

// `require_system_access` + `SystemAuthorized` land in Task 2.

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    // Helper: build AuthClaims for a synthetic principal.
    fn claims(sub: &str, email: &str) -> AuthClaims {
        AuthClaims {
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
}
