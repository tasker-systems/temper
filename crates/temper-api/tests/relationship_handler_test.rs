#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for the relationship write endpoints.
//!
//! These tests exercise the full stack: JWT auth → system access gate →
//! handler → DbBackend → Postgres. They use the `TestApp` harness (a live
//! Axum server on a random port backed by a per-test isolated DB), which is
//! the same pattern used by `resources_test.rs` and `auth_test.rs`.
//!
//! The system is seeded with `access_mode = 'open'` so all authenticated
//! profiles pass the system-access gate without explicit team membership.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

// ─── Fixture helpers ─────────────────────────────────────────────────────────

/// Create a resource in the test profile's context, returning its id for
/// id-based edge addressing.
async fn insert_resource(
    pool: &PgPool,
    owner_id: Uuid,
    context_id: Uuid,
    title: &str,
    slug: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    // Substrate: kb_resources holds (id, title, origin_uri); ownership + home
    // live in kb_resource_homes. Home the resource in the owner's context so
    // assert_relationship resolves its home anchor and can_modify passes.
    sqlx::query(r#"INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)"#)
        .bind(id)
        .bind(title)
        .bind(format!("test://{slug}"))
        .execute(pool)
        .await
        .expect("insert_resource");
    sqlx::query(
        r#"INSERT INTO kb_resource_homes
            (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
           VALUES ($1, 'kb_contexts', $2, $3, $3)"#,
    )
    .bind(id)
    .bind(context_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("home resource");
    id
}

// ─── Test 1: POST /api/relationships → 200, returns RelationshipAck ──────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_returns_ack(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email = format!("rh-assert-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let source_a = insert_resource(&pool, profile_id, context_id, "Doc A", "rh-assert-a").await;
    let target_b = insert_resource(&pool, profile_id, context_id, "Doc B", "rh-assert-b").await;

    let body = json!({
        "source": source_a.to_string(),
        "target": target_b.to_string(),
        "edge_kind": "leads_to",
        "polarity": "forward",
        "label": "depends_on",
        "weight": 1.0
    });

    let resp = app
        .client
        .post(app.url("/api/relationships"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "assert should return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let ack: Value = app
        .client
        .post(app.url("/api/relationships"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("second request failed")
        .json()
        .await
        .expect("expected JSON ack");

    assert!(
        ack["edge_handle"].is_string(),
        "RelationshipAck must contain edge_handle string; got {ack}"
    );

    // Verify the edge_handle parses as a valid UUID.
    let cid_str = ack["edge_handle"].as_str().expect("edge_handle is string");
    Uuid::parse_str(cid_str).expect("edge_handle should be a valid UUID");

    // Verify edge row was projected into the substrate kb_edges.
    let edge_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges WHERE source_table = 'kb_resources' AND source_id = $1",
    )
    .bind(source_a)
    .fetch_one(&pool)
    .await
    .expect("edge count");
    assert!(
        edge_count >= 1,
        "at least one edge should be projected; got {edge_count}"
    );
}

// ─── Test 2: POST /api/relationships without auth → 401 ───────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_without_auth_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let body = json!({
        "source": Uuid::new_v4().to_string(),
        "target": Uuid::new_v4().to_string(),
        "edge_kind": "near",
        "polarity": "forward",
        "label": "relates_to",
        "weight": 1.0
    });

    let resp = app
        .client
        .post(app.url("/api/relationships"))
        .json(&body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing auth should return 401"
    );
}

// ─── Test 3: POST /api/relationships — profile cannot modify another's resource

/// Profile Q attempts to assert a relationship from a resource owned by
/// profile P. The source is referenced by UUID so resolution succeeds, but
/// `check_can_modify` inside DbBackend must reject the write with Forbidden.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_on_other_profile_resource_returns_403(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // P creates a resource.
    let email_p = format!("rh-authp-{}@example.com", Uuid::new_v4());
    let (profile_p, context_p) =
        common::fixtures::create_test_profile_with_context(&pool, &email_p).await;
    let resource_p = insert_resource(&pool, profile_p, context_p, "P's Doc", "rh-authp-doc").await;

    // Q gets a token.
    let email_q = format!("rh-authq-{}@example.com", Uuid::new_v4());
    let (profile_q, _) = common::fixtures::create_test_profile_with_context(&pool, &email_q).await;
    let sub_q = format!("test|{profile_q}");
    let token_q = common::generate_test_jwt(&sub_q, &email_q);

    // Q uses the source id to bypass resolve but hits check_can_modify.
    let body = json!({
        "source": resource_p.to_string(),
        "target": Uuid::new_v4().to_string(),
        "edge_kind": "near",
        "polarity": "forward",
        "label": "relates_to",
        "weight": 1.0
    });

    let resp = app
        .client
        .post(app.url("/api/relationships"))
        .header("Authorization", format!("Bearer {token_q}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "Q asserting on P's resource should return 403; body: {}",
        resp.text().await.unwrap_or_default()
    );
}

// ─── Test 4: POST /api/relationships/{cid}/fold → 200, edge marked folded ────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn fold_relationship_marks_edge_folded(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email = format!("rh-fold-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let source_a = insert_resource(&pool, profile_id, context_id, "Doc A", "rh-fold-a").await;
    let target_b = insert_resource(&pool, profile_id, context_id, "Doc B", "rh-fold-b").await;

    // First, assert the relationship.
    let assert_body = json!({
        "source": source_a.to_string(),
        "target": target_b.to_string(),
        "edge_kind": "leads_to",
        "polarity": "forward",
        "label": "depends_on",
        "weight": 1.0
    });

    let assert_resp: Value = app
        .client
        .post(app.url("/api/relationships"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&assert_body)
        .send()
        .await
        .expect("assert request failed")
        .json()
        .await
        .expect("assert JSON");

    let edge_handle = assert_resp["edge_handle"]
        .as_str()
        .expect("edge_handle in assert response");

    // Now fold it.
    let fold_resp = app
        .client
        .post(app.url(&format!("/api/relationships/{edge_handle}/fold")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "reason": "test fold via HTTP" }))
        .send()
        .await
        .expect("fold request failed");

    assert_eq!(
        fold_resp.status().as_u16(),
        200,
        "fold should return 200; body: {}",
        fold_resp.text().await.unwrap_or_default()
    );

    let fold_ack: Value = app
        .client
        .post(app.url(&format!("/api/relationships/{edge_handle}/fold")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "reason": "test fold via HTTP second pass" }))
        .send()
        .await
        .expect("second fold request failed")
        .json()
        .await
        .expect("fold ack JSON");

    assert!(
        fold_ack["edge_handle"].is_string(),
        "fold ack must contain edge_handle; got {fold_ack}"
    );

    // Verify edge is marked folded in the DB. The ack's edge_handle IS the
    // substrate edge id (DbBackend returns the kb_edges row id).
    let cid_uuid = Uuid::parse_str(edge_handle).expect("valid uuid");
    let is_folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_edges WHERE id = $1")
        .bind(cid_uuid)
        .fetch_one(&pool)
        .await
        .expect("is_folded query");

    assert!(is_folded, "edge should be folded after fold endpoint call");
}
