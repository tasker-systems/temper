//! HTTP e2e for POST /api/cogmaps/{id}/graph/slice (A2) — the cogmap-door
//! analog of `graph_atlas_slice_e2e.rs`'s team acceptance gate. Proves the full
//! stack (auth + handler + deny code) agrees at the e2e access tier; test-db
//! predicate tests alone (`graph_atlas_cogmap_slice_sql_test.rs`) are a false
//! signal for access changes.
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use temper_core::types::graph::EdgeKind;
use uuid::Uuid;

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    common::approve(&app.pool, __pid).await;
    __pid
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .unwrap();
}

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// kb_cogmaps requires a telos_resource_id; create a throwaway resource for it.
async fn create_readable_cogmap(pool: &sqlx::PgPool, reader: Uuid, name: &str) -> Uuid {
    let telos = create_resource(pool, &format!("{name}-telos"), "").await;
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap();

    // Join the cogmap to a fresh team the reader is a member of — the mechanism
    // `cogmap_readable_by_profile` reads (`kb_team_cogmaps` ⋈ profile_effective_teams).
    let team = create_team(pool, &format!("{name}-team")).await;
    add_member(pool, team, reader).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();

    cogmap
}

/// Home a resource in the cogmap AND make it visible to `profile` in one shot
/// (owner_profile_id/originator_profile_id both set to `profile` satisfies
/// `resources_visible_to`'s owned/originated clause) — same pattern as
/// `graph_atlas_cogmap_slice_sql_test.rs::home_in_cogmap`.
async fn home_resource_in_cogmap(
    pool: &sqlx::PgPool,
    cogmap: Uuid,
    profile: Uuid,
    title: &str,
) -> Uuid {
    let resource = create_resource(pool, title, &format!("temper://cm-slice-e2e/{title}")).await;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(resource)
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home resource in cogmap");
    resource
}

/// Any pre-existing kb_events row (the L0 kernel cogmap genesis migration inserts
/// one) — sufficient FK target for asserted_by_event_id/last_event_id in these tests.
async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one kb_events row exists (L0 genesis)")
}

/// Assert an INCOMING edge to `seed` (source = nbr, target = seed), homed in
/// the cogmap itself — exercises the bidirectional-walk guarantee the same way
/// `graph_atlas_slice_e2e.rs`'s A1 test does for the team-scoped stack.
async fn assert_incoming_edge(pool: &sqlx::PgPool, nbr: Uuid, seed: Uuid, cogmap: Uuid) -> Uuid {
    let event = any_event(pool).await;
    sqlx::query_scalar(
        "INSERT INTO kb_edges \
             (source_table, source_id, target_table, target_id, edge_kind, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, $3, 'kb_cogmaps', $4, $5, $5) \
         RETURNING id",
    )
    .bind(nbr)
    .bind(seed)
    .bind(EdgeKind::Contains)
    .bind(cogmap)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("assert edge")
}

async fn cogmap_slice(
    app: &common::E2eTestApp,
    token: &str,
    cogmap: Uuid,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/cogmaps/{cogmap}/graph/slice")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("cogmaps/graph/slice request failed");
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A reader (member of a team joined to the cogmap) reaches an in-cogmap
/// neighbor via an incoming edge; a non-reader (no membership tie to the
/// cogmap) is denied as absence (404); an empty seed set is a 400.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reader_reaches_cogmap_neighborhood_outsider_denied(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let reader = provision_profile(&app, &app.token).await;
    let cogmap = create_readable_cogmap(&pool, reader, "cm-slice-e2e").await;
    let seed = home_resource_in_cogmap(&pool, cogmap, reader, "seed").await;
    let nbr = home_resource_in_cogmap(&pool, cogmap, reader, "nbr").await;
    assert_incoming_edge(&pool, nbr, seed, cogmap).await; // nbr -> seed, incoming to seed

    // Reader: 200 + neighbor reachable via the incoming edge.
    let body = serde_json::json!({ "seeds": [seed], "depth": 2, "edge_kinds": [] });
    let (status, resp_body) = cogmap_slice(&app, &app.token, cogmap, body.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reader gets a 200 subgraph: {resp_body:?}"
    );
    let sub: temper_core::types::graph_atlas::AtlasSubgraph =
        serde_json::from_value(resp_body).expect("response deserializes as AtlasSubgraph");
    assert!(
        sub.nodes.iter().any(|n| n.id == nbr),
        "incoming-edge neighbor must be reachable: {:?}",
        sub.nodes
    );

    // Outsider (provisioned profile, no membership tie to the cogmap's team): 404.
    let outsider_jwt =
        common::generate_test_jwt("cm-slice-outsider", "cm-slice-outsider@test.example.com");
    provision_profile(&app, &outsider_jwt).await;
    let (status, _) = cogmap_slice(&app, &outsider_jwt, cogmap, body).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-reader of the cogmap is denied as absence"
    );

    // Empty seeds: 400.
    let (status, _) = cogmap_slice(
        &app,
        &app.token,
        cogmap,
        serde_json::json!({ "seeds": [], "depth": 2, "edge_kinds": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty seeds is a 400");
}
