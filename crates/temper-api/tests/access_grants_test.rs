#![cfg(feature = "test-db")]
//! Access-capability grant/revoke service primitive (D3b §3.C) + the per-profile backfill query
//! (§3.D). Drives `access_service::grant_capability`/`revoke_capability` directly (the surfaces are
//! covered by the handler + e2e tiers). Membership/grants are seeded directly via SQL so the general
//! `can()` seam genuinely flips.

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::access_service;

// ── fixtures ──────────────────────────────────────────────────────────────────────

async fn mint_profile(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("mint profile");
    // A profile that ACTS must carry its `<handle>@web` emitter — grant_capability/revoke_capability
    // now resolve one to author the grant_created/grant_revoked event, exactly as production does
    // (a fixture that skips it passes while production 500s at resolve_emitter).
    sqlx::query(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
    )
    .bind(id)
    .bind(format!("{handle}@web"))
    .execute(pool)
    .await
    .expect("mint web emitter");
    id
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
    // D11: `system_access='admin'` + gating ownership confer neither has_system_access nor
    // is_system_admin now. Grant governance + approved standing so this profile is a real admin.
    common::fixtures::make_test_admin(pool, id).await;
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

/// A fresh non-auto-join team (slug-unique). `auto_join_role` defaults NULL.
async fn mint_team(pool: &PgPool, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(id)
        .bind(slug)
        .execute(pool)
        .await
        .expect("mint team");
    id
}

async fn add_member(pool: &PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add member");
}

async fn bind_cogmap(pool: &PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("bind cogmap to team");
}

/// The migration's backfill SELECT (20260701000001 step 1), verbatim — run against a hand-built
/// fixture so the query logic is tested even though the migration itself ran at DB init.
async fn run_backfill(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         SELECT DISTINCT 'kb_cogmaps', tc.cogmap_id, 'kb_profiles', tm.profile_id, true, true, \
                (SELECT id FROM kb_profiles WHERE handle = 'system') \
         FROM kb_team_cogmaps tc \
         JOIN kb_teams t ON t.id = tc.team_id \
         JOIN kb_team_members tm ON tm.team_id = tc.team_id \
         WHERE t.auto_join_role IS NULL \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("run backfill");
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

/// Delegated administration ATTENUATES (plan 5b.3): a `can_grant` holder may confer what it holds
/// and no more.
///
/// This test previously asserted the OPPOSITE — that a delegate without write could confer write —
/// as intended behavior. The Task 5b access trace showed the same ungated seam let a `read+grant`
/// principal escalate ITSELF to `write+delete+grant` through the chokepoint (proven against a live
/// database, not inferred). Amplification was the defect; the third-party case was simply its
/// better-behaved half. Policy changed deliberately, so the assertion inverts.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_delegate_confers_only_what_it_holds(pool: PgPool) {
    let admin = mint_admin(&pool, "root-admin").await;
    let delegate = mint_profile(&pool, "delegate").await;
    let grantee = mint_profile(&pool, "grantee3").await;
    let cogmap = mint_unbound_cogmap(&pool, admin, "delegate-target").await;

    let read_and_grant = |principal: Uuid| GrantCapabilityRequest {
        subject_table: "kb_cogmaps".into(),
        subject_id: cogmap,
        principal_table: "kb_profiles".into(),
        principal_id: principal,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: true,
    };

    // Admin gives `delegate` read+grant (delegated administration) but NOT write.
    access_service::grant_capability(&pool, ProfileId::from(admin), &read_and_grant(delegate))
        .await
        .expect("admin delegates grant authority");

    // It may NOT confer write, which it does not hold.
    let err = access_service::grant_capability(
        &pool,
        ProfileId::from(delegate),
        &write_grant(cogmap, grantee),
    )
    .await
    .expect_err("a delegate without write must not confer write");
    assert!(
        matches!(err, ApiError::Forbidden),
        "expected Forbidden, got {err:?}"
    );
    assert!(
        !can_write_cogmap(&pool, grantee, cogmap).await,
        "the refused grant must have written nothing"
    );

    // Delegation still WORKS — it just cannot amplify. The same delegate confers read+grant, both
    // of which it holds. (Guards against "fixing" attenuation by breaking delegation outright.)
    access_service::grant_capability(&pool, ProfileId::from(delegate), &read_and_grant(grantee))
        .await
        .expect("a delegate confers the capabilities it does hold");

    // The admin arm stays unrestricted, so bootstrap and repair remain operable.
    access_service::grant_capability(&pool, ProfileId::from(admin), &write_grant(cogmap, grantee))
        .await
        .expect("a system admin may still amplify");
    assert!(can_write_cogmap(&pool, grantee, cogmap).await);
}

/// The escalation the trace proved reachable: a `can_grant` holder re-granting to ITSELF. The
/// third-party path and the self path go through the same call, so a fix that only considered
/// "granting to someone else" would leave this open — which is the more dangerous direction.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_delegate_cannot_escalate_itself(pool: PgPool) {
    let admin = mint_admin(&pool, "root-admin").await;
    let delegate = mint_profile(&pool, "self-escalator").await;
    let cogmap = mint_unbound_cogmap(&pool, admin, "self-escalation-target").await;

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

    let err = access_service::grant_capability(
        &pool,
        ProfileId::from(delegate),
        &write_grant(cogmap, delegate),
    )
    .await
    .expect_err("a delegate must not raise its own capabilities");
    assert!(
        matches!(err, ApiError::Forbidden),
        "expected Forbidden, got {err:?}"
    );
    assert!(
        !can_write_cogmap(&pool, delegate, cogmap).await,
        "the delegate must still lack write after the refused self-grant"
    );
}

/// 5b.4: `require_cogmap_write_admin` now binds the GRANT axis, not just the write axis.
///
/// It always kept the reserved L0 kernel admin-only for writes, but `grant_capability` never
/// consulted it — so a `can_grant` holder could mint `can_write` on the kernel, reaching by the
/// grant axis exactly what the write axis forbids. The seeded row below is not hypothetical:
/// `machine_authz.rs` seeds precisely this shape in its own fixtures.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_can_grant_holder_cannot_administer_the_l0_kernel(pool: PgPool) {
    let l0 = uuid::uuid!("00000000-0000-0000-0005-000000000001");
    let holder = mint_profile(&pool, "l0-grant-holder").await;
    let grantee = mint_profile(&pool, "l0-grantee").await;

    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, false, true, $2)",
    )
    .bind(l0)
    .bind(holder)
    .execute(&pool)
    .await
    .expect("seed an explicit can_grant on the kernel");

    // Non-vacuity: the seeded row really does carry grant authority by the general `can()` seam —
    // so the denial below comes from the L0 gate, not from the holder simply lacking can_grant.
    let holds_grant = sqlx::query_scalar::<_, Option<bool>>(
        "SELECT can('kb_profiles', $1, 'grant', 'kb_cogmaps', $2)",
    )
    .bind(holder)
    .bind(l0)
    .fetch_one(&pool)
    .await
    .expect("probe can_grant")
    .unwrap_or(false);
    assert!(holds_grant, "fixture must actually confer can_grant on L0");

    let err =
        access_service::grant_capability(&pool, ProfileId::from(holder), &write_grant(l0, grantee))
            .await
            .expect_err("the L0 kernel must stay admin-only on the grant axis");
    assert!(
        matches!(err, ApiError::Forbidden),
        "expected Forbidden, got {err:?}"
    );
    assert!(!can_write_cogmap(&pool, grantee, l0).await);
}

// ── (d) backfill snapshots real-team members, not auto-join members ──────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_snapshots_real_members(pool: PgPool) {
    // A member of a NON-auto-join team joined to a map. Membership alone would NOT authorize
    // post-Q-A; the backfill snapshot grant restores authoring.
    let member = mint_profile(&pool, "backfill-member").await;
    let team = mint_team(&pool, "backfill-real-team").await; // auto_join_role NULL
    add_member(&pool, team, member).await;
    let owner = mint_profile(&pool, "backfill-owner").await;
    let cogmap = mint_unbound_cogmap(&pool, owner, "backfill-target").await;
    bind_cogmap(&pool, cogmap, team).await;

    assert!(
        !can_write_cogmap(&pool, member, cogmap).await,
        "before backfill, a member has no write (Q-A)"
    );
    run_backfill(&pool).await;
    assert!(
        can_write_cogmap(&pool, member, cogmap).await,
        "a backfilled real-team member authors"
    );
}

// ── (e) the L0 kernel gets NO backfilled human write grant (auto-join exclusion) ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_kernel_has_no_backfilled_write_grant(pool: PgPool) {
    // L0 (`system-default`) is joined only to auto-join `temper-system`, so the migration's backfill
    // (which already ran) excluded it — no human holds write to the operator-governed kernel.
    let l0 = uuid::uuid!("00000000-0000-0000-0005-000000000001");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants \
          WHERE subject_table = 'kb_cogmaps' AND subject_id = $1 AND can_write",
    )
    .bind(l0)
    .fetch_one(&pool)
    .await
    .expect("count L0 write grants");
    assert_eq!(n, 0, "no human gets write to the kernel via backfill");
}
