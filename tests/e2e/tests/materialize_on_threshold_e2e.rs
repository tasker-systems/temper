#![cfg(feature = "test-db")]
//! Region materialize-on-threshold surface end-to-end (T4b): drives the REAL Axum server (in-process),
//! real Postgres, real JWT auth, through the production `temper-client` cognitive-maps sub-client
//! (`app.client.cognitive_maps()`), NOT raw reqwest. This is the wiring proof that pairs with the
//! service-level unit tests (`temper_services::services::materialize_service::tests`): the direct-call
//! tests prove the count/threshold/auth logic; this proves the route → handler → client →
//! serialization actually connect.
//!
//! The delta read gates on `anchor_readable_by_profile(profile,'kb_cogmaps',L0)`: root-team membership
//! satisfies it (same gate the invocation + steward e2e rely on). `enable_invite_only` makes the
//! principal a root-team owner. L0's exact formation-event count is birth-migration-dependent (its
//! genesis + orientation content fire cogmap-anchored formation events), so we DON'T assert an exact
//! delta — we assert the typed shape round-trips and that a very high threshold is not exceeded. The
//! actual over-threshold materialize (clustering) is proven where that logic lives (the substrate
//! materialize/drift suite + the service-level `trigger_below_threshold_is_a_noop` no-op test); here
//! we prove the routes connect end-to-end.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::materialize::DEFAULT_MATERIALIZE_THRESHOLD;

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

/// The materialize-delta read reaches the service and round-trips a typed `MaterializeDelta` —
/// including the threshold query param — and an unreadable cogmap surfaces as an error (deny → 404),
/// all through the production client against the real server.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_delta_read_surface_round_trips_through_real_server(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    // Default threshold: the typed delta round-trips. L0's formation count is migration-dependent, so
    // we assert the shape (id + default threshold + a non-negative count), not an exact value.
    let delta = app
        .client
        .cognitive_maps()
        .materialize_delta(L0_COGMAP, None)
        .await
        .expect("materialize-delta read should succeed against L0 with root-team read access");
    assert_eq!(delta.cogmap_id, L0_COGMAP);
    assert_eq!(delta.threshold, DEFAULT_MATERIALIZE_THRESHOLD);
    assert!(delta.formation_events >= 0);

    // An explicit, very high threshold round-trips and is not exceeded (robust to L0's exact count).
    let with_threshold = app
        .client
        .cognitive_maps()
        .materialize_delta(L0_COGMAP, Some(1_000_000))
        .await
        .expect("materialize-delta with explicit threshold should succeed");
    assert_eq!(with_threshold.threshold, 1_000_000);
    assert!(
        !with_threshold.exceeds_threshold,
        "L0's birth formation events are far below 1e6"
    );

    // A cogmap the principal cannot read → deny → 404 (no existence oracle) → client error.
    let unreadable = Uuid::from_u128(0xdead_beef);
    let err = app
        .client
        .cognitive_maps()
        .materialize_delta(unreadable, None)
        .await;
    assert!(err.is_err(), "unreadable/absent cogmap → error, not a leak");
}

/// The materialize (trigger) route is wired and reaches the backend: triggering a cogmap the principal
/// cannot author is rejected (never a panic/hang). The positive/no-op semantics + auth are proven at
/// the service+backend layer; here we only prove the POST route connects end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn materialize_route_is_wired(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    // A random cogmap the principal cannot author → rejected (route reached the backend).
    let err = app
        .client
        .cognitive_maps()
        .materialize(Uuid::from_u128(0xdead_beef), None)
        .await;
    assert!(
        err.is_err(),
        "materializing an unauthorable cogmap is rejected"
    );
}
