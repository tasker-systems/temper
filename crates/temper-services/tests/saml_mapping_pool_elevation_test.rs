#![cfg(feature = "test-db")]
//! A SAML group mapping onto a team whose subtree holds a pool needs the admin's confirmation, and a
//! structural team act that puts a pool beneath an already-mapped team answers and records each
//! mapping it elevated (migration `20261126000010`).
//!
//! Reads flow down the team DAG, so a pool's members (the root's, the gating team's, an auto-join
//! team's: every approved profile) read every team above it. An IdP group mapped with an authoring
//! role (owner, maintainer, member) onto a pool's ancestor writes what the whole pool reads. The
//! witnesses here:
//!
//! - a `member` mapping onto a pool's parent is refused without confirmation and writes nothing;
//!   with it, it succeeds and is recorded as confirmed;
//! - a `watcher` mapping onto a pool's parent, the gating team or the root needs none;
//! - a dead team between the target and the pool still counts;
//! - the gating repoint, attach, reparent, set_auto_join and creating an auto-join team under a
//!   parent each answer the mappings they elevated, with exactly one `elevated` record each;
//! - an act that elevates nothing (restore, a plain attach, an already-pooled team) answers an
//!   empty list and records nothing;
//! - a concurrent act that holds a row in the target's subtree makes the create wait, and the
//!   create then refuses rather than miss the pool the act brought;
//! - a create waiting on its target's row holds no gating lock, so a rename of the target (row,
//!   then the exclusive gating lock) does not deadlock with it.

use sqlx::PgPool;
use temper_core::types::saml_mapping::{CreateSamlGroupMappingRequest, SamlGroupMapping};
use temper_core::types::team::{ReparentRequest, TeamCreateRequest, TeamRole};
use temper_services::error::ApiError;
use temper_services::services::saml_provisioning_service::{
    reconcile_idp_memberships, ReconcileOutcome,
};
use temper_services::services::{access_service, saml_mapping_service, team_service};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

const IDP: &str = "pe-okta";

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile");
    for surface in Surface::ALL {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{}", surface.marker()))
        .execute(pool)
        .await
        .expect("seed emitter");
    }
    id
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) \
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("team")
}

/// A fixture link, written directly (the acts under test go through the doors).
async fn link(pool: &PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link");
}

async fn auto_join(pool: &PgPool, team: Uuid) {
    sqlx::query("UPDATE kb_teams SET auto_join_role = 'member' WHERE id = $1")
        .bind(team)
        .execute(pool)
        .await
        .expect("auto-join");
}

async fn kill(pool: &PgPool, team: Uuid) {
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(team)
        .execute(pool)
        .await
        .expect("soft-delete");
}

struct Fixture {
    admin: Uuid,
    root: Uuid,
    gating: Uuid,
    /// org has an auto-join child, `pool`: org is a pool's parent.
    org: Uuid,
    /// upper → mid (dead) → aj2 (auto-join): the pool sits below a dead team.
    upper: Uuid,
    /// No pool anywhere below.
    plain: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    let admin = profile(pool, "pe-admin").await;
    test_support::approved_admin(pool, admin).await;
    let root = team(pool, "temper-system").await;
    let gating = team(pool, "pe-gate").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'pe-gate' WHERE id = 1")
        .execute(pool)
        .await
        .expect("point the gating slug");
    let org = team(pool, "pe-org").await;
    let aj = team(pool, "pe-pool").await;
    auto_join(pool, aj).await;
    link(pool, org, aj).await;
    let upper = team(pool, "pe-upper").await;
    let mid = team(pool, "pe-mid").await;
    let aj2 = team(pool, "pe-pool2").await;
    auto_join(pool, aj2).await;
    link(pool, upper, mid).await;
    link(pool, mid, aj2).await;
    kill(pool, mid).await;
    let plain = team(pool, "pe-plain").await;
    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, \
                                  sp_entity_id, acs_url, nameid_format, email_attr, \
                                  stable_id_attr, groups_attr) \
         VALUES ($1, true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', \
                 'email', 'uid', 'groups')",
    )
    .bind(IDP)
    .execute(pool)
    .await
    .expect("seed idp");
    Fixture {
        admin,
        root,
        gating,
        org,
        upper,
        plain,
    }
}

fn req(team_id: Uuid, group: &str, role: TeamRole, confirm: bool) -> CreateSamlGroupMappingRequest {
    CreateSamlGroupMappingRequest {
        idp_key: IDP.to_string(),
        group: group.to_string(),
        team_id,
        role,
        confirm_elevated: confirm,
    }
}

fn mapping(team_id: Uuid, group: &str, role: TeamRole) -> SamlGroupMapping {
    SamlGroupMapping {
        idp_key: IDP.to_string(),
        group: group.to_string(),
        team_id,
        role,
    }
}

/// Map `group` onto `team` as `role`, confirming, as a fixture step.
async fn map(pool: &PgPool, admin: Uuid, team: Uuid, group: &str, role: TeamRole) {
    let proof = test_support::system_admin_proof_for(pool, admin).await;
    saml_mapping_service::create(pool, &proof, &req(team, group, role, true))
        .await
        .expect("fixture mapping");
}

async fn mapping_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_saml_group_mappings")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Every `saml_group_mapping_changed` payload with act `act`, oldest first.
async fn records(pool: &PgPool, act: &str) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'saml_group_mapping_changed' AND e.payload->>'act' = $1 ORDER BY e.id",
    )
    .bind(act)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn assert_one_elevated(recs: &[serde_json::Value], actor: Uuid, m: &SamlGroupMapping) {
    let hits: Vec<_> = recs
        .iter()
        .filter(|r| r["group_value"] == m.group.as_str() && r["team_id"] == m.team_id.to_string())
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "exactly one elevated record for {m:?}: {recs:?}"
    );
    assert_eq!(
        *hits[0],
        serde_json::json!({
            "act": "elevated",
            "actor": actor,
            "idp_key": IDP,
            "group_value": m.group,
            "team_id": m.team_id,
            "role": m.role.as_str(),
        })
    );
    serde_json::from_value::<temper_substrate::payloads::SamlGroupMappingChanged>(hits[0].clone())
        .expect("the record decodes as the typed payload");
}

// ── The create-time predicate ────────────────────────────────────────────────────────────────────

/// FAILS IF the predicate is reduced to the root and the gating team: org is neither, only an
/// auto-join team's parent.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_member_mapping_onto_a_pools_parent_needs_confirmation(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;

    for role in [TeamRole::Member, TeamRole::Maintainer] {
        let err = saml_mapping_service::create(&pool, &admin, &req(f.org, "org", role, false))
            .await
            .unwrap_err();
        match err {
            ApiError::BadRequest(m) => {
                assert!(m.contains("confirmation required"), "{m}");
                assert!(m.contains("a pool beneath it"), "{m}");
            }
            other => panic!("expected a named confirmation refusal, got {other:?}"),
        }
    }
    assert_eq!(
        mapping_count(&pool).await,
        0,
        "a refused create wrote a row"
    );
    assert!(records(&pool, "created").await.is_empty());

    let out =
        saml_mapping_service::create(&pool, &admin, &req(f.org, "org", TeamRole::Member, true))
            .await
            .expect("a confirmed pool-parent mapping");
    assert!(out.created && out.confirmed);
    let recs = records(&pool, "created").await;
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0]["confirmed"], true, "{}", recs[0]);

    // A team with no pool beneath it needs none.
    let plain = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.plain, "plain", TeamRole::Maintainer, false),
    )
    .await
    .unwrap();
    assert!(plain.created && !plain.confirmed);
}

/// A watcher reads only, so it needs no confirmation anywhere: a pool's parent, the gating team, or
/// the root. This is the one relaxation of the door's first rule.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_watcher_mapping_needs_no_confirmation_even_onto_an_instance_team(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    for (team, group) in [(f.org, "org-ro"), (f.gating, "gate-ro"), (f.root, "all-ro")] {
        let out = saml_mapping_service::create(
            &pool,
            &admin,
            &req(team, group, TeamRole::Watcher, false),
        )
        .await
        .unwrap_or_else(|e| panic!("watcher onto {group}: {e:?}"));
        assert!(out.created && !out.confirmed, "{group}");
    }
    let recs = records(&pool, "created").await;
    assert_eq!(recs.len(), 3);
    assert!(
        recs.iter().all(|r| r.get("confirmed").is_none()),
        "{recs:?}"
    );
}

/// A pool below a dead team is latent reach (a restore revives it), so it counts.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_dead_team_between_the_target_and_the_pool_still_counts(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    let err = saml_mapping_service::create(
        &pool,
        &admin,
        &req(f.upper, "upper", TeamRole::Member, false),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(m) if m.contains("a pool beneath it")),
        "{err:?}"
    );
    assert_eq!(mapping_count(&pool).await, 0);
}

// ── Elevation ────────────────────────────────────────────────────────────────────────────────────

/// FAILS IF the repoint's elevation scan is removed. host ← cand: repointing the gating team to
/// cand puts a pool beneath host and its parent top. The watcher mapping, and the mapping on an
/// unrelated team, are not elevated; a repeat (noop) elevates nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_gating_repoint_answers_and_records_the_mappings_it_elevated(pool: PgPool) {
    let f = fixture(&pool).await;
    let top = team(&pool, "pe-top").await;
    let host = team(&pool, "pe-host").await;
    let cand = team(&pool, "pe-cand").await;
    link(&pool, top, host).await;
    link(&pool, host, cand).await;
    map(&pool, f.admin, host, "host", TeamRole::Member).await;
    map(&pool, f.admin, top, "top", TeamRole::Owner).await;
    map(&pool, f.admin, host, "host-ro", TeamRole::Watcher).await;
    map(&pool, f.admin, f.plain, "plain", TeamRole::Member).await;

    let proof = test_support::system_admin_proof_for(&pool, f.admin).await;
    let repoint = temper_core::types::admin::UpdateSettingsRequest {
        gating_team_slug: Some("pe-cand".to_owned()),
        ..Default::default()
    };
    let out = access_service::update_system_settings(&pool, &proof, &repoint)
        .await
        .expect("repoint");
    let host_m = mapping(host, "host", TeamRole::Member);
    let top_m = mapping(top, "top", TeamRole::Owner);
    assert_eq!(out.elevated_mappings, vec![host_m.clone(), top_m.clone()]);
    let recs = records(&pool, "elevated").await;
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_one_elevated(&recs, f.admin, &host_m);
    assert_one_elevated(&recs, f.admin, &top_m);

    // The same slug again opens nothing.
    let again = access_service::update_system_settings(&pool, &proof, &repoint)
        .await
        .unwrap();
    assert!(again.elevated_mappings.is_empty());
    assert_eq!(records(&pool, "elevated").await.len(), 2);
}

/// FAILS IF the attach's elevation scan is removed. Attaching an auto-join team under host elevates
/// host's mapping and its parent's; attaching a plain team elevates nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn attaching_an_auto_join_child_answers_and_records_each_elevated_mapping(pool: PgPool) {
    let f = fixture(&pool).await;
    let top = team(&pool, "pe-top").await;
    let host = team(&pool, "pe-host").await;
    link(&pool, top, host).await;
    let aj = team(&pool, "pe-aj").await;
    auto_join(&pool, aj).await;
    let leaf = team(&pool, "pe-leaf").await;
    map(&pool, f.admin, host, "host", TeamRole::Maintainer).await;
    map(&pool, f.admin, top, "top", TeamRole::Member).await;
    let admin_p = test_support::human_principal_for(&pool, f.admin).await;

    let plain = team_service::attach_child(&pool, &admin_p, host, leaf)
        .await
        .expect("plain attach");
    assert!(
        plain.elevated_mappings.is_empty(),
        "a plain child is no pool"
    );
    assert!(records(&pool, "elevated").await.is_empty());

    let out = team_service::attach_child(&pool, &admin_p, host, aj)
        .await
        .expect("attach the auto-join team");
    let host_m = mapping(host, "host", TeamRole::Maintainer);
    let top_m = mapping(top, "top", TeamRole::Member);
    assert_eq!(out.elevated_mappings, vec![host_m.clone(), top_m.clone()]);
    let recs = records(&pool, "elevated").await;
    assert_eq!(recs.len(), 2, "{recs:?}");
    assert_one_elevated(&recs, f.admin, &host_m);
    assert_one_elevated(&recs, f.admin, &top_m);

    // A second pool under an already-pooled team elevates nothing again.
    let aj3 = team(&pool, "pe-aj3").await;
    auto_join(&pool, aj3).await;
    let again = team_service::attach_child(&pool, &admin_p, host, aj3)
        .await
        .unwrap();
    assert!(again.elevated_mappings.is_empty());
    assert_eq!(records(&pool, "elevated").await.len(), 2);
}

/// Setting auto-join on a descendant puts a pool beneath the mapped team.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn setting_auto_join_on_a_descendant_elevates_the_mapping_above_it(pool: PgPool) {
    let f = fixture(&pool).await;
    let host = team(&pool, "pe-host").await;
    let kid = team(&pool, "pe-kid").await;
    link(&pool, host, kid).await;
    map(&pool, f.admin, host, "host", TeamRole::Member).await;
    let proof = test_support::system_admin_proof_for(&pool, f.admin).await;

    let out = team_service::set_auto_join(&pool, &proof, kid, Some(TeamRole::Watcher))
        .await
        .expect("set auto-join");
    let host_m = mapping(host, "host", TeamRole::Member);
    assert_eq!(out.elevated_mappings, vec![host_m.clone()]);
    let recs = records(&pool, "elevated").await;
    assert_eq!(recs.len(), 1);
    assert_one_elevated(&recs, f.admin, &host_m);

    // Changing the role of a team already a pool elevates nothing.
    let again = team_service::set_auto_join(&pool, &proof, kid, Some(TeamRole::Member))
        .await
        .unwrap();
    assert!(again.elevated_mappings.is_empty());
    assert_eq!(records(&pool, "elevated").await.len(), 1);
}

/// Reparenting a pool under a mapped team elevates it, and the record carries the move's
/// correlation id.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reparenting_a_pool_under_a_mapped_team_elevates_it(pool: PgPool) {
    let f = fixture(&pool).await;
    let from = team(&pool, "pe-from").await;
    let to = team(&pool, "pe-to").await;
    let aj = team(&pool, "pe-aj").await;
    auto_join(&pool, aj).await;
    link(&pool, from, aj).await;
    map(&pool, f.admin, to, "to", TeamRole::Member).await;
    let admin_p = test_support::human_principal_for(&pool, f.admin).await;

    let out = team_service::reparent(
        &pool,
        &admin_p,
        aj,
        &ReparentRequest {
            from: from.to_string(),
            to: to.to_string(),
        },
    )
    .await
    .expect("reparent");
    let to_m = mapping(to, "to", TeamRole::Member);
    assert_eq!(out.elevated_mappings, vec![to_m.clone()]);
    let recs = records(&pool, "elevated").await;
    assert_one_elevated(&recs, f.admin, &to_m);
    let correlated: Option<Uuid> = sqlx::query_scalar(
        "SELECT e.correlation_id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'saml_group_mapping_changed' AND e.payload->>'act' = 'elevated'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(correlated, Some(out.correlation_id));
}

/// Creating an auto-join team under a parent elevates the parent's mapping: recorded always, and
/// answered on `?answer=effects` (the service's `CreatedTeam`).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn creating_an_auto_join_team_under_a_mapped_parent_elevates_it(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin_p = test_support::human_principal_for(&pool, f.admin).await;
    // Created through the door, so the admin owns it: a child takes a manager of its parent.
    let host = team_service::create_team(
        &pool,
        &admin_p,
        &TeamCreateRequest {
            slug: "pe-host".to_owned(),
            name: None,
            parent: None,
            parents: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("host")
    .id;
    map(&pool, f.admin, host, "host", TeamRole::Member).await;

    let plain = team_service::create_team_with_effects(
        &pool,
        &admin_p,
        &TeamCreateRequest {
            slug: "pe-new-plain".to_owned(),
            name: None,
            parent: Some(host.to_string()),
            parents: None,
            auto_join_role: None,
        },
    )
    .await
    .expect("plain create");
    assert!(plain.elevated_mappings.is_empty());

    let created = team_service::create_team_with_effects(
        &pool,
        &admin_p,
        &TeamCreateRequest {
            slug: "pe-new-pool".to_owned(),
            name: None,
            parent: Some(host.to_string()),
            parents: None,
            auto_join_role: Some(TeamRole::Member),
        },
    )
    .await
    .expect("create an auto-join team");
    let host_m = mapping(host, "host", TeamRole::Member);
    assert_eq!(created.elevated_mappings, vec![host_m.clone()]);
    assert_one_elevated(&records(&pool, "elevated").await, f.admin, &host_m);
}

/// Restore flips `is_active` only, and dead teams already count, so it elevates nothing: the
/// mapping above a dead pool needed confirmation when it was made.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn restoring_a_dead_team_above_a_pool_elevates_nothing(pool: PgPool) {
    let f = fixture(&pool).await;
    map(&pool, f.admin, f.upper, "upper", TeamRole::Member).await;
    let mid: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'pe-mid'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let admin_p = test_support::human_principal_for(&pool, f.admin).await;
    let out = team_service::restore_team(&pool, &admin_p, mid)
        .await
        .expect("restore");
    assert!(out.elevated_mappings.is_empty());
    assert!(records(&pool, "elevated").await.is_empty());
}

// ── Locks ────────────────────────────────────────────────────────────────────────────────────────

/// Wait until some backend is blocked on a lock `holder` holds.
async fn await_blocked_by(pool: &PgPool, holder: i32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                             WHERE datname = current_database() AND wait_event_type = 'Lock' \
                               AND $1 = ANY (pg_blocking_pids(pid)))",
        )
        .bind(holder)
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the create never blocked on the holder"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

async fn holder(pool: &PgPool) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    (tx, pid)
}

/// The race the subtree lock closes. An act holding a row below the target (as attach holds the
/// parent it links under) links a pool there and commits while the create waits; the create must
/// not decide on the subtree it walked before. It refuses (retryable), and the retry sees the pool.
///
/// FAILS IF the create locks only its target row: it would not wait, would find no pool, and would
/// write an unconfirmed mapping the act's elevation scan never saw.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_pool_linked_below_the_target_during_the_create_is_not_missed(pool: PgPool) {
    let f = fixture(&pool).await;
    let host = team(&pool, "pe-host").await;
    let kid = team(&pool, "pe-kid").await;
    link(&pool, host, kid).await;
    let aj = team(&pool, "pe-aj").await;
    auto_join(&pool, aj).await;

    let (mut act, pid) = holder(&pool).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(kid)
        .execute(&mut *act)
        .await
        .unwrap();

    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    let create = tokio::spawn({
        let pool = pool.clone();
        async move {
            saml_mapping_service::create(&pool, &admin, &req(host, "host", TeamRole::Member, false))
                .await
        }
    });
    await_blocked_by(&pool, pid).await;
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(kid)
        .bind(aj)
        .execute(&mut *act)
        .await
        .unwrap();
    act.commit().await.unwrap();

    let err = create.await.unwrap().unwrap_err();
    assert!(
        matches!(&err, ApiError::Conflict(m) if m.contains("retry")),
        "{err:?}"
    );
    assert_eq!(mapping_count(&pool).await, 0, "nothing written");

    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    let retry =
        saml_mapping_service::create(&pool, &admin, &req(host, "host", TeamRole::Member, false))
            .await
            .unwrap_err();
    assert!(
        matches!(&retry, ApiError::BadRequest(m) if m.contains("a pool beneath it")),
        "{retry:?}"
    );
}

/// A rename holds its team row, then takes the gating lock exclusively. A create of a mapping onto
/// that team waits on the row holding no gating lock, so the rename completes and the create then
/// proceeds.
///
/// FAILS IF the create takes the shared gating lock before the team row (the first door's order):
/// the two wait on each other and one is aborted as a deadlock.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_create_waiting_on_its_target_does_not_deadlock_a_rename(pool: PgPool) {
    let f = fixture(&pool).await;
    let (mut rename, pid) = holder(&pool).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(f.plain)
        .execute(&mut *rename)
        .await
        .unwrap();

    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    let create = tokio::spawn({
        let pool = pool.clone();
        let plain = f.plain;
        async move {
            saml_mapping_service::create(&pool, &admin, &req(plain, "p", TeamRole::Member, false))
                .await
        }
    });
    await_blocked_by(&pool, pid).await;
    sqlx::query("SELECT _gating_team_lock(true)")
        .execute(&mut *rename)
        .await
        .expect("the rename's exclusive gating lock is not held up by the waiting create");
    rename.commit().await.unwrap();

    let out = create.await.unwrap().expect("the create proceeds");
    assert!(out.created && !out.confirmed);
}

// ── The elevation scan's scope ──────────────────────────────────────────────────────────────────

/// An act's elevation diff covers only mappings at or above the teams it writes at. A mapping onto
/// another pooled team that commits between the act's two reads (simulated here, between the
/// before-read and the record, on one connection) is not the act's to answer or record.
///
/// FAILS IF the diff is taken over the whole instance: the unrelated mapping would be recorded as
/// `elevated`, under the act's actor.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_elevation_diff_is_scoped_to_the_acts_own_teams(pool: PgPool) {
    let f = fixture(&pool).await;
    let host = team(&pool, "pe-host").await;
    let emitter: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_entities WHERE profile_id = $1 ORDER BY name LIMIT 1",
    )
    .bind(f.admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let before: serde_json::Value = sqlx::query_scalar("SELECT _saml_mappings_over_pool($1)")
        .bind(vec![host])
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    // A concurrent, unrelated confirmed mapping onto org, a pool's parent.
    map(&pool, f.admin, f.org, "org", TeamRole::Member).await;
    let recorded: serde_json::Value =
        sqlx::query_scalar("SELECT _saml_mapping_elevations_record($1, $2, $3, $4, NULL)")
            .bind(emitter)
            .bind(f.admin)
            .bind(vec![host])
            .bind(before)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(recorded, serde_json::json!([]));
    assert!(records(&pool, "elevated").await.is_empty());
}

/// An authoring create onto temper-system locks the root row only, never its subtree (every team),
/// and is refused (watcher only): a concurrent act holding a team below the root does not hold it up.
///
/// FAILS IF the root's subtree is locked: the create waits on the held row past the timeout.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_create_onto_the_root_locks_only_the_root_row(pool: PgPool) {
    let f = fixture(&pool).await;
    link(&pool, f.root, f.plain).await;
    let (mut act, _pid) = holder(&pool).await;
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(f.plain)
        .execute(&mut *act)
        .await
        .unwrap();
    let admin = test_support::system_admin_proof_for(&pool, f.admin).await;
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        saml_mapping_service::create(&pool, &admin, &req(f.root, "all", TeamRole::Member, true)),
    )
    .await
    .expect("the create does not wait on a team below the root")
    .unwrap_err();
    assert!(
        matches!(&err, ApiError::BadRequest(m) if m.contains("at most `watcher`")),
        "{err:?}"
    );
    act.rollback().await.unwrap();
}

// ── The instance clamp (Pete, 2026-11-12) ───────────────────────────────────────────────────────

/// A mapping row onto temper-system or the gating team above `watcher`, however it was written
/// (before the rule, or by SQL), confers `watcher` at the reconcile; an IdP row already held above
/// it is lowered. A mapping onto an ordinary team still confers its role.
///
/// FAILS IF the reconcile confers the row's role on an instance team.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_reconcile_confers_watcher_only_on_an_instance_team(pool: PgPool) {
    let f = fixture(&pool).await;
    let person = profile(&pool, "pe-person").await;
    for (group, team, role) in [
        ("admins", f.gating, "owner"),
        ("everyone", f.root, "member"),
        ("plain", f.plain, "maintainer"),
    ] {
        sqlx::query(
            "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) \
             VALUES ($1, $2, $3, $4::team_role)",
        )
        .bind(IDP)
        .bind(group)
        .bind(team)
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    let groups = vec![
        "admins".to_string(),
        "everyone".to_string(),
        "plain".to_string(),
    ];
    let ReconcileOutcome::Reconciled(counts) =
        reconcile_idp_memberships(&pool, person, IDP, Some(&groups))
            .await
            .expect("reconcile")
    else {
        panic!("expected a reconcile");
    };
    assert_eq!(counts.clamped, 2, "{counts:?}");
    let role = |team: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            )
            .bind(team)
            .bind(person)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(role(f.gating).await, "watcher");
    assert_eq!(role(f.root).await, "watcher");
    assert_eq!(role(f.plain).await, "maintainer");
}

/// A repoint that makes a mapped team the gating team records each mapping above `watcher` onto it
/// as `clamped`, naming the repoint's actor, answers it in `clamped_mappings`, and does not answer it
/// as elevated. A mapping onto the team's parent is elevated as before.
///
/// FAILS IF the repoint records no clamp, or reports the new gating team's own mapping as elevated.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_gating_repoint_records_the_mappings_it_clamps(pool: PgPool) {
    let f = fixture(&pool).await;
    let top = team(&pool, "pe-top").await;
    let cand = team(&pool, "pe-cand").await;
    link(&pool, top, cand).await;
    map(&pool, f.admin, cand, "cand", TeamRole::Member).await;
    map(&pool, f.admin, cand, "cand-ro", TeamRole::Watcher).await;
    map(&pool, f.admin, top, "top", TeamRole::Member).await;

    let proof = test_support::system_admin_proof_for(&pool, f.admin).await;
    let out = access_service::update_system_settings(
        &pool,
        &proof,
        &temper_core::types::admin::UpdateSettingsRequest {
            gating_team_slug: Some("pe-cand".to_owned()),
            ..Default::default()
        },
    )
    .await
    .expect("repoint");
    let cand_m = mapping(cand, "cand", TeamRole::Member);
    assert_eq!(out.clamped_mappings, vec![cand_m.clone()]);
    assert_eq!(
        out.elevated_mappings,
        vec![mapping(top, "top", TeamRole::Member)]
    );
    let recs = records(&pool, "clamped").await;
    assert_eq!(recs.len(), 1, "{recs:?}");
    assert_eq!(
        recs[0],
        serde_json::json!({
            "act": "clamped",
            "actor": f.admin,
            "idp_key": IDP,
            "group_value": "cand",
            "team_id": cand,
            "role": "member",
        })
    );
    serde_json::from_value::<temper_substrate::payloads::SamlGroupMappingChanged>(recs[0].clone())
        .expect("the record decodes as the typed payload");
}

/// The migration's Section 4, run against seeded rows: each mapping above `watcher` onto
/// temper-system or the gating team gets exactly one `clamped` record naming the migration, with no
/// person actor; a watcher row and an ordinary team's row get none; no row changes. And nothing is
/// recorded as confirmed (the B′ extension was withdrawn, Pete, 2026-11-12).
///
/// FAILS IF the section records any other row, records one twice, or changes a row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_migration_records_existing_instance_mappings_as_clamped(pool: PgPool) {
    use sqlx::Executor;
    let f = fixture(&pool).await;
    for (team, group, role) in [
        (f.root, "admins", "owner"),
        (f.gating, "staff", "member"),
        (f.gating, "staff-ro", "watcher"),
        (f.org, "org", "member"),
    ] {
        sqlx::query(
            "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) \
             VALUES ($1, $2, $3, $4::team_role)",
        )
        .bind(IDP)
        .bind(group)
        .bind(team)
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    let migration =
        include_str!("../../../migrations/20261126000010_saml_mapping_pool_confirmation.sql");
    let rest = migration
        .split_once("-- Section 4.")
        .and_then(|(_, rest)| rest.split_once("SELECT declare_migration("))
        .map(|(body, _)| body)
        .expect("the migration carries its Section 4");
    let block = &rest[rest.find("DO $$").expect("Section 4 is a DO block")..];
    let before = mapping_count(&pool).await;
    pool.execute(block).await.expect("Section 4 never refuses");

    assert_eq!(mapping_count(&pool).await, before, "no row changed");
    let recs = records(&pool, "clamped").await;
    let groups: Vec<&str> = recs
        .iter()
        .map(|r| r["group_value"].as_str().unwrap())
        .collect();
    assert_eq!(groups, vec!["admins", "staff"], "{recs:?}");
    for r in &recs {
        assert_eq!(r["migration"], 20261126000010_i64, "{r}");
        assert!(r.get("actor").is_none(), "no person actor: {r}");
        assert!(
            r.get("confirmed").is_none(),
            "a clamp confirms nothing: {r}"
        );
    }
    assert!(records(&pool, "adopted").await.is_empty());
}
