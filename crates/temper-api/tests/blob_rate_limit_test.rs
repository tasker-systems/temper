#![cfg(feature = "test-db")]
//! Witnesses for the per-principal blob-write budget
//! (`temper_services::rate_limit::PrincipalDoor::BlobWrite`), driven through the real router
//! (`create_app`), so the mount at the route table is what is under test, not a hand-built
//! router.
//!
//! The budget's two clauses: a configured door refuses the call past the window with the seam's
//! refusal shape, and the same door unconfigured neither refuses nor counts. The keying clauses:
//! one budget per principal, shared across the five write doors, and never spent by a caller a
//! gate refused or by a blob read. Counters live in `kb_principal_rate_counters`, which no other
//! door writes, so a count of its rows is a count of this budget's principals.

mod common;

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use common::fixtures;
use common::{generate_test_jwt, setup_test_app_with_state, TestApp};
use temper_services::config::{BlobConfig, BlobCredentialMode};
use temper_services::rate_limit::{RateLimitConfig, WindowLimit};
use temper_substrate::blob_store::InMemoryBlobStore;

/// A test-sized `BlobConfig`, as `blob_handler_test` builds it.
fn blob_cfg() -> BlobConfig {
    BlobConfig {
        store_id: "store_test".to_string(),
        read_write_token: Some("vercel_rw_test_store_test".to_string()),
        credential_mode: BlobCredentialMode::Token,
        oidc_token_source: Arc::new(|| None),
        max_bytes: 1 << 20,
        allowlist: vec!["image/png".to_string()],
        single_request_max_bytes: 64 * 1024,
    }
}

/// The app with a fake provider, the blob config, and the given blob-write budget (`None`: the
/// shipped default, nothing configured).
async fn app_with_budget(pool: PgPool, budget: Option<WindowLimit>) -> TestApp {
    setup_test_app_with_state(pool, move |state| {
        state.blob_store = Some(Arc::new(InMemoryBlobStore::default()));
        let mut config = (*state.config).clone();
        config.blob = Some(blob_cfg());
        config.rate_limit = budget.map(|limit| RateLimitConfig {
            reconcile: None,
            create_request: None,
            blob_write: Some(limit),
        });
        state.config = Arc::new(config);
    })
    .await
}

fn budget(max: i64) -> Option<WindowLimit> {
    Some(WindowLimit {
        max,
        window_secs: 3600,
    })
}

/// A provisioned profile with its own context, and the JWT that reaches it.
async fn principal(pool: &PgPool) -> (Uuid, String) {
    let email = format!("blob-rl-{}@example.com", Uuid::new_v4());
    let (profile, ctx) = fixtures::create_test_profile_with_context(pool, &email).await;
    (ctx, generate_test_jwt(&format!("test|{profile}"), &email))
}

/// A single-request commit of `bytes` into `ctx`.
async fn commit(app: &TestApp, token: &str, ctx: Uuid, bytes: Vec<u8>) -> reqwest::Response {
    let file = reqwest::multipart::Part::bytes(bytes)
        .file_name("figure.png")
        .mime_str("image/png")
        .expect("mime");
    let form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("home_table", "kb_contexts")
        .text("home_id", ctx.to_string());
    app.client
        .post(app.url("/api/blobs"))
        .header("Authorization", format!("Bearer {token}"))
        .multipart(form)
        .send()
        .await
        .expect("request failed")
}

async fn counter_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_principal_rate_counters")
        .fetch_one(pool)
        .await
        .expect("count counter rows")
}

/// One request to `path` with `method`, as `token` (or anonymous), with a small JSON body.
async fn send(
    app: &TestApp,
    method: reqwest::Method,
    path: &str,
    token: Option<&str>,
) -> reqwest::Response {
    let mut request = app
        .client
        .request(method, app.url(path))
        .json(&serde_json::json!({}));
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    request.send().await.expect("request failed")
}

/// The five blob write doors, as (method, path) with throwaway ids: past the budget, the layer
/// refuses before any handler reads the id or the body.
fn write_doors() -> Vec<(reqwest::Method, String)> {
    let id = Uuid::now_v7();
    vec![
        (reqwest::Method::POST, "/api/blobs".to_string()),
        (reqwest::Method::POST, "/api/blobs/uploads".to_string()),
        (
            reqwest::Method::POST,
            format!("/api/blobs/uploads/{id}/segments?seq=0"),
        ),
        (
            reqwest::Method::POST,
            format!("/api/blobs/uploads/{id}/finalize"),
        ),
        (reqwest::Method::DELETE, format!("/api/blobs/{id}")),
    ]
}

/// **Limit-bites witness.** With a budget of two, the third commit inside the window is refused
/// with the seam's refusal: `429`, code `TOO_MANY_REQUESTS`, and a `Retry-After` naming the rest
/// of the window. Fails while the doors carry no budget: the third commit succeeds.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_configured_blob_door_refuses_past_the_budget(pool: PgPool) {
    let app = app_with_budget(pool, budget(2)).await;
    let (ctx, token) = principal(&app.pool).await;

    for i in 0..2u8 {
        let resp = commit(&app, &token, ctx, vec![i, 1, 2, 3]).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "commit {i} is inside the budget"
        );
    }

    let resp = commit(&app, &token, ctx, vec![9, 9, 9]).await;
    assert_eq!(
        resp.status().as_u16(),
        429,
        "the third commit is past the budget"
    );
    let retry: i64 = resp
        .headers()
        .get("retry-after")
        .expect("a refusal carries Retry-After")
        .to_str()
        .expect("ascii")
        .parse()
        .expect("Retry-After is seconds");
    assert!(
        retry > 0 && retry <= 3600,
        "the rest of the window, got {retry}"
    );
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["error"]["code"], "TOO_MANY_REQUESTS", "body {body}");
}

/// **Default-off witness.** Unconfigured, the same door takes ten commits, more than any budget
/// above allows, and writes no counter row: the seam is absent, not dormant.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unconfigured_blob_door_neither_refuses_nor_counts(pool: PgPool) {
    let app = app_with_budget(pool, None).await;
    let (ctx, token) = principal(&app.pool).await;

    for i in 0..10u8 {
        let resp = commit(&app, &token, ctx, vec![i, 4, 5]).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "commit {i} on an unconfigured door"
        );
    }
    assert_eq!(
        counter_rows(&app.pool).await,
        0,
        "default off writes no counter row"
    );
}

/// **Mount witness: every write door carries the budget, and no read does.** A budget of zero
/// declines every call on the budgeted doors, so each of the five answers `429`; the blob reads
/// and `relate`, which share their paths, answer as they would with no budget at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_write_door_carries_the_budget_and_no_read_does(pool: PgPool) {
    let app = app_with_budget(pool, budget(0)).await;
    let (_ctx, token) = principal(&app.pool).await;

    for (method, path) in write_doors() {
        let resp = send(&app, method.clone(), &path, Some(&token)).await;
        assert_eq!(
            resp.status().as_u16(),
            429,
            "{method} {path} must spend the budget"
        );
    }

    let id = Uuid::now_v7();
    let reads = [
        (reqwest::Method::GET, "/api/blobs".to_string()),
        (reqwest::Method::GET, format!("/api/blobs/{id}")),
        (reqwest::Method::GET, format!("/api/blobs/uploads/{id}")),
        (reqwest::Method::GET, format!("/api/blobs/{id}/relations")),
        (reqwest::Method::POST, format!("/api/blobs/{id}/relations")),
    ];
    for (method, path) in reads {
        let resp = send(&app, method.clone(), &path, Some(&token)).await;
        assert_ne!(
            resp.status().as_u16(),
            429,
            "{method} {path} carries no budget"
        );
    }
}

/// **Shared-budget witness.** The five doors spend one budget: a segmented upload's begin,
/// append and finalize use three calls of a budget of three, and the commit after them is
/// refused. Under per-door budgets the commit would pass.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_write_doors_share_one_budget(pool: PgPool) {
    let app = app_with_budget(pool, budget(3)).await;
    let (ctx, token) = principal(&app.pool).await;

    let resp = app
        .client
        .post(app.url("/api/blobs/uploads"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "home_table": "kb_contexts",
            "home_id": ctx,
            "content_type": "image/png",
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "begin");
    let begun: serde_json::Value = resp.json().await.expect("json");
    let upload_id = begun["upload_id"].as_str().expect("upload_id").to_string();

    let segment = vec![7u8; 512];
    let resp = app
        .client
        .post(app.url(&format!("/api/blobs/uploads/{upload_id}/segments?seq=0")))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/octet-stream")
        .body(segment)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "append");

    let resp = app
        .client
        .post(app.url(&format!("/api/blobs/uploads/{upload_id}/finalize")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "expected_segments": 1, "expected_total_bytes": 512 }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200, "finalize");

    let resp = commit(&app, &token, ctx, vec![1, 2, 3]).await;
    assert_eq!(
        resp.status().as_u16(),
        429,
        "the commit is the fourth call against one budget of three"
    );
    assert_eq!(
        counter_rows(&app.pool).await,
        1,
        "one counter row for the one principal"
    );
}

/// **Keying witness: the principal is the key.** One principal exhausting its budget leaves
/// another's untouched. Under a shared constant key the second principal's first commit would be
/// refused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn each_principal_spends_its_own_budget(pool: PgPool) {
    let app = app_with_budget(pool, budget(1)).await;
    let (ctx_a, token_a) = principal(&app.pool).await;
    let (ctx_b, token_b) = principal(&app.pool).await;

    assert_eq!(
        commit(&app, &token_a, ctx_a, vec![1])
            .await
            .status()
            .as_u16(),
        200
    );
    assert_eq!(
        commit(&app, &token_a, ctx_a, vec![2])
            .await
            .status()
            .as_u16(),
        429
    );
    assert_eq!(
        commit(&app, &token_b, ctx_b, vec![3])
            .await
            .status()
            .as_u16(),
        200,
        "a second principal's budget is its own"
    );
    assert_eq!(
        counter_rows(&app.pool).await,
        2,
        "one counter row per principal"
    );
}

/// **Ordering witness.** The budget sits inside the tier's gates: an unauthenticated caller gets
/// the `401` on every write door and the counter never hears of it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unauthenticated_caller_spends_no_budget(pool: PgPool) {
    let app = app_with_budget(pool, budget(0)).await;

    for (method, path) in write_doors() {
        let resp = send(&app, method.clone(), &path, None).await;
        assert_eq!(
            resp.status().as_u16(),
            401,
            "{method} {path} refuses at its gate"
        );
    }
    assert_eq!(
        counter_rows(&app.pool).await,
        0,
        "a refused caller spends nothing"
    );
}

/// **Declared-behaviour witness: an unmounted method on a budgeted path spends the budget.** The
/// budget wraps the path whole, the method router's 405 fallback included, so with a budget of
/// zero an admitted caller's `PUT` on a write door's path is answered 429, not 405. Pins what
/// `routes::mount_group` states; if the budget ever stops wrapping the fallback, this turns red
/// and that comment must change with it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unmounted_method_on_a_budgeted_path_spends_the_budget(pool: PgPool) {
    let app = app_with_budget(pool, budget(0)).await;
    let (_ctx, token) = principal(&app.pool).await;

    let id = Uuid::now_v7();
    let resp = send(
        &app,
        reqwest::Method::PUT,
        &format!("/api/blobs/{id}"),
        Some(&token),
    )
    .await;
    assert_eq!(
        resp.status().as_u16(),
        429,
        "the 405 fallback is inside the budget"
    );
}

/// **Erasure witness: a principal's counters go with its profile.** The counter references the
/// profile by foreign key with `ON DELETE CASCADE`, so the erasure manifests' fk derivation finds
/// it and a deleted profile leaves no counter behind.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_profiles_counters_go_with_the_profile(pool: PgPool) {
    let app = app_with_budget(pool, budget(5)).await;
    let (ctx, token) = principal(&app.pool).await;
    assert_eq!(
        commit(&app, &token, ctx, vec![1]).await.status().as_u16(),
        200
    );
    assert_eq!(counter_rows(&app.pool).await, 1);

    let cascades: bool = sqlx::query_scalar(
        "SELECT confdeltype = 'c' FROM pg_constraint \
          WHERE conrelid = 'kb_principal_rate_counters'::regclass AND contype = 'f' \
            AND confrelid = 'kb_profiles'::regclass",
    )
    .fetch_one(&app.pool)
    .await
    .expect("the profile foreign key");
    assert!(
        cascades,
        "the counter's profile reference cascades on delete"
    );
}

/// **Ordering witness: the budget is inside the system-access gate.** A verified caller with no
/// approved standing (a JIT-provisioned profile is born denied) is refused 403 by
/// `require_system_access` on every write door and spends nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unapproved_caller_spends_no_budget(pool: PgPool) {
    let app = app_with_budget(pool, budget(0)).await;
    let email = format!("blob-rl-unapproved-{}@example.com", Uuid::new_v4());
    let token = generate_test_jwt(&format!("test|{}", Uuid::new_v4()), &email);

    for (method, path) in write_doors() {
        let resp = send(&app, method.clone(), &path, Some(&token)).await;
        assert_eq!(
            resp.status().as_u16(),
            403,
            "{method} {path} refuses at system access"
        );
    }
    assert_eq!(
        counter_rows(&app.pool).await,
        0,
        "an unapproved caller spends nothing"
    );
}
