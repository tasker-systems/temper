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
//!     clobber a manually-set role (enrollment defers to explicit state — `DO NOTHING`).
//!
//! Enrollment is materialized at the standing committer itself (`20260923000010`), so every
//! approval door inherits it — the committer is the pin: a door that forgets to enroll is
//! not representable. The pool is APPEND-ONLY: no standing transition ever removes an
//! auto-join membership, because post-D11 those rows are owned by other authorities — D14
//! machine hygiene (`enroll_in_gating_team` enrolls born-`denied` machines), D17/D11
//! revocation intent (grants and memberships deliberately survive for the rebind story),
//! and IdP provenance (`reconcile_idp_memberships` owns `source='idp'` rows). The
//! invariant is one-directional: every standing-approved profile is a member; the converse
//! holds only among transitions the committer saw, and stale rows are harmless under D18.

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

/// Drive the standing committer with an explicit act. Every door — direct grant, request
/// review, promotion, reactivation, revoke, deactivate — routes through this one function,
/// which is why the auto-join mirror lives inside it.
async fn apply_standing(pool: &PgPool, profile: Uuid, act: &str, resulting: &str) {
    sqlx::query("SELECT principal_standing_apply($1,$2,$3,NULL,'test transition')")
        .bind(profile)
        .bind(act)
        .bind(resulting)
        .execute(pool)
        .await
        .expect("apply standing");
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

/// The direct-grant door enrolls: a standing transition to `approved` through the committer
/// alone — no review, no separate ensure call — lands the principal in every auto-join team.
/// This is the gap the materialization closes (admin_approve previously conferred standing
/// only, and the pool drifted incomplete with no signal).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn direct_grant_enrolls_at_committer(pool: PgPool) {
    let everyone2 = create_team(&pool, "everyone-2", Some("watcher")).await;
    let root = temper_system_id(&pool).await;

    let erin = insert_profile(&pool, "erin").await;
    apply_standing(&pool, erin, "provision", "denied").await;
    assert_eq!(
        auto_join_membership_count(&pool, erin).await,
        0,
        "denied principals enroll nowhere"
    );

    // The direct grant: approve from `denied` (D14), through the committer and nothing else.
    apply_standing(&pool, erin, "approve", "approved").await;
    assert_eq!(
        role_in(&pool, root, erin).await.as_deref(),
        Some("watcher"),
        "the committer's enrollment arm must fire on approve"
    );
    assert_eq!(
        role_in(&pool, everyone2, erin).await.as_deref(),
        Some("watcher")
    );
    assert_eq!(auto_join_membership_count(&pool, erin).await, 2);
}

/// Revocation removes NOTHING — the pool is append-only. Post-D11, auto-join memberships
/// are owned by other authorities: D14 machine hygiene, D17/D11 revocation intent (grants
/// and memberships deliberately survive for the rebind story), and IdP provenance. A
/// standing transition that deleted any of them would reintroduce the silent-corruption
/// class this fix exists to prevent.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revocation_leaves_memberships_untouched(pool: PgPool) {
    let everyone2 = create_team(&pool, "everyone-2", Some("watcher")).await;
    let root = temper_system_id(&pool).await;

    let frank = insert_profile(&pool, "frank").await;
    approve(&pool, frank).await;
    assert_eq!(auto_join_membership_count(&pool, frank).await, 2);

    // An admin explicitly elevates frank on one auto-join team.
    sqlx::query(
        "UPDATE kb_team_members SET role = 'maintainer' WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(everyone2)
    .bind(frank)
    .execute(&pool)
    .await
    .unwrap();

    apply_standing(&pool, frank, "revoke", "revoked").await;
    assert_eq!(
        role_in(&pool, root, frank).await.as_deref(),
        Some("watcher"),
        "revocation must leave the enrollment alone (D17/D11: memberships survive)"
    );
    assert_eq!(
        role_in(&pool, everyone2, frank).await.as_deref(),
        Some("maintainer"),
        "an explicit grant must survive every standing transition"
    );
    assert_eq!(auto_join_membership_count(&pool, frank).await, 2);

    // Re-approval enrolls nothing new (DO NOTHING) — the roster is exactly as it was.
    apply_standing(&pool, frank, "approve", "approved").await;
    assert_eq!(auto_join_membership_count(&pool, frank).await, 2);
    assert_eq!(
        role_in(&pool, root, frank).await.as_deref(),
        Some("watcher")
    );
}

/// Deactivation removes NOTHING either, and reactivation therefore finds the roster
/// exactly as it was — Reactivate restores rather than guesses, and the pool never lost
/// what it would restore.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn deactivate_then_reactivate_roundtrip(pool: PgPool) {
    let root = temper_system_id(&pool).await;

    let gita = insert_profile(&pool, "gita").await;
    approve(&pool, gita).await;
    assert_eq!(role_in(&pool, root, gita).await.as_deref(), Some("watcher"));

    apply_standing(&pool, gita, "deactivate", "deactivated").await;
    assert_eq!(
        role_in(&pool, root, gita).await.as_deref(),
        Some("watcher"),
        "deactivation must leave the enrollment alone (append-only pool)"
    );

    apply_standing(&pool, gita, "reactivate", "approved").await;
    assert_eq!(
        role_in(&pool, root, gita).await.as_deref(),
        Some("watcher"),
        "reactivation finds the roster exactly as it was"
    );
}

/// THE INVARIANT: for every auto-join team, membership ⊇ {profiles with has_system_access} —
/// asserted as an enumerated violation set over mixed doors, not inferred from absence.
///
/// A fresh install carries exactly ONE violation: the boot-seeded `system` profile holds
/// approved standing granted by the frozen `20260720000120` backfill — outside every door —
/// so no committer transition ever enrolled it into teams created later. That residual is
/// what the reconcile verb exists for; the invariant holds exactly once it has run.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn committer_invariant_sweep(pool: PgPool) {
    create_team(&pool, "everyone-2", Some("watcher")).await;

    // Mixed doors: boot-style provision-to-approved, direct grant, reactivate.
    let h1 = insert_profile(&pool, "hana").await;
    approve(&pool, h1).await;
    let h2 = insert_profile(&pool, "hugo").await;
    apply_standing(&pool, h2, "provision", "denied").await;
    apply_standing(&pool, h2, "approve", "approved").await;
    let h3 = insert_profile(&pool, "iris").await;
    apply_standing(&pool, h3, "provision", "denied").await;
    apply_standing(&pool, h3, "approve", "approved").await;
    apply_standing(&pool, h3, "deactivate", "deactivated").await;
    apply_standing(&pool, h3, "reactivate", "approved").await;
    // A never-approved profile: outside the eligible set.
    insert_profile(&pool, "outsider").await;

    let violations_before: Vec<(String, String)> = sqlx::query_as(
        "SELECT t.slug, p.handle FROM kb_teams t CROSS JOIN kb_profiles p \
         WHERE t.auto_join_role IS NOT NULL AND has_system_access(p.id) \
           AND NOT EXISTS (SELECT 1 FROM kb_team_members m \
                            WHERE m.team_id = t.id AND m.profile_id = p.id)",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        violations_before,
        vec![("everyone-2".to_string(), "system".to_string())],
        "the only residual is standing granted outside every door (the frozen backfill)"
    );

    sqlx::query("SELECT auto_join_reconcile()")
        .execute(&pool)
        .await
        .unwrap();

    let violations_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_teams t CROSS JOIN kb_profiles p \
         WHERE t.auto_join_role IS NOT NULL AND has_system_access(p.id) \
           AND NOT EXISTS (SELECT 1 FROM kb_team_members m \
                            WHERE m.team_id = t.id AND m.profile_id = p.id)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        violations_after, 0,
        "after reconcile, every auto-join team contains every eligible profile"
    );
}

/// An IdP-authored row at the team's `auto_join_role` survives every standing transition —
/// the pool is append-only, and SAML owns its rows outright (`reconcile_idp_memberships`
/// skips teams where a native row exists): a standing transition deleting one would
/// silently convert IdP authority into a native row the IdP never reasserts.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn demotion_preserves_idp_authored_rows(pool: PgPool) {
    let root = temper_system_id(&pool).await;

    let kate = insert_profile(&pool, "kate").await;
    approve(&pool, kate).await;
    // SAML asserts kate's membership in the pool: the row becomes IdP-authored.
    sqlx::query("UPDATE kb_team_members SET source = 'idp' WHERE team_id = $1 AND profile_id = $2")
        .bind(root)
        .bind(kate)
        .execute(&pool)
        .await
        .unwrap();

    apply_standing(&pool, kate, "deactivate", "deactivated").await;
    assert_eq!(
        role_in(&pool, root, kate).await.as_deref(),
        Some("watcher"),
        "an IdP-authored row must survive the committer's delete arm"
    );
}

/// `auto_join_reconcile` converges a drifted instance — profiles approved before a team's
/// flag existed — reports exactly the pairs it added, never rewrites explicit roles, and
/// reconciles to zero on a converged instance.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn reconcile_converges_and_reports(pool: PgPool) {
    // Approved while only temper-system is auto-join; the flag lands afterwards (the drift).
    let jules = insert_profile(&pool, "jules").await;
    approve(&pool, jules).await;

    let corp = create_team(&pool, "corp", Some("watcher")).await;
    assert_eq!(
        role_in(&pool, corp, jules).await,
        None,
        "drift precondition"
    );

    let mut added: Vec<(String, String)> =
        sqlx::query_as("SELECT team_slug, profile_handle FROM auto_join_reconcile()")
            .fetch_all(&pool)
            .await
            .unwrap();
    added.sort();
    // jules, and the boot-seeded `system` profile (standing granted outside every door by
    // the frozen backfill — the residual the reconcile verb exists for).
    assert_eq!(
        added,
        vec![
            ("corp".to_string(), "jules".to_string()),
            ("corp".to_string(), "system".to_string()),
        ],
        "reconcile must report exactly the pairs it added"
    );
    assert_eq!(
        role_in(&pool, corp, jules).await.as_deref(),
        Some("watcher")
    );

    // An explicit role is never rewritten by reconciliation.
    sqlx::query(
        "UPDATE kb_team_members SET role = 'maintainer' WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(corp)
    .bind(jules)
    .execute(&pool)
    .await
    .unwrap();

    let second: Vec<(String, String)> =
        sqlx::query_as("SELECT team_slug, profile_handle FROM auto_join_reconcile()")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        second.is_empty(),
        "a converged instance reconciles to zero, got {second:?}"
    );
    assert_eq!(
        role_in(&pool, corp, jules).await.as_deref(),
        Some("maintainer"),
        "reconcile must not clobber an explicit role"
    );
}
