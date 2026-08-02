//! Owner-scoped create idempotency key at the HTTP surface (issue #581, spike rung 3).
//!
//! Two `POST /api/ingest` requests carrying the SAME `idempotency_key` (by the same owner) converge
//! on the already-committed resource: the second returns the first's id and mints no twin — the
//! lost-acknowledgment recovery. A create with NO key keeps today's behaviour (a fresh id each time).
//!
//! This is the surface-level companion to `temper-services`' `keyed_create_backend_test.rs`, which
//! covers the backend/substrate claim directly; here we prove the field threads all the way through
//! the ingest handler onto `CreateResource`.
#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::IngestPayload;

/// Build a profile + JWT, return `(token, profile_id, context_id)`.
async fn auth(pool: &PgPool) -> (String, Uuid, Uuid) {
    let email = format!("idempotency-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, profile_id, context_id)
}

/// A minimal one-shot ingest payload homed in `context_id`, carrying an optional idempotency key.
///
/// Body is deliberately **empty**: idempotency is a create-identity concern, independent of body,
/// and an empty body means no chunks and no server-side embed — so this stays in the no-ONNX
/// `test-db` tier (a content-bearing, chunk-less payload would force a `bge-768` embed that only the
/// `test-embed` tier can serve).
fn ingest_payload(context_id: Uuid, title: &str, key: Option<Uuid>) -> IngestPayload {
    IngestPayload {
        idempotency_key: key,
        segmented: None,
        title: title.to_string(),
        origin_uri: format!("test://idempotency-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    }
}

/// Count live resources homed in a context — isolates this test's own writes.
async fn homed_count(pool: &PgPool, context: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = $1 AND r.is_active",
    )
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("count homed resources")
}

async fn post_ingest(app: &common::TestApp, token: &str, payload: &IngestPayload) -> Value {
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(payload)
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "ingest must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );
    resp.json().await.expect("ingest JSON")
}

/// Two ingests with the SAME key converge: same resource id, exactly one homed row, and one
/// `(owner, key)` mapping recorded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn same_idempotency_key_returns_the_same_resource(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, profile_id, context_id) = auth(&pool).await;
    let key = Uuid::now_v7();

    let first = post_ingest(
        &app,
        &token,
        &ingest_payload(context_id, "First attempt", Some(key)),
    )
    .await;
    let first_id = Uuid::parse_str(first["id"].as_str().expect("id missing")).unwrap();

    // A retry of the "same" create — even with a different title — carrying the same key must
    // converge on the already-committed resource, not mint a twin.
    let second = post_ingest(
        &app,
        &token,
        &ingest_payload(context_id, "Second attempt", Some(key)),
    )
    .await;
    let second_id = Uuid::parse_str(second["id"].as_str().expect("id missing")).unwrap();

    assert_eq!(
        second_id, first_id,
        "a replay of the same idempotency_key returns the original resource id"
    );
    assert_eq!(
        homed_count(&pool, context_id).await,
        1,
        "a replay mints no duplicate — exactly one resource is homed"
    );

    // The resource id is server-minted, never the client's key.
    assert_ne!(
        first_id, key,
        "the resource id is server-generated, never the supplied idempotency key"
    );

    // Exactly one (owner, key) mapping is recorded.
    let mappings: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_idempotency_keys \
         WHERE owner_profile_id = $1 AND idempotency_key = $2",
    )
    .bind(profile_id)
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("count key mappings");
    assert_eq!(
        mappings, 1,
        "one (owner, key) mapping, shared by both requests"
    );
}

/// A create with NO idempotency key keeps today's behaviour: a fresh id each time, distinct
/// resources, and no mapping row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unkeyed_create_mints_fresh_each_time(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, _profile_id, context_id) = auth(&pool).await;

    let a = post_ingest(&app, &token, &ingest_payload(context_id, "Doc A", None)).await;
    let b = post_ingest(&app, &token, &ingest_payload(context_id, "Doc B", None)).await;

    let a_id = Uuid::parse_str(a["id"].as_str().expect("id missing")).unwrap();
    let b_id = Uuid::parse_str(b["id"].as_str().expect("id missing")).unwrap();

    assert_ne!(a_id, b_id, "unkeyed creates are distinct resources");
    assert_eq!(
        homed_count(&pool, context_id).await,
        2,
        "two unkeyed creates home two resources"
    );
}
