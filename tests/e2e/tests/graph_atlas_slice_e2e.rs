//! HTTP e2e for POST /api/teams/{id}/graph/slice (R4) — the acceptance gate.
//! Proves the full stack (auth + handler + deny code) agrees, at the e2e access tier;
//! test-db predicate tests alone are a false signal for access changes.
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
    resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
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

/// Team-anchored read grant — the mechanism `resources_in_team_scope` reads.
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
    .unwrap();
}

/// A context owned by a team — real (FK-backed by `owner_id`) so it passes
/// `anchor_readable_by_profile`'s "context OWNED by a team the principal is a
/// member of" branch, which `graph_traverse_scoped`'s edge gate depends on.
async fn create_team_context(pool: &sqlx::PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("create team context")
}

/// Any pre-existing kb_events row (the L0 kernel cogmap genesis migration inserts
/// one) — sufficient FK target for asserted_by_event_id/last_event_id in these tests.
async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one kb_events row exists (L0 genesis)")
}

async fn assert_edge(
    pool: &sqlx::PgPool,
    source: Uuid,
    target: Uuid,
    edge_kind: EdgeKind,
    home_anchor_table: &str,
    home_anchor_id: Uuid,
    event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_edges \
             (source_table, source_id, target_table, target_id, edge_kind, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, $3, $4, $5, $6, $6) \
         RETURNING id",
    )
    .bind(source)
    .bind(target)
    .bind(edge_kind)
    .bind(home_anchor_table)
    .bind(home_anchor_id)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("assert edge")
}

async fn slice(
    app: &common::E2eTestApp,
    token: &str,
    team: Uuid,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/teams/{team}/graph/slice")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("graph/slice request failed");
    let status = resp.status();
    let body = resp
        .json::<serde_json::Value>()
        .await
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A member gets a 200 `AtlasSubgraph` for a seed resource in the team's scope;
/// the doc-type-less seed node serializes without a `doc_type` key at all (the
/// wire type's `skip_serializing_if` contract, not just a `null` value).
/// Empty seeds is a 400; a non-member of the team is a 404 (deny-as-absence).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn slice_returns_subgraph_validates_seeds_and_denies_outsiders(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let outsider_token =
        common::generate_test_jwt("gas-e2e-outsider", "gas-e2e-outsider@test.example.com");
    let _outsider = provision_profile(&app, &outsider_token).await;

    let team = create_team(&pool, "gas-e2e-team").await;
    add_member(&pool, team, member).await;

    let seed = create_resource(&pool, "seed resource", "temper://gas-e2e/seed").await;
    grant_read_to_team(&pool, seed, team, member).await;
    // Deliberately no kb_properties doc_type row on `seed`.

    // Happy path: member, valid non-empty seeds.
    let (status, body) = slice(
        &app,
        &app.token,
        team,
        serde_json::json!({ "seeds": [seed], "depth": 2, "edge_kinds": [] }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "member gets a 200 subgraph: {body:?}"
    );
    let nodes = body["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 1, "seed with no walked edges yields one node");
    let seed_node = &nodes[0];
    assert_eq!(seed_node["id"], seed.to_string());
    assert!(
        !seed_node
            .as_object()
            .expect("node is an object")
            .contains_key("doc_type"),
        "doc-type-less node omits the `doc_type` key entirely (not `null`): {seed_node:?}"
    );
    assert_eq!(body["edges"].as_array().expect("edges array").len(), 0);

    // Empty seeds — 400.
    let (status, _) = slice(
        &app,
        &app.token,
        team,
        serde_json::json!({ "seeds": [], "depth": 2, "edge_kinds": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "empty seeds is a 400");

    // Outsider — 404 (deny-as-absence; not a member of the team).
    let (status, _) = slice(
        &app,
        &outsider_token,
        team,
        serde_json::json!({ "seeds": [seed], "depth": 2, "edge_kinds": [] }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-member of the team is denied as absence"
    );
}

/// C3: a walked edge carries its `kb_edges.id` as a non-null UUID on the wire —
/// the prerequisite for R5 edge trails (`readTrail('edge', id)`), which need a
/// stable id to address a rendered `AtlasEdge`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn slice_edge_carries_its_kb_edges_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;

    let team = create_team(&pool, "gas-e2e-edge-id-team").await;
    add_member(&pool, team, member).await;
    let ctx = create_team_context(&pool, team, "gas-e2e-edge-id-ctx").await;
    let event = any_event(&pool).await;

    let source = create_resource(&pool, "edge source", "temper://gas-e2e/edge-id-src").await;
    let target = create_resource(&pool, "edge target", "temper://gas-e2e/edge-id-tgt").await;
    grant_read_to_team(&pool, source, team, member).await;
    grant_read_to_team(&pool, target, team, member).await;

    let edge_id = assert_edge(
        &pool,
        source,
        target,
        EdgeKind::Contains,
        "kb_contexts",
        ctx,
        event,
    )
    .await;

    let (status, body) = slice(
        &app,
        &app.token,
        team,
        serde_json::json!({ "seeds": [source], "depth": 2, "edge_kinds": [] }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "member gets a 200 subgraph: {body:?}"
    );

    let edges = body["edges"].as_array().expect("edges array");
    assert_eq!(edges.len(), 1, "one walked edge: {edges:?}");
    let wire_id = edges[0]["id"].as_str().expect("edge id is a string");
    assert_eq!(
        wire_id.parse::<Uuid>().expect("edge id parses as a UUID"),
        edge_id,
        "wire edge id matches the kb_edges row it was induced from"
    );
}
