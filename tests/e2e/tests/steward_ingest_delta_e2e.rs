#![cfg(feature = "test-db")]
//! Steward ingest-trigger surface end-to-end (T4a): drives the REAL Axum server (in-process),
//! real Postgres, real JWT auth, through the production `temper-client` steward sub-client
//! (`app.client.steward()`), NOT raw reqwest. This is the wiring proof that pairs with the
//! service-level unit tests (`temper_services::services::steward_service::tests`): the direct-call
//! tests prove the counting/threshold/watermark/auth logic; this proves the route → handler →
//! client → serialization actually connect.
//!
//! The delta read gates on `anchor_readable_by_profile(profile,'kb_cogmaps',L0)`: root-team
//! membership satisfies it (same gate the invocation e2e relies on). `enable_invite_only` makes
//! the principal a root-team owner. L0's team (`temper-system`) owns no event-bearing context in a
//! fresh DB, so the delta is 0/0 — enough to prove the vertical connects and the shape round-trips.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::steward::DEFAULT_STEWARD_INGEST_THRESHOLD;

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

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

/// The delta read reaches the service and round-trips a typed `IngestDelta` — including the
/// threshold query param — and an unreadable cogmap surfaces as an error (deny → 404), all through
/// the production client against the real server.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delta_read_surface_round_trips_through_real_server(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    // Default threshold: L0 has no team-context ingest in a fresh DB → 0/0, below default.
    let delta = app
        .client
        .steward()
        .delta(L0_COGMAP, None)
        .await
        .expect("delta read should succeed against L0 with root-team read access");
    assert_eq!(delta.cogmap_id, L0_COGMAP);
    assert_eq!(delta.new_resources, 0, "no team-context ingest yet");
    assert_eq!(delta.new_events, 0);
    assert_eq!(delta.threshold, DEFAULT_STEWARD_INGEST_THRESHOLD);
    assert!(!delta.exceeds_threshold, "0 < default threshold");

    // The explicit threshold query param round-trips.
    let with_threshold = app
        .client
        .steward()
        .delta(L0_COGMAP, Some(1))
        .await
        .expect("delta with explicit threshold should succeed");
    assert_eq!(with_threshold.threshold, 1);
    assert!(!with_threshold.exceeds_threshold, "0 < 1");

    // A cogmap the principal cannot read → deny → 404 (no existence oracle) → client error.
    let unreadable = Uuid::from_u128(0xdead_beef);
    let err = app.client.steward().delta(unreadable, None).await;
    assert!(err.is_err(), "unreadable/absent cogmap → error, not a leak");
}

/// The watermark-advance route is wired and reaches the backend: advancing a cogmap the principal
/// cannot author is rejected (never a panic/hang). The positive advance semantics + auth are proven
/// at the service+backend layer (`advance_requires_cogmap_write_grant`,
/// `advancing_watermark_shrinks_the_delta`); here we only prove the POST route connects end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn advance_watermark_route_is_wired(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    // A random cogmap/event the principal cannot author → rejected (route reached the backend).
    let err = app
        .client
        .steward()
        .advance_watermark(Uuid::from_u128(0xdead_beef), Uuid::from_u128(1))
        .await;
    assert!(err.is_err(), "advancing an unauthorable cogmap is rejected");
}
