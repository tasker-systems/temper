//! HTTP e2e for GET /api/graph/elements/{kind}/{id}/trail (R5) — the acceptance
//! gate. Proves the full stack (auth + handler + kind-parsing) agrees; the
//! SQL-level keying/visibility semantics are covered by
//! `element_trail_sql_test.rs`.
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    resp.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn mk_entity(pool: &sqlx::PgPool, profile: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id")
        .bind(profile)
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("insert entity")
}

async fn mk_owned_context(pool: &sqlx::PgPool, profile: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(profile)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("insert context")
}

async fn insert_event(pool: &sqlx::PgPool, type_name: &str, emitter: Uuid, payload: Value) -> Uuid {
    insert_event_with_metadata(pool, type_name, emitter, payload, serde_json::json!({})).await
}

/// Same as [`insert_event`], but with caller-supplied `kb_events.metadata` — used
/// to exercise the `metadata->>'confidence'` extraction in `event_service::element_trail`.
async fn insert_event_with_metadata(
    pool: &sqlx::PgPool,
    type_name: &str,
    emitter: Uuid,
    payload: Value,
    metadata: Value,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, metadata) \
         VALUES ((SELECT id FROM kb_event_types WHERE name = $1), $2, $3, $4) \
         RETURNING id",
    )
    .bind(type_name)
    .bind(emitter)
    .bind(payload)
    .bind(metadata)
    .fetch_one(pool)
    .await
    .expect("insert event")
}

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

/// Home a resource to a `kb_contexts` anchor owned/originated by `profile` — the
/// branch `resources_visible_to` reads for direct ownership.
async fn home_resource(pool: &sqlx::PgPool, resource: Uuid, context: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(context)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home resource");
}

async fn insert_edge(
    pool: &sqlx::PgPool,
    id: Uuid,
    source: Uuid,
    target: Uuid,
    home_anchor_id: Uuid,
    asserted_by_event_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_edges \
             (id, source_table, source_id, target_table, target_id, edge_kind, polarity, weight, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ($1, 'kb_resources', $2, 'kb_resources', $3, 'contains', 'forward', 1.0, \
                 'kb_contexts', $4, $5, $5)",
    )
    .bind(id)
    .bind(source)
    .bind(target)
    .bind(home_anchor_id)
    .bind(asserted_by_event_id)
    .execute(pool)
    .await
    .expect("insert edge");
}

async fn trail(app: &common::E2eTestApp, token: &str, kind: &str, id: Uuid) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/graph/elements/{kind}/{id}/trail")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("element trail request failed");
    let status = resp.status();
    let body = resp.json::<Value>().await.unwrap_or(Value::Null);
    (status, body)
}

/// A member GETs the trail for an edge they can read (home-anchored to a
/// context they own) and gets a 200 with the events time-ordered; `kind=bogus`
/// is a 400.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edge_trail_returns_ordered_events_and_rejects_bad_kind(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let member = provision_profile(&app, &app.token).await;
    let entity = mk_entity(&pool, member, "ete-entity").await;
    let context = mk_owned_context(&pool, member, "ete-context").await;

    // Real, visible endpoints homed in the member-owned context — the edge trail
    // enforces endpoint_readable_by_profile(source/target) as well as the home.
    let src = create_resource(&pool, "ete-src", "temper://ete/src").await;
    let tgt = create_resource(&pool, "ete-tgt", "temper://ete/tgt").await;
    home_resource(&pool, src, context, member).await;
    home_resource(&pool, tgt, context, member).await;

    let edge_id = Uuid::now_v7();
    let assert_event = insert_event(
        &pool,
        "relationship_asserted",
        entity,
        serde_json::json!({"edge_id": edge_id, "weight": 1.0}),
    )
    .await;
    insert_edge(&pool, edge_id, src, tgt, context, assert_event).await;

    // Carries a confidence band in metadata — exercises the `metadata->>'confidence'`
    // extraction into `ElementEvent.confidence` (M5: otherwise never exercised).
    let reweight_event = insert_event_with_metadata(
        &pool,
        "relationship_reweighted",
        entity,
        serde_json::json!({"edge_id": edge_id, "weight": 2.0}),
        serde_json::json!({"confidence": "probable"}),
    )
    .await;

    // Happy path: 200 with both events, time-ordered.
    let (status, body) = trail(&app, &app.token, "edge", edge_id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "member reads the edge trail: {body:?}"
    );
    assert_eq!(body["element_kind"], "edge");
    assert_eq!(body["element_id"], edge_id.to_string());
    let events = body["events"].as_array().expect("events array");
    assert_eq!(events.len(), 2, "both events surface: {events:?}");
    assert_eq!(events[0]["event_id"], assert_event.to_string());
    assert_eq!(events[0]["kind"], "relationship_asserted");
    assert_eq!(events[1]["event_id"], reweight_event.to_string());
    assert_eq!(events[1]["kind"], "relationship_reweighted");
    assert_eq!(
        events[1]["confidence"], "probable",
        "metadata->>'confidence' surfaces on the ElementEvent: {events:?}"
    );
    assert!(
        events[0]["confidence"].is_null(),
        "the assert event has no confidence in its metadata: {events:?}"
    );

    // Unknown kind — 400.
    let (status, _) = trail(&app, &app.token, "bogus", edge_id).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unknown element kind is rejected"
    );
}
