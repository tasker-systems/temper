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

/// Mint a real `AuthenticatedProfile` for a seeded profile id — for tests that exercise the auth
/// ladder directly (e.g. minting a `SystemAdmin` via `require_system_admin`). The claims are a
/// minimal human token: the proofs downstream only read `profile.id`.
///
/// This runs the **actual** Level-1 gate rather than assembling the proof by hand, which it used to
/// do back when the type had public fields. Sealing it removed the shortcut, and that is the point:
/// a helper that forges the proof it is meant to supply would hand tests an identity production can
/// never produce. So a fixture seeded `Deactivated` panics here exactly as it would be refused in
/// production — if you need that case, assert on the gate itself.
pub async fn authenticated_profile_for(
    pool: &PgPool,
    profile_id: Uuid,
) -> crate::auth::AuthenticatedProfile {
    use temper_core::types::ids::ProfileId;
    use temper_core::types::{AuthClaims, PrincipalKind};

    let profile = crate::services::profile_service::get_by_id(pool, ProfileId::from(profile_id))
        .await
        .expect("load seeded profile");
    let claims = AuthClaims {
        principal_kind: PrincipalKind::Human,
        provider: "test".to_string(),
        external_user_id: format!("test|{profile_id}"),
        email: format!("{profile_id}@test.invalid"),
        email_verified: Some(true),
        exp: 0,
        iat: 0,
    };
    crate::auth::gate_resolved_profile(pool, profile, &claims)
        .await
        .expect("seeded fixture must pass the Level-1 gate")
}

/// Mint a real, sealed `SystemAdmin` proof — seeding a fresh approved-admin operator and passing it
/// through the actual `require_system_admin` gate. For mechanics tests that must *call* a proof-gated
/// admin fn but do not themselves exercise the gate; the seal has no test bypass, so the honest path
/// is to mint one. The operator handle is uniquified so a test may mint more than one.
pub async fn system_admin_proof(pool: &PgPool) -> crate::auth::SystemAdmin {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('operator-' || gen_random_uuid(), 'operator') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed operator profile");
    approved_admin(pool, id).await;
    system_admin_proof_for(pool, id).await
}

/// Mint a sealed `SystemAdmin` proof for an **already-seeded** profile.
///
/// The variant to reach for when a test asserts on WHICH profile acted — on the ledger row it
/// authors, say. [`system_admin_proof`] seeds its own operator, so it cannot serve a test that
/// already holds the admin's id, and every such test was otherwise re-deriving these three lines
/// privately (there were three near-identical copies before this existed).
///
/// The profile must already satisfy `is_system_admin`; this runs the real gate, so a fixture that
/// forgot [`grant_governance`] panics here rather than silently acting unauthorized.
pub async fn system_admin_proof_for(pool: &PgPool, profile_id: Uuid) -> crate::auth::SystemAdmin {
    let authed = authenticated_profile_for(pool, profile_id).await;
    crate::auth::require_system_admin(pool, &authed)
        .await
        .expect("the seeded profile must satisfy is_system_admin to mint a proof")
}
