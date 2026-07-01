#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for the facet write endpoint (`POST /api/facets`).
//!
//! These tests exercise the full stack: JWT auth → system access gate →
//! handler → DbBackend → Postgres. They use the `TestApp` harness (a live
//! Axum server on a random port backed by a per-test isolated DB), which is
//! the same pattern used by `relationship_handler_test.rs`.
//!
//! The system is seeded with `access_mode = 'open'` so all authenticated
//! profiles pass the system-access gate without explicit team membership.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

// ─── Fixture helpers ─────────────────────────────────────────────────────────

/// Create a resource in the test profile's context, returning its id for
/// id-based facet addressing.
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
    // set_facet resolves its home anchor and can_modify passes.
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

// ─── Test 1: POST /api/facets → 200, returns FacetAck ────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn set_facet_returns_ack(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email = format!("fh-set-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resource = insert_resource(&pool, profile_id, context_id, "Doc A", "fh-set-a").await;

    let body = json!({
        "resource": resource.to_string(),
        "values": {"summary": "example facet"},
        "weight": 1.0
    });

    let resp = app
        .client
        .post(app.url("/api/facets"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let ack: Value = resp.json().await.expect("expected JSON ack");
    assert_eq!(status, 200, "set_facet should return 200; body: {ack}");

    assert!(
        ack["property_id"].is_string(),
        "FacetAck must contain property_id string; got {ack}"
    );

    // Verify the property_id parses as a valid UUID.
    let pid_str = ack["property_id"].as_str().expect("property_id is string");
    Uuid::parse_str(pid_str).expect("property_id should be a valid UUID");

    // Verify the property row was written into kb_properties (owner_table/owner_id
    // polymorphic ownership — kb_properties has no dedicated resource_id column).
    let property_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_resources' AND owner_id = $1",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .expect("property count");
    assert!(
        property_count >= 1,
        "at least one property should be written; got {property_count}"
    );
}

// ─── Test 1b: setting the same facet key twice → 409 Conflict (not 500) ──────
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn set_facet_duplicate_key_returns_409(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email = format!("fh-dup-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let resource = insert_resource(&pool, profile_id, context_id, "Doc Dup", "fh-dup-a").await;

    let body = json!({
        "resource": resource.to_string(),
        "values": {"summary": "first"},
    });

    // First set succeeds.
    let first = app
        .client
        .post(app.url("/api/facets"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        first.status().as_u16(),
        200,
        "first facet set should succeed"
    );

    // Second set of the same active facet key hits uq_kb_properties_active → 409, not 500.
    let second = app
        .client
        .post(app.url("/api/facets"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        second.status().as_u16(),
        409,
        "re-setting an active facet key must be a 409 Conflict, not a 500"
    );
}

// ─── Test 2: POST /api/facets without auth → 401 ─────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn set_facet_without_auth_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let body = json!({
        "resource": Uuid::new_v4().to_string(),
        "values": {"summary": "example facet"},
        "weight": 1.0
    });

    let resp = app
        .client
        .post(app.url("/api/facets"))
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

// ─── Test 3: POST /api/facets — profile cannot modify another's resource ─────

/// Profile Q attempts to set a facet on a resource owned by profile P. The
/// resource is referenced by UUID so resolution succeeds, but
/// `check_can_modify` inside DbBackend must reject the write with Forbidden.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn set_facet_on_other_profile_resource_returns_403(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // P creates a resource.
    let email_p = format!("fh-authp-{}@example.com", Uuid::new_v4());
    let (profile_p, context_p) =
        common::fixtures::create_test_profile_with_context(&pool, &email_p).await;
    let resource_p = insert_resource(&pool, profile_p, context_p, "P's Doc", "fh-authp-doc").await;

    // Q gets a token.
    let email_q = format!("fh-authq-{}@example.com", Uuid::new_v4());
    let (profile_q, _) = common::fixtures::create_test_profile_with_context(&pool, &email_q).await;
    let sub_q = format!("test|{profile_q}");
    let token_q = common::generate_test_jwt(&sub_q, &email_q);

    let body = json!({
        "resource": resource_p.to_string(),
        "values": {"summary": "example facet"},
        "weight": 1.0
    });

    let resp = app
        .client
        .post(app.url("/api/facets"))
        .header("Authorization", format!("Bearer {token_q}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "Q setting a facet on P's resource should return 403; body: {}",
        resp.text().await.unwrap_or_default()
    );
}
