#![cfg(feature = "test-db")]
//! Steward dispatch-tick surface end-to-end (goal 019f3220): drives the REAL Axum server
//! (in-process), real Postgres, real JWT auth, hitting `POST /api/steward/dispatch` with raw HTTP —
//! which is exactly how the Eve code dispatcher calls it in production (a bearer-token `fetch`, not
//! the Rust client). This is the vertical-wiring proof that pairs with the service-level unit tests
//! (`temper_services::services::{steward_service, workflow_job_service}::tests`): those prove the
//! sweep/queue logic; this proves route → handler → backend composite (reap→sweep→enqueue→claim) →
//! serialization actually connect, and that single-flight holds across two real dispatch calls.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::steward::DispatchTickResponse;

/// Pre-flight a token by hitting GET /api/profile (auto-provisions the profile), returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse")
}

/// Seed a single drifted, principal-readable, team-joined cogmap: a team the principal is a member
/// of (→ cogmap read), a team-owned context, and 6 `resource_created` events (above the default
/// threshold 5). Returns the cogmap id. Mirrors the service-test seed shape.
async fn seed_drifted_map(pool: &sqlx::PgPool, principal: Uuid) -> Uuid {
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('dispatchteam', 'D') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(principal)
    .execute(pool)
    .await
    .unwrap();

    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('dmap', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();

    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'building', 'Building') RETURNING id",
    )
    .bind(team)
    .fetch_one(pool)
    .await
    .unwrap();
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e') RETURNING id",
    )
    .bind(principal)
    .fetch_one(pool)
    .await
    .unwrap();
    for _ in 0..6 {
        sqlx::query(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'resource_created'), $1, 'kb_contexts', $2)",
        )
        .bind(entity)
        .bind(ctx)
        .execute(pool)
        .await
        .unwrap();
    }
    cogmap
}

async fn dispatch(app: &common::E2eTestApp) -> DispatchTickResponse {
    let resp = app
        .reqwest_client
        .post(app.url("/api/steward/dispatch"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("dispatch request failed");
    assert_eq!(resp.status(), StatusCode::OK, "dispatch should succeed");
    resp.json().await.expect("dispatch json parse")
}

/// The dispatch route is wired end-to-end: against a fresh DB with no drifted maps, the composite
/// runs server-side (reap→sweep→enqueue→claim) and round-trips a well-formed empty response.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dispatch_route_is_wired_and_returns_empty_when_no_drift(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    let body = dispatch(&app).await;
    assert!(
        body.claimed.is_empty(),
        "no drifted team-joined maps → nothing claimed"
    );
}

/// A drifted map is swept, enqueued, and claimed in one dispatch; a second immediate dispatch claims
/// nothing (single-flight — the first job is in_progress), proving the concurrency guard end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dispatch_claims_a_drifted_map_then_single_flight_blocks_the_next(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;
    let cogmap = seed_drifted_map(&pool, principal).await;

    let first = dispatch(&app).await;
    assert_eq!(first.claimed.len(), 1, "the one drifted map is claimed");
    assert_eq!(first.claimed[0].cogmap_id, cogmap);
    assert_eq!(first.claimed[0].attempts, 1, "first claim → attempts 1");

    let second = dispatch(&app).await;
    assert!(
        second.claimed.is_empty(),
        "single-flight: the in_progress job is not re-claimed"
    );
}
