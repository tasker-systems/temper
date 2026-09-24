#![cfg(feature = "test-db")]
//! The operator reconcile verb's SAML interplay (`20260923000010`): when the repair writes
//! native rows on a team that ALSO carries `kb_saml_group_mappings` rows, the outcome names
//! that team. The IdP's role assertions for those (team, profile) pairs are permanently
//! pre-empted from then on — `reconcile_idp_memberships` skips any pair the profile holds
//! natively — so the operator must see the conversion the repair is making. A touched team
//! with no mapping names nothing, and a converged instance names no teams at all.

use sqlx::PgPool;
use temper_services::auth::{require_system_admin, SystemAdmin};
use temper_services::services::access_service;
use temper_services::test_support;
use uuid::Uuid;

async fn a_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Mint the `SystemAdmin` proof for an admin profile — the capability the acts require.
async fn admin_proof(pool: &PgPool, admin_id: Uuid) -> SystemAdmin {
    let a = test_support::authenticated_profile_for(pool, admin_id).await;
    require_system_admin(pool, &a)
        .await
        .expect("admin mints a proof")
}

/// Simulate the drift the verb repairs: an approval that predates a team's flag. The
/// committer enrolls into the teams flagged at approval time, so a team flagged afterwards
/// by bare SQL (no `backfill_auto_join_team` run) misses every prior approval.
async fn approve(pool: &PgPool, profile: Uuid) {
    sqlx::query("SELECT principal_standing_apply($1,'provision','approved',NULL,'test approve')")
        .bind(profile)
        .execute(pool)
        .await
        .unwrap();
}

async fn flagged_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) \
         VALUES ($1, $1, 'watcher'::team_role) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reconcile_names_saml_mapped_teams_it_touched(pool: PgPool) {
    let admin = a_profile(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let proof = admin_proof(&pool, admin).await;

    // kate approved while only temper-system is flagged; the committer enrolls her there.
    let kate = a_profile(&pool, "kate").await;
    approve(&pool, kate).await;

    // Two teams flagged afterwards by bare SQL — the drift. `eng` carries a SAML group
    // mapping; `ops` does not.
    let eng = flagged_team(&pool, "eng").await;
    flagged_team(&pool, "ops").await;
    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, \
         sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr) \
         VALUES ('https://idp.example', true, 'cert', 'https://idp.example/sso', \
                 'https://idp.example/entity', 'https://temper.example/sp', \
                 'https://temper.example/acs', 'urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress', \
                 'email', 'stable-id')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) \
         VALUES ('https://idp.example', 'eng-group', $1, 'watcher')",
    )
    .bind(eng)
    .execute(&pool)
    .await
    .unwrap();

    let outcome = access_service::reconcile_auto_join(&pool, &proof)
        .await
        .unwrap();

    assert!(
        outcome
            .added
            .iter()
            .any(|r| r.team_slug == "eng" && r.profile_handle == "kate"),
        "the drift repair adds the (eng, kate) pair"
    );
    assert!(
        outcome
            .added
            .iter()
            .any(|r| r.team_slug == "ops" && r.profile_handle == "kate"),
        "the drift repair adds the (ops, kate) pair"
    );
    assert_eq!(
        outcome.saml_mapped_teams,
        vec!["eng".to_string()],
        "the outcome names exactly the touched teams that carry SAML group mappings — the \
         IdP's role assertions for the pairs written there are pre-empted from here on"
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn converged_instance_names_nothing(pool: PgPool) {
    let admin = a_profile(&pool, "admin").await;
    test_support::approved_admin(&pool, admin).await;
    let proof = admin_proof(&pool, admin).await;

    // The first converge is not necessarily pair-free on a fresh install — the boot-seeded
    // `system` profile holds standing granted by the frozen backfill, outside every door,
    // so the sweep adds it. No mappings exist here, so nothing is named either way.
    let first = access_service::reconcile_auto_join(&pool, &proof)
        .await
        .unwrap();
    assert_eq!(
        first.saml_mapped_teams,
        Vec::<String>::new(),
        "with no SAML mappings anywhere, nothing is named"
    );

    // After it the instance is converged: no pairs, no names.
    let second = access_service::reconcile_auto_join(&pool, &proof)
        .await
        .unwrap();
    assert!(second.added.is_empty(), "a converged instance adds nothing");
    assert!(
        second.saml_mapped_teams.is_empty(),
        "a converged instance names no teams"
    );
}
