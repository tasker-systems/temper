#![cfg(feature = "test-db")]
//! The steward family's attribution witness — the per-family bite the network door
//! demands (beat 5).
//!
//! Neither steward act writes a ledger row of its own. The delta is a read; the advance
//! moves two cursor columns on `kb_cogmaps` and completes the active workflow job, and
//! emits no `kb_events` row (its command carries an `origin`, which the backend does not
//! read). So, as for the search + query family (G3b), there is no family row to read
//! `<handle>@mcp` back from. What the witness pins is:
//!
//! 1. **The family's acts cross the TRUSTED path.** Driven through the real listener,
//!    both acts' hops carry the carrier beside the service credential, and
//!    `relay_trust` honors it (`relay_trust = trusted`), not degrades it. This is the
//!    fact the family's attribution rides; the ledger half of that trusted path (a
//!    relayed write reads back `<handle>@mcp`) is family-independent and is pinned once,
//!    in `search_query_attribution_witness_test.rs`.
//! 2. **An advance writes no ledger row** — neither its cursor UPDATE nor its completion of
//!    the active steward job (the test opens one, so that branch runs). There is nothing for
//!    attribution to attach to today. If a future change makes the advance emit an event, this pin goes red, and
//!    the change must then answer the attribution question with a witness of its own.
//!
//! The bite, probe-proven at authoring: refuse the carrier in `relay_trust`'s honor arm
//! and witness 1 reddens.

mod common;

use serde_json::json;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use common::tracing_layer::{relay_trust_values, TestTracingLayer};
use temper_core::types::workflow_job::{DispatchType, Persona};

/// A cogmap the harness principal can read and author, joined to a team that owns a
/// context with one event in the cogmap's ingest window. Answers `(cogmap, event)`.
async fn authorable_map_with_one_event(pool: &PgPool) -> (Uuid, Uuid) {
    let principal: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(pool)
        .await
        .expect("the harness principal is provisioned");
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('steward-witness', 'Steward witness') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("team");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(principal)
    .execute(pool)
    .await
    .expect("membership");
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('witness telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("telos");
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('steward witness map', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("cogmap");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("team joins the map");
    common::grant_cogmap_write(pool, cogmap, principal).await;
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'steward-witness', 'Steward witness') RETURNING id",
    )
    .bind(team)
    .fetch_one(pool)
    .await
    .expect("team context");
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'steward-witness-emitter') RETURNING id",
    )
    .bind(principal)
    .fetch_one(pool)
    .await
    .expect("emitter entity");
    let event: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         VALUES ((SELECT id FROM kb_event_types WHERE name = 'resource_created'), $1, 'kb_contexts', $2) RETURNING id",
    )
    .bind(entity)
    .bind(ctx)
    .fetch_one(pool)
    .await
    .expect("event");
    (cogmap, event)
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("event count")
}

/// Both steward acts cross the TRUSTED path: driven through the real listener, each
/// act's hop carries the carrier beside the credential, and `relay_trust` honors it.
/// With the carrier refused, these acts degrade and this witness reddens.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_familys_acts_cross_the_trusted_path(pool: PgPool) {
    let (layer, _events, spans) = TestTracingLayer::with_spans();
    let _guard = tracing_subscriber::registry().with(layer).set_default();
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    let (cogmap, event) = authorable_map_with_one_event(&app.pool).await;

    let delta = temper_mcp::tools::steward::steward_ingest_delta(
        &svc,
        &parts,
        serde_json::from_value(json!({ "cogmap": cogmap.to_string() }))
            .expect("input deserializes"),
    )
    .await;
    assert!(delta.is_ok(), "the delta crosses the door: {delta:?}");

    let advance = temper_mcp::tools::steward::steward_advance_watermark(
        &svc,
        &parts,
        serde_json::from_value(json!({ "cogmap": cogmap.to_string(), "event_id": event }))
            .expect("input deserializes"),
    )
    .await;
    assert!(advance.is_ok(), "the advance crosses the door: {advance:?}");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // Exactly the two acts' hops, both honored: the harness's own requests carry no
    // carrier, and the relay client makes no extra request, so a retried or degraded hop
    // cannot hide behind a looser count.
    assert_eq!(
        relay_trust_values(&spans.lock().unwrap()),
        vec!["trusted", "trusted"],
        "both acts' carriers were honored and none degraded"
    );
}

/// An advance through the door lands (the ack names the stored watermark), completes the
/// active steward job, and across both of those writes emits no `kb_events` row — there is no family row for attribution to attach to. A change that
/// makes the advance emit must replace this pin with a ledger witness reading `@mcp`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_advance_writes_no_ledger_row(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    let (cogmap, event) = authorable_map_with_one_event(&app.pool).await;

    // An active steward job, so the advance's second write — completing it — runs for real
    // rather than as the no-op it is when nothing is in flight.
    let job = temper_services::services::workflow_job_service::enqueue(
        &app.pool,
        cogmap,
        Persona::Steward.as_str(),
        DispatchType::Steward.as_str(),
    )
    .await
    .expect("enqueue")
    .expect("a fresh map has no job in flight");

    let before = event_count(&app.pool).await;
    let res = temper_mcp::tools::steward::steward_advance_watermark(
        &svc,
        &parts,
        serde_json::from_value(json!({ "cogmap": cogmap.to_string(), "event_id": event }))
            .expect("input deserializes"),
    )
    .await
    .expect("the advance lands through the door");
    let ack: serde_json::Value =
        serde_json::from_str(res.content[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response");
    assert_eq!(ack["watermark"], json!(event), "the advance landed: {ack}");
    let after = event_count(&app.pool).await;
    let status: String = sqlx::query_scalar("SELECT status FROM kb_workflow_jobs WHERE id = $1")
        .bind(job)
        .fetch_one(&app.pool)
        .await
        .expect("job status");
    assert_eq!(
        status, "done",
        "the advance completed the active steward job"
    );

    assert_eq!(
        after, before,
        "the advance emitted a ledger row — attribution now has a row to attach to, and this \
         family needs a ledger witness reading <handle>@mcp in place of this pin"
    );
}
