#![cfg(feature = "test-db")]
//! Access-capability grant/revoke service primitive (D3b §3.C) + the per-profile backfill query
//! (§3.D). Drives `access_service::grant_capability`/`revoke_capability` directly (the surfaces are
//! covered by the handler + e2e tiers). Membership/grants are seeded directly via SQL so the general
//! `can()` seam genuinely flips.

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::access_service;

// ── fixtures ──────────────────────────────────────────────────────────────────────

async fn mint_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("mint profile")
}

/// Mint an admin that passes `is_system_admin`. The canonical seed leaves `gating_team_slug` NULL
/// (open mode), and `is_system_admin` resolves through that slug — so we first configure it to
/// `temper-system`, then `system_access='admin'` enrolls the profile as an `owner` of temper-system
/// via the auto-join trigger (the production-shaped config, mirroring `cogmap_authz_test`).
async fn mint_admin(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .expect("configure gating team");
    let id = mint_profile(pool, handle).await;
    sqlx::query("UPDATE kb_profiles SET system_access = 'admin' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("promote admin");
    id
}

async fn system_emitter(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.handle = 'system' AND e.name = 'system'",
    )
    .fetch_one(pool)
    .await
    .expect("system emitter must exist")
}

/// Birth a fresh (unbound) cognitive map via `cogmap_genesis`. Returns the cogmap id.
async fn mint_unbound_cogmap(pool: &PgPool, owner: Uuid, name: &str) -> Uuid {
    let cogmap = Uuid::now_v7();
    let telos = Uuid::now_v7();
    let emitter = system_emitter(pool).await;
    sqlx::query("SELECT cogmap_genesis($1, $2, $3)")
        .bind(json!({
            "cogmap_id": cogmap,
            "name": name,
            "owner_profile_id": owner,
            "telos": { "resource_id": telos, "title": format!("{name} telos"),
                       "origin_uri": format!("temper://test/{name}/telos"), "blocks": [] },
        }))
        .bind(json!({}))
        .bind(emitter)
        .execute(pool)
        .await
        .expect("birth cogmap");
    cogmap
}

async fn can_write_cogmap(pool: &PgPool, profile: Uuid, cogmap: Uuid) -> bool {
    sqlx::query_scalar::<_, Option<bool>>(
        "SELECT can('kb_profiles', $1, 'write', 'kb_cogmaps', $2)",
    )
    .bind(profile)
    .bind(cogmap)
    .fetch_one(pool)
    .await
    .expect("can() query")
    .unwrap_or(false)
}

fn write_grant(cogmap: Uuid, grantee: Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_cogmaps".into(),
        subject_id: cogmap,
        principal_table: "kb_profiles".into(),
        principal_id: grantee,
        can_read: true,
        can_write: true,
        can_delete: false,
        can_grant: false,
    }
}

// ── (a) admin grants + revokes cogmap write; the general seam flips ─────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_can_grant_and_revoke_cogmap_write(pool: PgPool) {
    let admin = mint_admin(&pool, "grant-admin").await;
    let grantee = mint_profile(&pool, "grantee").await; // no membership, no grant
    let cogmap = mint_unbound_cogmap(&pool, admin, "grant-target").await;

    assert!(
        !can_write_cogmap(&pool, grantee, cogmap).await,
        "no grant ⇒ no write"
    );

    let out = access_service::grant_capability(
        &pool,
        ProfileId::from(admin),
        &write_grant(cogmap, grantee),
    )
    .await
    .expect("admin grant");
    assert!(out.granted, "a fresh grant reports granted=true");
    assert!(
        can_write_cogmap(&pool, grantee, cogmap).await,
        "explicit can_write grant confers write"
    );

    access_service::revoke_capability(
        &pool,
        ProfileId::from(admin),
        &RevokeCapabilityRequest {
            subject_table: "kb_cogmaps".into(),
            subject_id: cogmap,
            principal_table: "kb_profiles".into(),
            principal_id: grantee,
        },
    )
    .await
    .expect("admin revoke");
    assert!(
        !can_write_cogmap(&pool, grantee, cogmap).await,
        "revoke removes write"
    );
}

// ── (b) a non-admin, non-granter is forbidden ───────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_granter_is_forbidden(pool: PgPool) {
    let stranger = mint_profile(&pool, "stranger").await; // not admin, no can_grant
    let grantee = mint_profile(&pool, "grantee2").await;
    let cogmap = mint_unbound_cogmap(&pool, stranger, "forbidden-target").await;

    let err = access_service::grant_capability(
        &pool,
        ProfileId::from(stranger),
        &write_grant(cogmap, grantee),
    )
    .await
    .expect_err("a non-admin non-granter cannot grant");
    assert!(matches!(err, ApiError::Forbidden));
}

// ── (c) a can_grant holder (delegated admin) can grant ──────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn can_grant_holder_can_delegate(pool: PgPool) {
    let admin = mint_admin(&pool, "root-admin").await;
    let delegate = mint_profile(&pool, "delegate").await;
    let grantee = mint_profile(&pool, "grantee3").await;
    let cogmap = mint_unbound_cogmap(&pool, admin, "delegate-target").await;

    // Admin gives `delegate` read+grant (delegated administration) but NOT write.
    access_service::grant_capability(
        &pool,
        ProfileId::from(admin),
        &GrantCapabilityRequest {
            subject_table: "kb_cogmaps".into(),
            subject_id: cogmap,
            principal_table: "kb_profiles".into(),
            principal_id: delegate,
            can_read: true,
            can_write: false,
            can_delete: false,
            can_grant: true,
        },
    )
    .await
    .expect("admin delegates grant authority");

    // `delegate` (can_grant, not admin) can now grant write to a third party.
    access_service::grant_capability(
        &pool,
        ProfileId::from(delegate),
        &write_grant(cogmap, grantee),
    )
    .await
    .expect("delegate grants write via can_grant");
    assert!(can_write_cogmap(&pool, grantee, cogmap).await);
}
