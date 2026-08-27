#![cfg(feature = "test-db")]
//! Integration tests for SAML membership reconcile. Each test runs on an isolated
//! `#[sqlx::test]` database with the workspace migrations applied.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use temper_core::types::TeamRole;
use temper_services::services::saml_provisioning_service::{
    reconcile_idp_memberships, ReconcileCounts, ReconcileOutcome,
};
use uuid::Uuid;

/// Minimal fixtures: a profile, two teams, one IdP, and mappings. Returns (profile, team_a, team_b).
async fn seed(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("user-{}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();

    let team_a: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("eng-{}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();

    let team_b: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("ops-{}", Uuid::now_v7()))
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr)
         VALUES ('acme', true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', 'email', 'uid', 'groups')",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES
         ('acme', 'engineering', $1, 'member'),
         ('acme', 'eng-leads',   $1, 'maintainer'),
         ('acme', 'operations',  $2, 'member')",
    )
    .bind(team_a)
    .bind(team_b)
    .execute(pool)
    .await
    .unwrap();

    (profile, team_a, team_b)
}

async fn membership(pool: &PgPool, team: Uuid, profile: Uuid) -> Option<(String, String)> {
    sqlx::query_as::<_, (TeamRole, String)>(
        "SELECT role, source::text FROM kb_team_members WHERE team_id=$1 AND profile_id=$2",
    )
    .bind(team)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|(r, s)| (format!("{r:?}"), s))
}

/// Unwrap an outcome that is expected to have actually compared something.
///
/// A test that wanted a reconcile and silently accepted a skip would assert nothing at all — every
/// count would be zero and every membership assertion would be about untouched rows. So the
/// mismatch panics here rather than passing quietly downstream.
fn counts(outcome: ReconcileOutcome) -> ReconcileCounts {
    match outcome {
        ReconcileOutcome::Reconciled(c) => c,
        ReconcileOutcome::SignalMissing => {
            panic!("expected a reconcile, got SignalMissing — nothing was compared")
        }
    }
}

/// The recorded pair for one (profile, idp): (last_reconciled_at, last_skipped_at).
async fn record(
    pool: &PgPool,
    profile: Uuid,
    idp_key: &str,
) -> Option<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> {
    sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
        "SELECT last_reconciled_at, last_skipped_at FROM kb_saml_principal_reconcile
          WHERE profile_id = $1 AND idp_key = $2",
    )
    .bind(profile)
    .bind(idp_key)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn adds_idp_memberships_and_picks_max_role(pool: PgPool) {
    let (profile, team_a, team_b) = seed(&pool).await;

    let out = counts(
        reconcile_idp_memberships(
            &pool,
            profile,
            "acme",
            Some(&[
                "engineering".into(),
                "eng-leads".into(),
                "operations".into(),
            ]),
        )
        .await
        .unwrap(),
    );

    assert_eq!(out.added, 2);
    // engineering(member) + eng-leads(maintainer) collapse to Maintainer on team_a.
    assert_eq!(
        membership(&pool, team_a, profile).await,
        Some(("Maintainer".into(), "idp".into()))
    );
    assert_eq!(
        membership(&pool, team_b, profile).await,
        Some(("Member".into(), "idp".into()))
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revokes_idp_memberships_no_longer_asserted(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    reconcile_idp_memberships(&pool, profile, "acme", Some(&["engineering".into()]))
        .await
        .unwrap();
    assert!(membership(&pool, team_a, profile).await.is_some());

    // Second login: no groups asserted -> the idp row is revoked.
    let out = counts(
        reconcile_idp_memberships(&pool, profile, "acme", Some(&[]))
            .await
            .unwrap(),
    );
    assert_eq!(out.revoked, 1);
    assert_eq!(membership(&pool, team_a, profile).await, None);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn native_membership_is_never_touched(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    // A native membership on team_a (e.g. a join request approval).
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) VALUES ($1,$2,'owner','native')",
    )
    .bind(team_a)
    .bind(profile)
    .execute(&pool)
    .await
    .unwrap();

    // IdP asserts engineering (maps to team_a member) -> must skip; native owner survives.
    let out = counts(
        reconcile_idp_memberships(&pool, profile, "acme", Some(&["engineering".into()]))
            .await
            .unwrap(),
    );
    assert_eq!(out.skipped_native, 1);
    assert_eq!(out.added, 0);
    assert_eq!(
        membership(&pool, team_a, profile).await,
        Some(("Owner".into(), "native".into()))
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn asserted_groups_are_captured_for_discovery_even_when_unmapped(pool: PgPool) {
    let (profile, _team_a, _team_b) = seed(&pool).await;
    // 'engineering' is mapped; 'ghosts' is NOT mapped — both must still be captured.
    reconcile_idp_memberships(
        &pool,
        profile,
        "acme",
        Some(&["engineering".into(), "ghosts".into()]),
    )
    .await
    .unwrap();

    let seen: Vec<String> = sqlx::query_scalar(
        "SELECT group_value FROM kb_saml_seen_groups WHERE idp_key = 'acme' ORDER BY group_value",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(seen, vec!["engineering".to_string(), "ghosts".to_string()]);
}

// ── The staleness record ─────────────────────────────────────────────────────
//
// Four properties, each one an acceptance criterion of
// "How stale a principal's IdP-derived reach is, is not recorded anywhere"
// (task 01a03893-e2bf-7973-b885-54978e6088f6). Every one of them fails against the code as it
// stood before that task: there was no `kb_saml_principal_reconcile` to select from, and the
// no-signal case never reached this crate at all.

/// When reach was last brought into agreement is a fact on disk, not something to reconstruct.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_reconcile_records_when_reach_was_brought_into_agreement(pool: PgPool) {
    let (profile, _team_a, _team_b) = seed(&pool).await;

    assert_eq!(
        record(&pool, profile, "acme").await,
        None,
        "no record before anything has happened"
    );

    reconcile_idp_memberships(&pool, profile, "acme", Some(&["engineering".into()]))
        .await
        .unwrap();

    let (reconciled, skipped) = record(&pool, profile, "acme").await.expect("a record");
    assert!(reconciled.is_some(), "the agreement is timestamped");
    assert!(skipped.is_none(), "nothing was skipped");
}

/// A skip is not a success, and does not become one by leaving the success column alone.
///
/// This is the criterion the `null` guard's silence used to defeat: the memberships after a skip
/// are byte-identical to the memberships after a reconcile that changed nothing, so the membership
/// rows can never tell the two apart. The record can.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_missing_group_signal_records_a_skip_rather_than_a_reconcile(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;

    let outcome = reconcile_idp_memberships(&pool, profile, "acme", None)
        .await
        .unwrap();
    assert!(matches!(outcome, ReconcileOutcome::SignalMissing));

    let (reconciled, skipped) = record(&pool, profile, "acme").await.expect("a record");
    assert!(skipped.is_some(), "the declined attempt is timestamped");
    assert!(
        reconciled.is_none(),
        "a skip must never read as an agreement"
    );
    assert_eq!(
        membership(&pool, team_a, profile).await,
        None,
        "and nothing was provisioned by a skip"
    );
}

/// Never-reconciled and reconciled-long-ago do not share a representation.
///
/// `NULL` is the whole of "never": there is no sentinel timestamp, no epoch, nothing an arithmetic
/// staleness comparison could accidentally treat as very old but real.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn never_reconciled_is_distinguishable_from_reconciled_long_ago(pool: PgPool) {
    let (never, _team_a, _team_b) = seed(&pool).await;
    let ever: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("ever-{}", Uuid::now_v7()))
    .fetch_one(&pool)
    .await
    .unwrap();

    // One has only ever presented assertions with no group signal; the other reconciled once.
    reconcile_idp_memberships(&pool, never, "acme", None)
        .await
        .unwrap();
    reconcile_idp_memberships(&pool, ever, "acme", Some(&["engineering".into()]))
        .await
        .unwrap();

    let (never_reconciled, _) = record(&pool, never, "acme").await.expect("a record");
    let (ever_reconciled, _) = record(&pool, ever, "acme").await.expect("a record");
    assert!(never_reconciled.is_none());
    assert!(ever_reconciled.is_some());
}

/// The guard suspends de-provisioning for as long as the signal stays missing — and says so.
///
/// The criterion names this case explicitly, and it is the one worth pinning: repeated logins
/// carrying no group signal must not accumulate into a revocation, must not refresh the agreement
/// timestamp, and must leave a trail whose most recent entry is the skip. The IdP's group mapping
/// is deleted first so that a reconcile, had one run, would certainly have revoked — which is what
/// makes the surviving membership evidence about the guard rather than about the mapping.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_missing_signal_suspends_de_provisioning_for_as_long_as_it_is_missing(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;

    reconcile_idp_memberships(&pool, profile, "acme", Some(&["engineering".into()]))
        .await
        .unwrap();
    let (agreed_at, _) = record(&pool, profile, "acme").await.expect("a record");
    assert!(membership(&pool, team_a, profile).await.is_some());

    sqlx::query("DELETE FROM kb_saml_group_mappings WHERE idp_key = 'acme'")
        .execute(&pool)
        .await
        .unwrap();

    // Three further logins, none of them carrying a group signal.
    for _ in 0..3 {
        reconcile_idp_memberships(&pool, profile, "acme", None)
            .await
            .unwrap();
    }

    assert!(
        membership(&pool, team_a, profile).await.is_some(),
        "reach the provider stopped asserting is still held — the guard, working"
    );
    let (still_agreed_at, skipped) = record(&pool, profile, "acme").await.expect("a record");
    assert_eq!(
        still_agreed_at, agreed_at,
        "and no skip moved the moment agreement was last reached"
    );
    assert!(
        skipped.unwrap() > agreed_at.unwrap(),
        "so the record reads: agreed once, and nothing has been compared since"
    );
}

/// The view answers the question for a principal holding reach that has no record at all.
///
/// This is the population an inner join from the record outward silently omits — and it is the
/// population the question is most about, since it includes every principal whose reach predates
/// the record. Seeded by writing the membership directly, which is exactly how such a row got there.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_view_shows_reach_held_with_no_recorded_reconcile(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) VALUES ($1,$2,'member','idp')",
    )
    .bind(team_a)
    .bind(profile)
    .execute(&pool)
    .await
    .unwrap();

    let row = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>, bool)>(
        "SELECT idp_memberships, last_reconciled_at, last_signal_was_missing
           FROM vw_saml_reconcile_staleness WHERE profile_id = $1",
    )
    .bind(profile)
    .fetch_one(&pool)
    .await
    .expect("the principal is visible despite having no reconcile record");

    assert_eq!(row.0, 1, "one idp-derived membership is held");
    assert!(row.1.is_none(), "and no agreement is recorded for it");
    assert!(!row.2);
}
