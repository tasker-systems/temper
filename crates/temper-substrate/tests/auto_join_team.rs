#![cfg(feature = "artifact-tests")]
//! Auto-join team enrollment (`20260629000002`, repointed onto standing by Phase 2 A4
//! `20260722000010`): a team flagged with `kb_teams.auto_join_role` is an always-complete
//! "everyone" pool for every principal that `has_system_access`.
//!
//! Each test runs on an ephemeral `public`-schema database via
//! `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, so the full canonical
//! chain (schema + functions + seed + L0 + this migration) is applied: `temper-system`
//! exists with `auto_join_role = 'watcher'`, and the boot-seeded `system` admin has been
//! backfilled as its owner.
//!
//! Semantics under test (post-Phase-2-A4):
//!   - eligibility gates on `has_system_access(profile)`, which reads ONLY an `approved`
//!     `kb_principal_standing` row (D2) — NOT the retired `kb_profiles.system_access` column;
//!   - enrollment role is the team's `auto_join_role` UNIFORMLY — the old `system_access='admin'
//!     → owner` coupling is gone (admin-ness lives in `kb_principal_governance` now; auto-join
//!     membership is decorative under D18);
//!   - `ensure_auto_join_memberships` / `backfill_auto_join_team` are idempotent and never
//!     clobber a manually-set role.
//!
//! The trigger `trg_sync_system_membership` and its revoke-time `ELSE DELETE` (the old "losing
//! access removes membership from ALL auto-join teams", Q-C) are NOT tested here: the trigger is
//! bound to the doomed `system_access` column and PR-B drops both. Demotion is owned by
//! governance now (spec §11 / E5), and stale decorative memberships are harmless under D18.

use sqlx::PgPool;
use uuid::Uuid;

/// Insert a bare profile (no `system_access` write — Phase 2 A4). No standing row yet, so
/// `has_system_access` is false until [`approve`] mints one. The AFTER-INSERT trigger fires and
/// no-ops (not eligible).
async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("insert profile")
}

/// Mint the authoritative `approved` standing row that makes `has_system_access` true — the
/// incumbent eligibility pattern (the scenario loaders and boot-seed use the same call).
async fn approve(pool: &PgPool, profile: Uuid) {
    sqlx::query("SELECT principal_standing_apply($1,'provision','approved',NULL,'test approve')")
        .bind(profile)
        .execute(pool)
        .await
        .expect("mint approved standing");
}

/// Mark an already-approved profile as a governing admin. Under the repointed functions this must
/// make NO difference to auto-join role (the `admin → owner` coupling is gone).
async fn make_admin(pool: &PgPool, profile: Uuid) {
    sqlx::query("SELECT principal_governance_set($1,true,NULL,'test admin')")
        .bind(profile)
        .execute(pool)
        .await
        .expect("set governance");
}

/// Create a team, optionally flagged as an auto-join team at `auto_join_role`.
async fn create_team(pool: &PgPool, slug: &str, auto_join_role: Option<&str>) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) VALUES ($1, $1, $2::team_role) RETURNING id",
    )
    .bind(slug)
    .bind(auto_join_role)
    .fetch_one(pool)
    .await
    .expect("create team")
}

/// The boot-seeded `temper-system` root team id.
async fn temper_system_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(pool)
        .await
        .expect("temper-system exists")
}

/// This profile's role in `team`, or `None` if not a member.
async fn role_in(pool: &PgPool, team: Uuid, profile: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(team)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .expect("query role")
}

/// Count of this profile's memberships in teams flagged as auto-join (excludes the
/// personal team, which is never auto-join).
async fn auto_join_membership_count(pool: &PgPool, profile: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_team_members tm \
         JOIN kb_teams t ON t.id = tm.team_id \
         WHERE tm.profile_id = $1 AND t.auto_join_role IS NOT NULL",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("count auto-join memberships")
}

/// temper-system is an ordinary auto-join team (Q-A): the migration flags it `watcher`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn temper_system_is_flagged_auto_join_watcher(pool: PgPool) {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT auto_join_role::text FROM kb_teams WHERE slug = 'temper-system'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(role.as_deref(), Some("watcher"));
}

/// `ensure_auto_join_memberships` enrolls an `approved` profile into EVERY auto-join team at the
/// team's `auto_join_role` — and a governing admin gets the SAME role (no admin→owner coupling).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ensure_enrolls_approved_profile_at_team_role(pool: PgPool) {
    let everyone2 = create_team(&pool, "everyone-2", Some("watcher")).await;
    let root = temper_system_id(&pool).await;

    // A plain approved profile: eligible only after standing is minted.
    let alice = insert_profile(&pool, "alice").await;
    assert_eq!(
        auto_join_membership_count(&pool, alice).await,
        0,
        "not eligible before approval"
    );
    approve(&pool, alice).await;
    sqlx::query("SELECT ensure_auto_join_memberships($1)")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        role_in(&pool, root, alice).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(
        role_in(&pool, everyone2, alice).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(auto_join_membership_count(&pool, alice).await, 2);

    // A governing admin enrolls at the team's auto_join_role too — the admin→owner coupling is gone.
    let admin = insert_profile(&pool, "adminuser").await;
    approve(&pool, admin).await;
    make_admin(&pool, admin).await;
    sqlx::query("SELECT ensure_auto_join_memberships($1)")
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        role_in(&pool, root, admin).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(
        role_in(&pool, everyone2, admin).await.as_deref(),
        Some("watcher")
    );
}

/// `backfill_auto_join_team` enrolls all pre-existing `has_system_access` profiles when a
/// team's flag is newly enabled, and is idempotent on re-run (and does not clobber manual roles).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn backfill_enrolls_approved_profiles_idempotently(pool: PgPool) {
    let bob = insert_profile(&pool, "bob").await;
    approve(&pool, bob).await;

    // A plain team, then flagged auto-join after the fact.
    let corp = create_team(&pool, "corp", None).await;
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'watcher' WHERE id = $1")
        .bind(corp)
        .execute(&pool)
        .await
        .unwrap();

    // Before backfill: bob is not yet in corp (the column flip alone enrolls nobody).
    assert_eq!(role_in(&pool, corp, bob).await, None);

    // Backfill enrolls every has_system_access profile at the team's role. bob → watcher; the
    // boot-seeded `system` admin → watcher too (uniform role — no admin→owner).
    sqlx::query("SELECT backfill_auto_join_team($1)")
        .bind(corp)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(role_in(&pool, corp, bob).await.as_deref(), Some("watcher"));
    let system: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = 'system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        role_in(&pool, corp, system).await.as_deref(),
        Some("watcher")
    );

    // A manually-elevated role survives a re-run (ON CONFLICT DO NOTHING).
    sqlx::query(
        "UPDATE kb_team_members SET role = 'maintainer' WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(corp)
    .bind(bob)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("SELECT backfill_auto_join_team($1)")
        .bind(corp)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        role_in(&pool, corp, bob).await.as_deref(),
        Some("maintainer"),
        "backfill must not clobber a manually-set role"
    );
}

/// A profile with no `approved` standing (`has_system_access` false) enrolls nowhere — eligibility
/// is standing, not the retired column.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ineligible_profile_enrolls_nowhere(pool: PgPool) {
    let carol = insert_profile(&pool, "carol").await; // never approved
    sqlx::query("SELECT ensure_auto_join_memberships($1)")
        .bind(carol)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(auto_join_membership_count(&pool, carol).await, 0);
}

/// `ensure_auto_join_memberships` is idempotent: a second call produces no change and no error.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ensure_is_idempotent(pool: PgPool) {
    let dave = insert_profile(&pool, "dave").await;
    approve(&pool, dave).await;

    for _ in 0..2 {
        sqlx::query("SELECT ensure_auto_join_memberships($1)")
            .bind(dave)
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(auto_join_membership_count(&pool, dave).await, 1); // temper-system only
    let root = temper_system_id(&pool).await;
    assert_eq!(role_in(&pool, root, dave).await.as_deref(), Some("watcher"));
}
