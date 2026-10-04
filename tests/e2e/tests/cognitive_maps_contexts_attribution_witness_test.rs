#![cfg(feature = "test-db")]
//! The cognitive_maps + contexts families' attribution witness (the G3c witness
//! idiom), carried through the network door with its suite: the actor the door
//! stamps.
//!
//! The invocation envelopes carry no dedicated surface column — the surface survives
//! on the ledger ONLY as the emitter entity's composed name
//! (`<handle>@<surface-marker>`), which `resolve_emitter` resolves at the write
//! (writes.rs:52-64) and `_event_append` stores in `emitter_entity_id`. So the
//! witness opens an envelope through the TOOL and reads the open event's emitter
//! entity name back through `kb_entities`: it must read `<handle>@mcp` — the
//! harness's own handle, the `mcp` marker the door's planted carrier stamps
//! (`X-Temper-Relayed-Surface: mcp` beside the service credential), the attribution
//! the direct command's `Surface::Mcp` field used to carry.
//!
//! Bite contract: a relay client that drops the carrier (or mislabels it) emits
//! `<handle>@web` and reddens the `@mcp` assertion — probed from the relay-client
//! builder, the one place the origin now lives.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

/// The parity harness, once per test — the same shape the suite uses.
async fn harness(
    pool: PgPool,
) -> (
    common::E2eTestApp,
    TemperMcpService,
    axum::http::request::Parts,
) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

/// Open an invocation envelope for the harness against L0 through the tool — the
/// write whose ledger row is the witness. The F2 write gate requires an explicit
/// write grant on the originating map; auto-join membership alone confers read.
async fn open_through_the_tool(
    app: &common::E2eTestApp,
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
) -> Uuid {
    let harness_profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the harness profile");
    common::grant_cogmap_write(&app.pool, L0_COGMAP, harness_profile).await;
    let res = temper_mcp::tools::invocations::invocation_manage(
        svc,
        parts,
        serde_json::from_value(json!({
            "action": "open",
            "trigger_kind": "attribution_witness",
            "originating_cogmap": L0_COGMAP.to_string(),
        }))
        .expect("the wire shape"),
    )
    .await
    .expect("open invocation against L0");
    let text = res.content[0].as_text().expect("a text part").text.as_str();
    serde_json::from_str::<serde_json::Value>(text).expect("the ack JSON")["invocation_id"]
        .as_str()
        .expect("the invocation id")
        .parse()
        .expect("a uuid")
}

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// The relayed `invocation_open` stamps the ledger with the CALLER'S surface: the
/// open event's emitter entity reads `<handle>@mcp` — the handle is the harness's
/// own, and the marker is the surface the door's carrier planted.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_tool_invocation_open_stamps_the_ledger_with_the_callers_surface(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let invocation = open_through_the_tool(&app, &svc, &parts).await;

    // The open event is the envelope's first act; its emitter entity is the
    // provenance row. Read the name back whole — no join through the projection.
    let emitter_name: String = sqlx::query_scalar(
        "SELECT e.name \
         FROM kb_events ev \
         JOIN kb_entities e ON e.id = ev.emitter_entity_id \
         WHERE ev.invocation_id = $1 \
         ORDER BY ev.id \
         LIMIT 1",
    )
    .bind(invocation)
    .fetch_one(&app.pool)
    .await
    .expect("the open event's emitter entity");

    let handle_end = emitter_name
        .rfind('@')
        .map(|at| {
            (
                emitter_name[..at].to_string(),
                emitter_name[at..].to_string(),
            )
        })
        .expect("the emitter name is <handle>@<surface>");
    assert_eq!(
        handle_end.1, "@mcp",
        "the surface marker rides the emitter name — the actor both bindings stamp: {emitter_name}"
    );
    assert!(
        !handle_end.0.is_empty(),
        "the handle half names the caller: {emitter_name}"
    );

    // And the harness's own profile handle is the handle half — the act is THEIRS,
    // not the service's, not another surface's.
    let harness_handle: String =
        sqlx::query_scalar("SELECT handle FROM kb_profiles WHERE email = $1")
            .bind("e2e@test.example.com")
            .fetch_one(&app.pool)
            .await
            .expect("the harness handle");
    assert_eq!(
        handle_end.0, harness_handle,
        "the emitter is the calling profile's entity: {emitter_name} vs {harness_handle}"
    );
}
