//! Test-only fixtures for the D11 admission model.
//!
//! Under D11 every principal is **born `Denied`** — a profile with no `kb_principal_standing`
//! row is refused by `admit`/`require_system_access` (see [`crate::services::standing_service`]).
//! Before D11, `access_mode='open'` made every profile ambiently approved, so a fixture could
//! seed a caller with a bare `INSERT kb_profiles` and expect it to act. That ambient is gone.
//!
//! These helpers restore, per fixture, what open-mode conferred centrally: an `approved` standing
//! (and, for admins, a `kb_principal_governance` grant — the capability the repointed
//! `is_system_admin` now reads instead of gating-team ownership).
//!
//! They write the state rows **directly** rather than through `standing_service::apply`. A fixture
//! wants the end state, not the append-only history: the service path would require a legal
//! `provision`→`approve` act sequence, an acting admin, and would stamp log events the fixture has
//! no use for. The upserts here are idempotent because the auto-join generalization means a freshly
//! provisioned profile may already hold a standing row before the fixture runs.

use sqlx::PgPool;
use uuid::Uuid;

/// Give `profile` an `approved` standing so `admit` and `require_system_access` let it act.
///
/// Upsert, not insert: the caller may have provisioned the profile through a path that already
/// wrote a (`denied`) standing row.
pub async fn approve(pool: &PgPool, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         VALUES ($1, 'approved')
         ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
    )
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed approved standing");
}

/// Grant `profile` governance — the capability the repointed `is_system_admin` predicate reads.
///
/// Gating-team ownership no longer confers admin standing on its own; a fixture that needs a system
/// admin must write this row. `granted_by` is left NULL (a fixture bootstrap, not a delegated grant).
pub async fn grant_governance(pool: &PgPool, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_principal_governance (profile_id)
         VALUES ($1)
         ON CONFLICT (profile_id) DO NOTHING",
    )
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed governance grant");
}

/// The common fixture shape: an `approved` principal that is also a system admin.
///
/// Equivalent to [`approve`] followed by [`grant_governance`]. Use for any fixture principal that
/// was, under open-mode, an ambient admin able to act on the system surface.
pub async fn approved_admin(pool: &PgPool, profile: Uuid) {
    approve(pool, profile).await;
    grant_governance(pool, profile).await;
}

/// Load a real `AuthenticatedProfile` for a seeded profile id — for tests that exercise the auth
/// ladder directly (e.g. minting a `SystemAdmin` via `require_system_admin`). The claims are a
/// minimal human token: the proofs downstream only read `profile.id`.
pub async fn authenticated_profile_for(
    pool: &PgPool,
    profile_id: Uuid,
) -> temper_core::types::AuthenticatedProfile {
    use temper_core::types::ids::ProfileId;
    use temper_core::types::{AuthClaims, AuthenticatedProfile, PrincipalKind};

    let profile = crate::services::profile_service::get_by_id(pool, ProfileId::from(profile_id))
        .await
        .expect("load seeded profile");
    AuthenticatedProfile {
        profile,
        claims: AuthClaims {
            principal_kind: PrincipalKind::Human,
            provider: "test".to_string(),
            external_user_id: format!("test|{profile_id}"),
            email: format!("{profile_id}@test.invalid"),
            email_verified: Some(true),
            exp: 0,
            iat: 0,
        },
    }
}
