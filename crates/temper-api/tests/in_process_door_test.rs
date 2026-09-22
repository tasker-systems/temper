#![cfg(feature = "test-db")]
//! The in-process door, witnessed end-to-end.
//!
//! [Beat G1](temper task `01a0b9fe-7689-7692-b4ce-8323b8b19473`) of the one-seam goal: a
//! real `create_app` router handed to temper-client's `Router::oneshot` transport, one real
//! write driven through it with an `mcp_audience` bearer, and the ledger checked for what
//! must be true — the same validated token authorizes identically at either door
//! (`one-trust-domain`), and an MCP-originated act attributes `@mcp`, never silently `@api`
//! (`attribution-stays-true`). These two tests are the beat's acceptance witnesses; the full
//! tool-family migration is G3's, so the door is exercised at the endpoint a tool will
//! mirror (`POST /api/ingest`, the create path temper-cli already drives).

mod common;

use std::sync::Arc;

use sqlx::PgPool;

use temper_client::{auth::MemoryTokenStore, TemperClient};
use temper_core::types::authorship::ActInput;
use temper_core::types::ingest::IngestPayload;
use temper_workflow::operations::Surface;

const MCP_AUDIENCE: &str = "https://test.example/mcp";

/// The emitter entity name the `resource_created` event carries. The event's producing
/// anchor is the resource's home, not the resource, so the id is read from the payload —
/// the key `_project_resource_created` itself projects from.
async fn resource_created_emitter(pool: &PgPool, resource_id: uuid::Uuid) -> String {
    sqlx::query_scalar(
        "SELECT ent.name \
           FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
           JOIN kb_entities ent ON ent.id = ev.emitter_entity_id \
          WHERE et.name = 'resource_created' \
            AND ev.payload->>'resource_id' = $1::text",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("the create event carries its emitter")
}

/// Provision the caller through the door (the first authenticated call auto-provisions the
/// profile and its default context), seed the standing the write gates require, and return
/// the default context's id — the `context_ref` the ingest payload needs.
async fn provision_approve_and_default_context(
    pool: &PgPool,
    client: &TemperClient,
    email: &str,
) -> uuid::Uuid {
    client
        .profile()
        .get()
        .await
        .expect("the first call through the door auto-provisions the profile");
    common::fixtures::approve_standing_by_email(pool, email).await;
    sqlx::query_scalar(
        "SELECT c.id \
           FROM kb_contexts c \
           JOIN kb_profiles p ON p.id = c.owner_id \
          WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("the auto-provisioned default context")
}

/// The beat's headline witness: a caller authenticated with an `mcp_audience` token — a
/// token the MCP middleware already accepts — writes through the in-process door, and the
/// event lands attributed to the caller's `<handle>@mcp` emitter, never silently `@web`.
/// Fails while `require_auth` takes the API audience alone (the token is refused) and fails
/// while the surface extension is missing (the act lands on the untrusted header path, which
/// degrades `mcp` to `web`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_mcp_act_through_the_in_process_door_authenticates_and_attributes_to_mcp(pool: PgPool) {
    let verify_pool = pool.clone();
    let state = common::test_state_with_config(pool, |config| {
        config.auth.mcp_audience = MCP_AUDIENCE.to_string();
    })
    .await;
    let router = temper_api::create_app(state);

    let sub = format!("mcp-door-sub-{}", uuid::Uuid::new_v4());
    let email = format!("mcp-door-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt_with_audience(&sub, &email, MCP_AUDIENCE);

    let client = TemperClient::in_process_with_token(
        router,
        Surface::Mcp,
        token,
        Arc::new(MemoryTokenStore::empty()),
    )
    .expect("the in-process client builds");

    let context_id = provision_approve_and_default_context(&verify_pool, &client, &email).await;
    let payload = IngestPayload {
        title: "Written through the in-process door".to_owned(),
        origin_uri: format!("test://in-process-door/{}", uuid::Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "session".to_owned(),
        goal: None,
        content_hash: None,
        idempotency_key: None,
        content: "The body that crossed the one seam.".to_owned(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        sources: Vec::new(),
        act: ActInput::default(),
        segmented: None,
    };

    let view = client
        .ingest()
        .create(&payload)
        .await
        .expect("an mcp_audience token authorizes the write at the router's own door");

    let emitter = resource_created_emitter(&verify_pool, *view.id).await;
    assert!(
        emitter.ends_with("@mcp"),
        "an MCP-originated act must attribute to the mcp emitter, got {emitter:?}"
    );
}

/// A parameter that cannot vary cannot be said to flow: the same door, the same token
/// audience, a different surface — the emitter follows the surface that rode the door, and
/// the `cli` emitter is the proof the door is not hard-wired to `mcp`. Mirrors the guard the
/// blob emitter witnesses already carry.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_in_process_door_emitter_follows_the_surface_the_client_carries(pool: PgPool) {
    let verify_pool = pool.clone();
    let state = common::test_state_with_config(pool, |config| {
        config.auth.mcp_audience = MCP_AUDIENCE.to_string();
    })
    .await;
    let router = temper_api::create_app(state);

    let sub = format!("cli-door-sub-{}", uuid::Uuid::new_v4());
    let email = format!("cli-door-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt_with_audience(&sub, &email, MCP_AUDIENCE);

    let client = TemperClient::in_process_with_token(
        router,
        Surface::CliCloud,
        token,
        Arc::new(MemoryTokenStore::empty()),
    )
    .expect("the in-process client builds");

    let context_id = provision_approve_and_default_context(&verify_pool, &client, &email).await;
    let payload = IngestPayload {
        title: "A cli-surfaced write through the same door".to_owned(),
        origin_uri: format!("test://in-process-door/{}", uuid::Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "session".to_owned(),
        goal: None,
        content_hash: None,
        idempotency_key: None,
        content: "The body that crossed the one seam.".to_owned(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        sources: Vec::new(),
        act: ActInput::default(),
        segmented: None,
    };

    let view = client
        .ingest()
        .create(&payload)
        .await
        .expect("the write crosses the door");

    let emitter = resource_created_emitter(&verify_pool, *view.id).await;
    assert!(
        emitter.ends_with("@cli"),
        "the emitter must follow the surface that rode the door, got {emitter:?}"
    );
}
