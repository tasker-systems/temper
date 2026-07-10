#![cfg(feature = "test-db")]
//! Steward dispatch-tick surface end-to-end (goal 019f3220): drives the REAL Axum server
//! (in-process), real Postgres, real JWT auth, hitting `POST /api/steward/dispatch` with raw HTTP —
//! which is exactly how the Eve code dispatcher calls it in production (a bearer-token `fetch`, not
//! the Rust client). This is the vertical-wiring proof that pairs with the service-level unit tests
//! (`temper_services::services::{steward_service, workflow_job_service}::tests`): those prove the
//! sweep/queue logic; this proves route → handler → backend composite (reap→sweep→enqueue→claim) →
//! serialization actually connect, and that single-flight holds across two real dispatch calls.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::steward::DispatchTickResponse;

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

/// Seed a single drifted, principal-readable, team-joined cogmap: a team the principal is a member
/// of (→ cogmap read), a team-owned context, and 6 `resource_created` events (above the default
/// threshold 5). Returns the cogmap id. Mirrors the service-test seed shape.
async fn seed_drifted_map(pool: &sqlx::PgPool, principal: Uuid) -> Uuid {
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('dispatchteam', 'D') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(principal)
    .execute(pool)
    .await
    .unwrap();

    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('dmap', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();

    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, 'building', 'Building') RETURNING id",
    )
    .bind(team)
    .fetch_one(pool)
    .await
    .unwrap();
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e') RETURNING id",
    )
    .bind(principal)
    .fetch_one(pool)
    .await
    .unwrap();
    for _ in 0..6 {
        sqlx::query(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'resource_created'), $1, 'kb_contexts', $2)",
        )
        .bind(entity)
        .bind(ctx)
        .execute(pool)
        .await
        .unwrap();
    }
    cogmap
}

/// Dispatch exactly as the steward cron does: a bearer-token POST with an empty body, optionally
/// carrying the per-tick `x-steward-correlation-id` header.
async fn dispatch_with(
    app: &common::E2eTestApp,
    correlation: Option<&str>,
) -> DispatchTickResponse {
    let mut req = app
        .reqwest_client
        .post(app.url("/api/steward/dispatch"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({}));
    if let Some(id) = correlation {
        req = req.header("x-steward-correlation-id", id);
    }
    let resp = req.send().await.expect("dispatch request failed");
    assert_eq!(resp.status(), StatusCode::OK, "dispatch should succeed");
    resp.json().await.expect("dispatch json parse")
}

async fn dispatch(app: &common::E2eTestApp) -> DispatchTickResponse {
    dispatch_with(app, None).await
}

/// Grant `profile` explicit write on `cogmap`. A top-level `invocation_open` is gated on cogmap-write
/// (`cogmap_authorable_by_profile`), which the steward principal holds in production — it authors the
/// map it tends. The drift sweep only needs read, which is why the claim tests get by without this.
async fn grant_cogmap_write(pool: &sqlx::PgPool, cogmap: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant cogmap write");
}

/// Open an invocation over `cogmap` the way a fanned-out steward session does.
async fn open_invocation(app: &common::E2eTestApp, cogmap: Uuid) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/invocations"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({
            "trigger_kind": "delegated",
            "originating_cogmap": cogmap,
        }))
        .send()
        .await
        .expect("invocation open failed");
    assert_eq!(resp.status(), StatusCode::OK, "open should succeed");
    let body: serde_json::Value = resp.json().await.expect("open json parse");
    body["invocation_id"]
        .as_str()
        .expect("invocation_id missing")
        .parse()
        .expect("invocation_id parse")
}

async fn job_correlation(pool: &sqlx::PgPool, job: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT correlation_id FROM kb_workflow_jobs WHERE id = $1")
        .bind(job)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The dispatch route is wired end-to-end: against a fresh DB with no drifted maps, the composite
/// runs server-side (reap→sweep→enqueue→claim) and round-trips a well-formed empty response.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dispatch_route_is_wired_and_returns_empty_when_no_drift(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;

    let body = dispatch(&app).await;
    assert!(
        body.claimed.is_empty(),
        "no drifted team-joined maps → nothing claimed"
    );
}

/// A drifted map is swept, enqueued, and claimed in one dispatch; a second immediate dispatch claims
/// nothing (single-flight — the first job is in_progress), proving the concurrency guard end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dispatch_claims_a_drifted_map_then_single_flight_blocks_the_next(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;
    let cogmap = seed_drifted_map(&pool, principal).await;

    let first = dispatch(&app).await;
    assert_eq!(first.claimed.len(), 1, "the one drifted map is claimed");
    assert_eq!(first.claimed[0].cogmap_id, cogmap);
    assert_eq!(first.claimed[0].attempts, 1, "first claim → attempts 1");

    let second = dispatch(&app).await;
    assert!(
        second.claimed.is_empty(),
        "single-flight: the in_progress job is not re-claimed"
    );
}

// ── Per-tick correlation (task 019f4be3) ─────────────────────────────────────
//
// The cron mints one id per tick and sends it as `x-steward-correlation-id`. These drive the REAL
// header over real HTTP, so they pin what the deployed cron actually does — not a Rust-level shim.

/// The full chain the steward runs each hour, minus the model: the cron dispatches with its tick id,
/// the server stamps every job it claims, and the session that job spawns inherits the id onto its
/// invocation — with the session passing NOTHING. This is the acceptance criterion.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_tick_threads_its_correlation_from_the_header_to_the_invocation(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;
    let cogmap = seed_drifted_map(&pool, principal).await;
    grant_cogmap_write(&pool, cogmap, principal).await;

    // `crypto.randomUUID()` shape: a v4 UUID, exactly what the cron sends.
    let tick = "6f1e5a2c-9d3b-4c7e-8a10-2b4d6e8f0a12";
    let body = dispatch_with(&app, Some(tick)).await;

    assert_eq!(
        body.correlation_id.map(|c| c.to_string()).as_deref(),
        Some(tick),
        "the response echoes the correlation the server parsed and stamped"
    );
    assert_eq!(body.claimed.len(), 1);
    assert_eq!(
        job_correlation(&pool, body.claimed[0].id).await,
        Some(tick.parse().unwrap()),
        "the claimed job records the tick that claimed it"
    );

    // The fanned-out session opens its envelope over the claimed map, threading nothing.
    let invocation = open_invocation(&app, cogmap).await;
    let inherited: Option<Uuid> =
        sqlx::query_scalar("SELECT correlation_id FROM kb_invocations WHERE id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        inherited,
        Some(tick.parse().unwrap()),
        "the run inherits its tick server-side from the active claimed job"
    );

    // …and it is visible on the read surface, so a tick's runs are queryable, not just greppable.
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/invocations/{invocation}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("invocation show failed");
    assert_eq!(resp.status(), StatusCode::OK);
    let view: serde_json::Value = resp.json().await.expect("show json parse");
    assert_eq!(view["correlation_id"].as_str(), Some(tick));
}

/// No header — the pre-existing caller. Dispatch works, nothing is stamped, and the run that follows
/// self-roots. Correlation is a correlation aid, never a gate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_dispatch_without_the_header_claims_and_self_roots(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;
    let cogmap = seed_drifted_map(&pool, principal).await;
    grant_cogmap_write(&pool, cogmap, principal).await;

    let body = dispatch(&app).await;
    assert_eq!(body.claimed.len(), 1, "claiming is unaffected");
    assert!(body.correlation_id.is_none(), "nothing to echo");
    assert_eq!(job_correlation(&pool, body.claimed[0].id).await, None);

    let invocation = open_invocation(&app, cogmap).await;
    let inherited: Option<Uuid> =
        sqlx::query_scalar("SELECT correlation_id FROM kb_invocations WHERE id = $1")
            .bind(invocation)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(inherited, None, "no tick to inherit");
}

/// A malformed header must never 400. A tick whose id is garbage still has drift to tend; refusing
/// the work over a provenance field would trade a broken trace for a broken steward.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_malformed_correlation_header_is_ignored_not_rejected(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let principal = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, principal).await;
    seed_drifted_map(&pool, principal).await;

    let body = dispatch_with(&app, Some("not-a-uuid")).await;
    assert_eq!(body.claimed.len(), 1, "the tick still does its work");
    assert!(
        body.correlation_id.is_none(),
        "the echo tells the caller its id did not survive parsing"
    );
    assert_eq!(job_correlation(&pool, body.claimed[0].id).await, None);
}
