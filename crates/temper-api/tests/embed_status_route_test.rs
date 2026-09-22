#![cfg(feature = "test-db")]
//! The embedding-status batch read, witnessed through the in-process door.
//!
//! [Beat G3a](temper task `01a0cb1b-42e6-7922-a9dc-b30c402d8a3f`): the MCP resources
//! tools derive each response's `embedding_status` from this read, so the read itself
//! must cross the router — the same door the tools cross. The route is deliberately
//! unregistered (`GET /api/embed/status`, plain `.route()`); this suite is its witness.

mod common;

use std::sync::Arc;

use sqlx::PgPool;

use temper_client::{auth::MemoryTokenStore, TemperClient};
use temper_core::types::ingest::IngestPayload;
use temper_workflow::operations::Surface;

const MCP_AUDIENCE: &str = "https://test.example/mcp";

async fn door_client(pool: PgPool, sub: &str, email: &str) -> TemperClient {
    let state = common::test_state_with_config(pool, |config| {
        config.auth.mcp_audience = MCP_AUDIENCE.to_string();
    })
    .await;
    let router = temper_api::create_app(state);
    let token = common::generate_test_jwt_with_audience(sub, email, MCP_AUDIENCE);
    TemperClient::in_process_with_token(
        router,
        Surface::Mcp,
        token,
        Arc::new(MemoryTokenStore::empty()),
    )
    .expect("the in-process client builds")
}

/// The read the MCP enrichment depends on: an authenticated caller asks for the ids a
/// gated read returned and gets the pipeline status map back — `ready` for a resource
/// with no chunks (an empty body is trivially ready, per the status enum's own doc).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_status_batch_answers_through_the_door(pool: PgPool) {
    let verify_pool = pool.clone();
    let sub = format!("embed-status-{}", uuid::Uuid::new_v4());
    let email = format!("embed-status-{}@example.com", uuid::Uuid::new_v4());
    let client = door_client(pool, &sub, &email).await;

    client
        .profile()
        .get()
        .await
        .expect("the first call auto-provisions the profile");
    common::fixtures::approve_standing_by_email(&verify_pool, &email).await;
    let context_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c \
         JOIN kb_profiles p ON p.id = c.owner_id \
         WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind(&email)
    .fetch_one(&verify_pool)
    .await
    .expect("the default context");

    let view = client
        .ingest()
        .create(&IngestPayload {
            title: "A resource whose status is asked for".to_owned(),
            origin_uri: format!("test://embed-status/{}", uuid::Uuid::new_v4()),
            context_ref: context_id.to_string(),
            home_cogmap_id: None,
            doc_type_name: "session".to_owned(),
            goal: None,
            content_hash: None,
            idempotency_key: None,
            content: String::new(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: None,
            sources: Vec::new(),
            act: temper_core::types::authorship::ActInput::default(),
            segmented: None,
        })
        .await
        .expect("the create lands");

    let statuses = client
        .embed()
        .status(&[*view.id])
        .await
        .expect("the status batch answers through the door");
    assert_eq!(
        statuses.get(&(*view.id)),
        Some(&temper_core::types::workflow_job::EmbeddingStatus::Ready),
        "no chunks ⇒ ready; got {statuses:?}"
    );
}

/// The route's own contract, pinned: it does NOT re-gate per resource — an id the
/// caller was never shown (or that never existed) answers like any other, `ready` for
/// no-chunks. Gating lives in the reads that produced the ids, exactly as the direct
/// call the route replaces behaved.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_batch_answers_for_ids_it_was_not_shown(pool: PgPool) {
    let verify_pool = pool.clone();
    let sub = format!("embed-ghost-{}", uuid::Uuid::new_v4());
    let email = format!("embed-ghost-{}@example.com", uuid::Uuid::new_v4());
    let client = door_client(pool, &sub, &email).await;
    client.profile().get().await.expect("profile provisions");
    // The standing gate denies EVERY gated request for an unapproved profile — even this
    // ungated-read — so approve before asking.
    common::fixtures::approve_standing_by_email(&verify_pool, &email).await;

    let ghost = uuid::Uuid::now_v7();
    let statuses = client
        .embed()
        .status(&[ghost])
        .await
        .expect("the batch answers for an arbitrary id");
    assert_eq!(
        statuses.get(&ghost),
        Some(&temper_core::types::workflow_job::EmbeddingStatus::Ready),
        "the route reports pipeline status, never resource existence: {statuses:?}"
    );
}
