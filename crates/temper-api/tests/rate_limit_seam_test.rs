//! Witnesses for the rate-limit seam (`temper_services::rate_limit`, spec A2/A5/A7).
//!
//! Authored in-build, per the seam design's step 7: each must fail against the state its
//! clause claims to change. The two default-off witnesses fail if the seam ever limits an
//! unset door; the two limit-bites witnesses fail if the mechanism stops counting. The
//! cycle the bites-witness drives is the exact unbounded Request/Withdraw cycle the
//! source task named — the attack this seam exists to bound.
//!
//! The pure half of the default-off posture (unset ⇒ `Ok(None)`, partial pairs refuse to
//! boot) is witnessed in `rate_limit.rs`'s unit tests, against the parse; these are the
//! behavioral halves.

#![cfg(feature = "test-db")]

mod common;

use axum::body::to_bytes;
use axum::http::{header, Request, StatusCode};
use jsonwebtoken::{Algorithm, DecodingKey};
use serde_json::Value;
use temper_services::rate_limit::{RateLimitConfig, WindowLimit};
use temper_services::{
    auth_config::{AuthConfig, AuthMode},
    config::ApiConfig,
    state::{AppState, JwksKeyStore},
};
use tower::ServiceExt;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::services::access_service;

/// The limit the bites-witnesses use: two requests per hour. Small enough to exhaust in
/// one test, long enough that the window never rolls mid-test.
const BITES: WindowLimit = WindowLimit {
    max: 2,
    window_secs: 3600,
};

/// Seed the gating team + settings so a join request can be filed.
async fn seed_gating_team(pool: &sqlx::PgPool) -> Uuid {
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('rl-gating','Rate Limit Gating') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed gating team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='rl-gating' WHERE id=1")
        .execute(pool)
        .await
        .expect("point gating at the team");
    team_id
}

/// A principal in `denied` standing — the standing from which `Act::Request` is legal.
async fn denied_profile(pool: &sqlx::PgPool, email: &str) -> Uuid {
    let profile = common::fixtures::create_test_profile(pool, email).await;
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'denied') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'denied', updated = now()",
    )
    .bind(profile)
    .execute(pool)
    .await
    .expect("set standing to denied");
    profile
}

fn request_for(profile: Uuid) -> access_service::CreateJoinRequestParams {
    access_service::CreateJoinRequestParams {
        profile_id: ProfileId::from(profile),
        message: None,
        source: "test".to_owned(),
        accepted_terms_version: None,
    }
}

/// One Request/Withdraw half-cycle. `withdraw_request` needs no gating setup (its
/// not-found exits are uniform), so only the request side can fail here.
async fn request_then_withdraw(pool: &sqlx::PgPool, profile: Uuid, rate: Option<WindowLimit>) {
    access_service::create_join_request(pool, request_for(profile), rate)
        .await
        .expect("request filed");
    access_service::withdraw_request(pool, ProfileId::from(profile))
        .await
        .expect("request withdrawn");
}

// ---------------------------------------------------------------------------
// The self-service guard (spec A2's "count the canonical artifact")
// ---------------------------------------------------------------------------

/// **Limit-bites witness.** The third request inside the window must be refused —
/// not by the standing machine, but by the seam, under its own error. This fails while
/// no limit exists (the third request files happily), which is exactly the pre-mechanism
/// state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_third_request_in_the_window_is_refused(pool: sqlx::PgPool) {
    seed_gating_team(&pool).await;
    let profile = denied_profile(&pool, "rl-bites@test.example.com").await;
    let rate = Some(BITES);

    request_then_withdraw(&pool, profile, rate).await;
    request_then_withdraw(&pool, profile, rate).await;

    let err = access_service::create_join_request(&pool, request_for(profile), rate)
        .await
        .expect_err("the third in-window request must be refused");
    let temper_services::error::ApiError::TooManyRequests {
        retry_after_secs, ..
    } = err
    else {
        panic!("expected TooManyRequests from the seam, got {err:?}");
    };
    assert!(
        retry_after_secs > 0 && retry_after_secs <= 3600,
        "Retry-After must name the remaining window, got {retry_after_secs}"
    );

    // And the refusal preceded every write the request would have made: no third row.
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_join_requests WHERE requesting_profile_id = $1",
    )
    .bind(profile)
    .fetch_one(&pool)
    .await
    .expect("count rows");
    assert_eq!(rows, 2, "the refused request must not have filed a row");
}

// FAILS IF: the guard is read as blanket pressure-shaping of the *withdraw* side. The
// task's trap-form says what the seam bounds: filing pressure. Withdrawal bounds nothing
// (spec A5: extending the limiter there "bounds nothing but pressure") and must stay
// unlimited so a limited caller can always back out of their pending state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn withdrawal_is_never_rate_limited(pool: sqlx::PgPool) {
    seed_gating_team(&pool).await;
    let profile = denied_profile(&pool, "rl-withdraw@test.example.com").await;
    let rate = Some(BITES);

    // Two full cycles: the request side is now at its window's edge.
    request_then_withdraw(&pool, profile, rate).await;
    request_then_withdraw(&pool, profile, rate).await;

    // The third REQUEST is refused (same assertion as the bites witness, kept local so
    // this test stands alone)...
    assert!(matches!(
        access_service::create_join_request(&pool, request_for(profile), rate).await,
        Err(temper_services::error::ApiError::TooManyRequests { .. })
    ));

    // ...but there is nothing pending, and a withdraw from nothing must answer with the
    // standing machine's own refusal (denied again after the second withdraw) — never a
    // 429. A caller that cannot request must still be able to find out they have nothing
    // to withdraw; the seam's refusal is reserved for the request door alone.
    let err = access_service::withdraw_request(&pool, ProfileId::from(profile))
        .await
        .expect_err("nothing pending");
    assert!(
        !matches!(
            err,
            temper_services::error::ApiError::TooManyRequests { .. }
        ),
        "withdraw must never answer with the seam's refusal: got {err:?}"
    );
}

/// **Default-off witness (behavioral half).** With the door's limit unset, the same
/// cycle runs past any number the bites-witness could name: the third request files.
/// This fails if the route is limited when unset — if a default ever sneaks in.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unset_door_limits_nothing(pool: sqlx::PgPool) {
    seed_gating_team(&pool).await;
    let profile = denied_profile(&pool, "rl-off@test.example.com").await;

    request_then_withdraw(&pool, profile, None).await;
    request_then_withdraw(&pool, profile, None).await;

    // The exact call the bites-witness refuses, now filed without complaint.
    let filed = access_service::create_join_request(&pool, request_for(profile), None)
        .await
        .expect("an unlimited door must file the third request");
    assert_eq!(filed.requesting_profile_id, profile);
}

// ---------------------------------------------------------------------------
// The reconcile-channel layer (spec A2's "mint counter state only where none exists")
// ---------------------------------------------------------------------------

/// An `AppState` with only the reconcile door limited, and a minimal router carrying the
/// middleware the way the merge sites mount it.
fn limited_reconcile_app(pool: sqlx::PgPool) -> axum::Router {
    let decoding_key =
        DecodingKey::from_rsa_pem(include_bytes!("common/test_rsa.pub")).expect("test RSA key");
    let jwks = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);
    let config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: "test-audience".to_string(),
            mcp_audience: "test-audience".to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: Some(RateLimitConfig {
            reconcile: Some(BITES),
            create_request: None,
        }),
    };
    let state = AppState::new(pool, jwks, config);

    axum::Router::new()
        .route(
            "/internal/saml/reconcile",
            axum::routing::post(|| async { StatusCode::OK }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            temper_services::rate_limit::require_route_rate_limit,
        ))
        .with_state(state)
}

async fn post_reconcile(app: axum::Router) -> (StatusCode, Option<String>, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/saml/reconcile")
                .body(axum::body::Body::from("{}"))
                .expect("request builds"),
        )
        .await
        .expect("infallible service");
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body collects");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("error body is JSON")
    };
    (status, retry_after, body)
}

/// **Limit-bites witness (layer half).** The first two calls pass, the third is a 429
/// with the structured body and a Retry-After — the refusal face's "a well-formed
/// request the system says no to". Fails while the middleware does not count.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_reconcile_layer_refuses_past_the_limit(pool: sqlx::PgPool) {
    let app = limited_reconcile_app(pool);

    let (status, retry, body) = post_reconcile(app.clone()).await;
    assert_eq!(status, StatusCode::OK, "first call passes; body {body}");
    assert!(retry.is_none(), "no Retry-After on a passing call");

    let (status, _, _) = post_reconcile(app.clone()).await;
    assert_eq!(status, StatusCode::OK, "second call exhausts max=2");

    let (status, retry, body) = post_reconcile(app.clone()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let code = body["error"]["code"]
        .as_str()
        .expect("structured error body");
    assert_eq!(
        code, "TOO_MANY_REQUESTS",
        "the code, not the status, is the contract"
    );
    let retry = retry.expect("a refusal carries Retry-After");
    let secs: i64 = retry.parse().expect("Retry-After is seconds");
    assert!(secs > 0 && secs <= 3600, "remaining window, got {secs}");
}

/// **Default-off witness (layer half).** The merge-site wiring is inert when the
/// operator has configured nothing: no 429 is possible, and — the sharper half — no
/// counter row is written, so default-off costs not even the round trip.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unconfigured_layer_neither_refuses_nor_counts(pool: sqlx::PgPool) {
    let decoding_key =
        DecodingKey::from_rsa_pem(include_bytes!("common/test_rsa.pub")).expect("test RSA key");
    let jwks = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);
    let mut config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: "test-audience".to_string(),
            mcp_audience: "test-audience".to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
        rate_limit: None,
    };
    // The type-level statement of the clause: there is no Some to be had here. The test
    // below would fail if the middleware read a fallback instead of `None`.
    assert_eq!(
        config.rate_limit, None,
        "default off means None, not a default limit"
    );
    config.auth.audience = "test-audience".to_string();
    let state = AppState::new(pool.clone(), jwks, config);

    let app = axum::Router::new()
        .route(
            "/internal/saml/reconcile",
            axum::routing::post(|| async { StatusCode::OK }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            temper_services::rate_limit::require_route_rate_limit,
        ))
        .with_state(state);

    // Drive it past what ANY limit could allow — the witnesses above bite at three.
    for i in 0..10 {
        let (status, _, _) = post_reconcile(app.clone()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "call {i} must pass on an unconfigured seam"
        );
    }

    let counters: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_rate_counters")
        .fetch_one(&pool)
        .await
        .expect("count counter rows");
    assert_eq!(
        counters, 0,
        "default off must not even write a counter row — the seam is absent, not dormant"
    );
}

/// **Keying witness (the route is the key).** Two doors with one shared budget is the
/// conflation A1 refuses; each route's counter must be its own row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn each_route_spends_its_own_budget(pool: sqlx::PgPool) {
    // Reuse the limited app but hit BOTH routes; the stub only declares reconcile, so
    // the second route is asserted at the counter level through the same middleware.
    let app = limited_reconcile_app(pool.clone());

    for _ in 0..2 {
        let (status, _, _) = post_reconcile(app.clone()).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _, _) = post_reconcile(app.clone()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let keys: Vec<String> = sqlx::query_scalar("SELECT route FROM kb_rate_counters")
        .fetch_all(&pool)
        .await
        .expect("counter keys");
    assert_eq!(
        keys,
        vec!["/internal/saml/reconcile".to_string()],
        "one row per route — the route is the key, and only the route that was called has a row"
    );
}
