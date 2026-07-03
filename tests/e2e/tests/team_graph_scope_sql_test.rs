//! SQL-level semantics for the R1 team-graph-scope functions (Chunk A).
//! Proves the new functions directly against the migrated schema — fast feedback
//! on the DAG walk + access asymmetry, before the HTTP endpoint exists (that is
//! `team_graph_scope_e2e.rs`). Access-semantics change → also gated at the e2e HTTP tier.
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    let body: serde_json::Value = resp.json().await.expect("profile json");
    body["id"].as_str().unwrap().parse().unwrap()
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("create team")
}

async fn link_parent(pool: &sqlx::PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link parent/child");
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add member");
}

/// Team-anchored read grant (the upward-transitive visibility mechanism), on the
/// current kb_access_grants store (subject = resource, principal = team).
async fn grant_read_to_team(pool: &sqlx::PgPool, resource: Uuid, team: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(resource)
    .bind(team)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("grant read to team");
}

/// team_descendants walks DOWN the DAG (mirror of team_ancestors).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_descendants_walks_down(pool: sqlx::PgPool) {
    let eng = create_team(&pool, "tgs-eng").await;
    let group = create_team(&pool, "tgs-group").await;
    let squad_a = create_team(&pool, "tgs-squad-a").await;
    link_parent(&pool, eng, group).await;
    link_parent(&pool, group, squad_a).await;

    let mut ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT team_id FROM team_descendants($1) ORDER BY team_id")
            .bind(eng)
            .fetch_all(&pool)
            .await
            .expect("team_descendants");
    let mut expected = vec![eng, group, squad_a];
    ids.sort();
    expected.sort();
    assert_eq!(ids, expected, "descendants = self + group + squad_a");
}

/// team_child_zones returns a direct child only when the profile can reach it
/// (member of the child or any of its descendants); non-reachable children are excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn child_zones_are_reachable_children_only(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;

    // eng ─ squad-a (member reaches via being in squad-a) ; eng ─ squad-b (not a member)
    let eng = create_team(&pool, "cz-eng").await;
    let squad_a = create_team(&pool, "cz-squad-a").await;
    let squad_b = create_team(&pool, "cz-squad-b").await;
    link_parent(&pool, eng, squad_a).await;
    link_parent(&pool, eng, squad_b).await;
    add_member(&pool, squad_a, member).await;

    let zones: Vec<Uuid> =
        sqlx::query_scalar("SELECT team_id FROM team_child_zones($1, $2) ORDER BY team_id")
            .bind(member)
            .bind(eng)
            .fetch_all(&pool)
            .await
            .expect("team_child_zones");
    assert_eq!(
        zones,
        vec![squad_a],
        "only squad-a is enterable; squad-b excluded"
    );
}

/// resources_in_team_scope includes a team's own bindings + ancestors,
/// and EXCLUDES a descendant's private bindings (no downward leak).
///
/// Uses TEAM READ-GRANTS (kb_access_grants), which are the upward-transitive
/// mechanism — team-OWNED contexts are deliberately flat in resources_visible_to
/// and would not demonstrate ancestor inheritance.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_scope_excludes_descendant_privates(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;

    let eng = create_team(&pool, "ts-eng").await;
    let squad_a = create_team(&pool, "ts-squad-a").await;
    link_parent(&pool, eng, squad_a).await;
    add_member(&pool, squad_a, member).await; // member reaches eng upward

    // A resource read-granted to eng (ancestor of squad-a).
    let eng_res: Uuid =
        sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ('eng doc','temper://ts/eng') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    grant_read_to_team(&pool, eng_res, eng, member).await;

    // A resource read-granted to squad-a (a DESCENDANT of eng — a "private").
    let sq_res: Uuid =
        sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ('squad doc','temper://ts/sq') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    grant_read_to_team(&pool, sq_res, squad_a, member).await;

    // Scope = eng: sees eng's own resource, NOT squad-a's (descendant private).
    let in_eng_scope: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_in_team_scope($1, $2)")
            .bind(member)
            .bind(eng)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        in_eng_scope.contains(&eng_res),
        "eng scope includes eng's own resource"
    );
    assert!(
        !in_eng_scope.contains(&sq_res),
        "eng scope EXCLUDES squad-a's private resource"
    );

    // Scope = squad-a: sees squad-a's own resource AND eng's (upward inheritance).
    let in_sq_scope: Vec<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resources_in_team_scope($1, $2)")
            .bind(member)
            .bind(squad_a)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        in_sq_scope.contains(&sq_res),
        "squad-a scope includes its own resource"
    );
    assert!(
        in_sq_scope.contains(&eng_res),
        "squad-a scope inherits eng (ancestor) upward"
    );
}
