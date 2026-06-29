#![cfg(feature = "test-db")]
//! `access_service::require_cogmap_write_admin` — the structural write-gate: a write to a cognitive
//! map joined to the gating (root) team requires `is_system_admin`. A map NOT joined to the gating
//! team is ungated. The L0 system-default map (`20260625000001`) is joined to `temper-system`, so it
//! is the canonical gated case.
//!
//! The canonical seed leaves `kb_system_settings.gating_team_slug` NULL (open mode). Both the gate's
//! root-join detection AND `is_system_admin` resolve through that slug, so these tests first
//! configure it to `temper-system` — the production-shaped config the gate is designed for.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_services::error::ApiError;
use temper_services::services::access_service;

mod common;

const L0_COGMAP: CogmapId = CogmapId(Uuid::from_u128(0x00000000_0000_0000_0005_000000000001));

/// Configure the gating team slug to the root team born by the L0 migration. Without this the
/// canonical seed runs in `open` mode with a NULL gating slug (no root team configured).
async fn set_gating_team(pool: &PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .expect("set gating team slug");
}

/// Mint an admin profile: a fresh profile whose `system_access = 'admin'` makes it an `owner` of
/// `temper-system` via the `sync_system_membership` trigger.
async fn admin_profile(pool: &PgPool, email: &str) -> Uuid {
    let id = common::fixtures::create_test_profile(pool, email).await;
    sqlx::query("UPDATE kb_profiles SET system_access = 'admin' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("promote to admin");
    id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_write_requires_system_admin(pool: PgPool) {
    set_gating_team(&pool).await;

    // A non-admin profile (default system_access = 'none') is refused on the root-joined L0 map.
    let non_admin = common::fixtures::create_test_profile(&pool, "nonadmin@example.com").await;
    let denied =
        access_service::require_cogmap_write_admin(&pool, ProfileId::from(non_admin), L0_COGMAP)
            .await;
    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "non-admin must be Forbidden on the root-team-joined L0 map, got {denied:?}"
    );

    // An admin (owner of temper-system) is allowed on the same map.
    let admin = admin_profile(&pool, "admin@example.com").await;
    access_service::require_cogmap_write_admin(&pool, ProfileId::from(admin), L0_COGMAP)
        .await
        .expect("admin must pass the L0 write gate");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_is_immutable_when_gating_unconfigured(pool: PgPool) {
    // Do NOT configure gating — the canonical seed leaves `gating_team_slug` NULL (open mode).
    // The L0 write gate is fail-CLOSED: the reserved map requires `is_system_admin` unconditionally,
    // and with gating unconfigured `is_system_admin` is false for everyone — so L0 is denied to all
    // (immutable) until an operator intentionally configures gating. Without the unconditional L0
    // branch this would be fail-OPEN (the NULL gating slug would make the root-join branch return Ok).
    let any_profile = common::fixtures::create_test_profile(&pool, "anyone@example.com").await;
    let denied =
        access_service::require_cogmap_write_admin(&pool, ProfileId::from(any_profile), L0_COGMAP)
            .await;
    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "L0 must be immutable (Forbidden to all) when gating is unconfigured, got {denied:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_root_cogmap_is_ungated(pool: PgPool) {
    set_gating_team(&pool).await;

    // A cognitive map NOT joined to the gating team (no kb_team_cogmaps row) is ungated — its own
    // access rules apply elsewhere, not this root-team write gate.
    let non_root_cogmap = CogmapId::new();
    let non_admin = common::fixtures::create_test_profile(&pool, "user@example.com").await;
    access_service::require_cogmap_write_admin(&pool, ProfileId::from(non_admin), non_root_cogmap)
        .await
        .expect("the gate does not apply to a non-root-team cogmap");
}
