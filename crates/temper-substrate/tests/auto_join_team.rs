#![cfg(feature = "artifact-tests")]
//! Auto-join team generalization (`20260629000002`): the `temper-system`-hardcoded
//! `sync_system_membership` trigger is generalized so ANY team flagged with
//! `kb_teams.auto_join_role` becomes an always-complete "everyone" pool.
//!
//! Each test runs on an ephemeral `public`-schema database via
//! `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, so the full canonical
//! chain (schema + functions + seed + L0 + this migration) is applied: `temper-system`
//! exists with `auto_join_role = 'watcher'`, and the boot-seeded `system` admin has been
//! backfilled as its owner. Pure SQL — no ONNX/embeddings.
//!
//! Semantics under test:
//!   - enrollment gates on `has_system_access(profile)` (computed), NOT the raw column;
//!   - `system_access = 'admin'` enrolls at `owner`, else the team's `auto_join_role`;
//!   - in `open` mode every profile auto-joins every auto-join team (the everyone-pool);
//!   - on losing access (`has_system_access` false) the trigger removes the profile from
//!     ALL auto-join teams (Q-C);
//!   - `ensure_auto_join_memberships` / `backfill_auto_join_team` are idempotent.

use sqlx::PgPool;
use uuid::Uuid;

/// Insert a bare profile (`system_access` defaults to `'none'`), returning its id. The
/// AFTER-INSERT trigger fires `sync_system_membership` (auto-join) + `sync_personal_team`.
async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("insert profile")
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

async fn set_access_mode(pool: &PgPool, mode: &str, gating_team_slug: Option<&str>) {
    sqlx::query(
        "UPDATE kb_system_settings SET access_mode = $1, gating_team_slug = $2 WHERE id = 1",
    )
    .bind(mode)
    .bind(gating_team_slug)
    .execute(pool)
    .await
    .expect("set access mode");
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

/// (open mode) Provisioning a profile auto-joins it into EVERY auto-join team at the team's
/// `auto_join_role`; an `admin` profile joins at `owner`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn open_mode_provision_auto_joins_every_team_at_role(pool: PgPool) {
    // A second auto-join team alongside the boot-seeded temper-system.
    let everyone2 = create_team(&pool, "everyone-2", Some("watcher")).await;
    let root = temper_system_id(&pool).await;

    // A plain (non-admin) profile auto-joins both auto-join teams as watcher.
    let alice = insert_profile(&pool, "alice").await;
    assert_eq!(
        role_in(&pool, root, alice).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(
        role_in(&pool, everyone2, alice).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(auto_join_membership_count(&pool, alice).await, 2);

    // An admin profile auto-joins both at owner (admin→owner mapping, applied uniformly).
    let admin = insert_profile(&pool, "adminuser").await;
    sqlx::query("UPDATE kb_profiles SET system_access = 'admin' WHERE id = $1")
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(role_in(&pool, root, admin).await.as_deref(), Some("owner"));
    assert_eq!(
        role_in(&pool, everyone2, admin).await.as_deref(),
        Some("owner")
    );
}

/// `backfill_auto_join_team` enrolls all pre-existing `has_system_access` profiles when a
/// team's flag is newly enabled, and is idempotent on re-run (and does not clobber manual roles).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn backfill_enrolls_existing_profiles_idempotently(pool: PgPool) {
    // Bob exists BEFORE the new team is flagged (open mode → he's already a temper-system watcher).
    let bob = insert_profile(&pool, "bob").await;

    // A plain team, then flagged auto-join after the fact.
    let corp = create_team(&pool, "corp", None).await;
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'watcher' WHERE id = $1")
        .bind(corp)
        .execute(&pool)
        .await
        .unwrap();

    // Before backfill: bob is not yet in corp (the column flip alone enrolls nobody).
    assert_eq!(role_in(&pool, corp, bob).await, None);

    // Backfill enrolls every has_system_access profile. system (seed admin) → owner; bob → watcher.
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
    assert_eq!(role_in(&pool, corp, system).await.as_deref(), Some("owner"));

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

/// (invite_only) A profile without gating-team membership is NOT enrolled; granting gating
/// membership + `ensure_auto_join_memberships` enrolls it into every auto-join team; losing
/// access then firing the trigger removes it from ALL auto-join teams (Q-C).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn invite_only_gates_enrollment_and_revoke_removes_all(pool: PgPool) {
    // A dedicated gating team (NOT itself an auto-join team) and a second everyone-pool.
    let gate = create_team(&pool, "gate", None).await;
    let everyone2 = create_team(&pool, "everyone-2", Some("watcher")).await;
    set_access_mode(&pool, "invite_only", Some("gate")).await;

    // Carol provisions with no gating membership → has_system_access false → enrolled nowhere.
    let carol = insert_profile(&pool, "carol").await;
    assert_eq!(auto_join_membership_count(&pool, carol).await, 0);

    // Grant access: add carol to the gating team (the approval-write shape), then ensure.
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(gate)
    .bind(carol)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("SELECT ensure_auto_join_memberships($1)")
        .bind(carol)
        .execute(&pool)
        .await
        .unwrap();
    let root = temper_system_id(&pool).await;
    assert_eq!(
        role_in(&pool, root, carol).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(
        role_in(&pool, everyone2, carol).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(auto_join_membership_count(&pool, carol).await, 2);

    // Revoke: remove gating membership (has_system_access → false), then fire the trigger via a
    // system_access UPDATE. The DELETE branch removes carol from ALL auto-join teams (Q-C).
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(gate)
        .bind(carol)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_profiles SET system_access = 'approved' WHERE id = $1")
        .bind(carol)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(auto_join_membership_count(&pool, carol).await, 0);
}

/// `ensure_auto_join_memberships` is idempotent: a second call produces no change and no error.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ensure_is_idempotent(pool: PgPool) {
    let dave = insert_profile(&pool, "dave").await; // open mode → already enrolled by the trigger
    let before = auto_join_membership_count(&pool, dave).await;
    assert_eq!(before, 1); // temper-system only

    for _ in 0..2 {
        sqlx::query("SELECT ensure_auto_join_memberships($1)")
            .bind(dave)
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(auto_join_membership_count(&pool, dave).await, before);
    let root = temper_system_id(&pool).await;
    assert_eq!(role_in(&pool, root, dave).await.as_deref(), Some("watcher"));
}
