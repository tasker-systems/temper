#![cfg(feature = "test-db")]
//! The search + query family's attribution witness — the per-family bite the
//! network door demands (learning 8 of the beat).
//!
//! A search or query is a READ: `search_select` and `run_composition` write no
//! ledger events, so the family's own act has no ledger row to read. What the
//! witness pins is the chain the family's attribution rides:
//!
//! 1. The family's acts cross the TRUSTED path — driven through the real
//!    listener, the relay's carrier is honored (`relayed_surface_trusted`), not
//!    degraded. This is the fact the family consumes.
//! 2. That same trusted path is what lands `@mcp` at the ledger: a relayed write
//!    (the relay's exact wire shape at the listener) reads back
//!    `<handle>@mcp` from `kb_events`.
//!
//! The bite: drop the `RelayedSurface` planting in `relay_trust` and BOTH halves
//! redden — the trusted event vanishes and the ledger read lands `@web`.

mod common;

use serde_json::json;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use common::tracing_layer::TestTracingLayer;

/// The emitter entity name on the most recent event for `handle` — the ledger
/// read (`kb_events.id` is UUIDv7, so newest-first needs no clock).
async fn latest_emitter_for(pool: &PgPool, handle: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT e.name FROM kb_events ev \
         JOIN kb_entities e ON e.id = ev.emitter_entity_id \
         JOIN kb_profiles p ON p.id = e.profile_id \
         WHERE p.handle = $1 \
         ORDER BY ev.id DESC LIMIT 1",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("an event exists for this profile")
}

async fn handle_of_only_profile(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>("SELECT handle FROM kb_profiles ORDER BY id DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a profile exists")
}

/// The relay's exact wire shape at the real listener: the bearer, the service
/// credential, and the one allowed carrier. A write landing this way is the
/// write a relayed tool act IS, as far as the door can tell — which is the whole
/// point of the credential's confidentiality.
async fn relayed_ingest(app: &common::E2eTestApp, title: &str) -> u16 {
    let ctx: uuid::Uuid = sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c \
         JOIN kb_profiles p ON p.id = c.owner_id \
         WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind("e2e@test.example.com")
    .fetch_one(&app.pool)
    .await
    .expect("the default context");
    let payload = json!({
        "title": title,
        "origin_uri": "mcp://witness/relayed-ingest",
        "context_ref": ctx.to_string(),
        "doc_type_name": "session",
        "content": "",
        "act": {},
        "sources": []
    });
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .bearer_auth(&app.token)
        .header(
            temper_workflow::operations::SERVICE_CREDENTIAL_HEADER,
            common::TEST_MCP_SERVICE_SECRET,
        )
        .header(temper_workflow::operations::RELAYED_SURFACE_HEADER, "mcp")
        .json(&payload)
        .send()
        .await
        .expect("the relayed write answers");
    resp.status().as_u16()
}

/// The relayed door's writes land `@mcp` at the ledger — read back through the
/// real listener's own DB. This is the attribution the family's acts share the
/// trust path with.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_relaysd_write_through_the_real_listener_lands_at_mcp_in_the_ledger(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("provision profile on first authenticated request");
    let handle = handle_of_only_profile(&app.pool).await;

    let status = relayed_ingest(&app, "the relayed door's ledger witness").await;
    assert_eq!(status, 200, "the relayed write lands: {status}");

    assert_eq!(
        latest_emitter_for(&app.pool, &handle).await,
        format!("{handle}@mcp"),
        "the trusted carrier attributed the act to the MCP emitter"
    );
}

/// The search + query family's own acts cross the TRUSTED path: driven through
/// the real listener, each act's hop carries the carrier beside the credential,
/// and `relay_trust` honors it. With the planting dropped these acts degrade to
/// `@web` and this witness reddens with the ledger one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_familys_acts_cross_the_trusted_path(pool: PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();

    let search = temper_mcp::tools::search::search(
        &svc,
        &parts,
        serde_json::from_value(json!({ "query": "anything" })).expect("input deserializes"),
    )
    .await;
    assert!(
        search.is_ok(),
        "the search act crosses the door: {search:?}"
    );

    let plan = json!({
        "stages": [],
        "outcome": { "returns": [] }
    });
    let query = temper_mcp::tools::query::run_query(
        &svc,
        &parts,
        serde_json::from_value(json!({ "plan": plan })).expect("input deserializes"),
    )
    .await;
    assert!(
        query.is_err(),
        "the empty plan is refused — but only after crossing: {query:?}"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = captured.lock().unwrap();
    let trusted_count = events
        .iter()
        .filter(|e| {
            e.fields
                .get("counter")
                .map(|c| c.contains("relayed_surface_trusted"))
                .unwrap_or(false)
        })
        .count();
    assert!(
        trusted_count >= 2,
        "both acts' carriers were honored, not degraded: {trusted_count} trusted events in {events:?}"
    );
}
