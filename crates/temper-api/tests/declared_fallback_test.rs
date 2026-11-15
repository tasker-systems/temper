#![cfg(feature = "test-db")]
//! The declared fallback: what an unmatched path answers.
//!
//! An unauthenticated caller gets the same refusal on an unmatched path as on a gated route, so it
//! cannot tell which paths exist. A validly signed caller still gets the 404, and the fallback
//! never provisions a profile to give it. Under `/internal/` every unmatched path is the refusal,
//! whoever asks.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

/// Status, `content-type` and body: the bytes a caller can compare.
async fn answer(response: reqwest::Response) -> (u16, Option<String>, String) {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (status, content_type, response.text().await.expect("body"))
}

async fn get(
    app: &common::TestApp,
    path: &str,
    bearer: Option<&str>,
) -> (u16, Option<String>, String) {
    let mut request = app.client.get(app.url(path));
    if let Some(token) = bearer {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    answer(request.send().await.expect("sent")).await
}

/// The uniform refusal, as a gated route gives it to a caller with no bearer.
async fn u401(app: &common::TestApp) -> (u16, Option<String>, String) {
    let refusal = get(app, "/api/profile", None).await;
    assert_eq!(refusal.0, 401, "{refusal:?}");
    refusal
}

/// An approved person; returns their bearer.
async fn person(pool: &PgPool, approved: bool) -> String {
    let email = format!("fallback-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(pool, &email).await;
    if !approved {
        sqlx::query("DELETE FROM kb_principal_standing WHERE profile_id = $1")
            .bind(profile_id)
            .execute(pool)
            .await
            .expect("remove standing");
    }
    common::generate_test_jwt(&format!("test|{profile_id}"), &email)
}

async fn profile_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
        .fetch_one(pool)
        .await
        .expect("count")
}

// FAILS IF: an unmatched path answers an unauthenticated caller anything but the refusal a gated
// route gives, by any bearer that does not verify: the 404 would name it as unmatched.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unmatched_is_u401_to_the_anonymous(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let expected = u401(&app).await;
    let expired = common::generate_expired_jwt("test|nobody", "nobody@example.com");
    for path in ["/api/no-such", "/no-such", "/api/resources/no/such/thing"] {
        for bearer in [None, Some("garbage"), Some(expired.as_str())] {
            assert_eq!(get(&app, path, bearer).await, expected, "{path} {bearer:?}");
        }
    }
}

// FAILS IF: the fallback provisions a profile for a validly signed identity that has none, or
// refuses it: a mistyped URL would mint an account, or tell a signed caller less than a 404.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_fallback_never_provisions(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let before = profile_count(&pool).await;
    let stranger = common::generate_test_jwt(
        &format!("test|stranger-{}", Uuid::new_v4().simple()),
        &format!("stranger-{}@example.com", Uuid::new_v4().simple()),
    );
    let (status, _, body) = get(&app, "/api/no-such", Some(&stranger)).await;
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("No route matches"), "{body}");
    assert_eq!(
        profile_count(&pool).await,
        before,
        "the fallback created a profile"
    );
}

// FAILS IF: a validly signed bearer reaches past a refused prefix: under `/internal/` no bearer
// route lives, so a bearer must change nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_internal_prefix_is_refused_with_a_bearer(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let expected = u401(&app).await;
    let bearer = person(&pool, true).await;
    for path in [
        "/internal/no-such",
        "/internal/saml/reconcile/extra",
        "/internal/",
    ] {
        assert_eq!(
            get(&app, path, Some(&bearer)).await,
            expected,
            "{path} signed"
        );
        assert_eq!(get(&app, path, None).await, expected, "{path} anonymous");
    }
}

// FAILS IF: the internal function answers an unmatched path anything but the refusal, a bearer or
// not: it serves no bearer route, so its fallback has nothing to look up.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_internal_app_refuses_every_unmatched(pool: PgPool) {
    let public = common::setup_test_app(pool.clone()).await;
    let expected = u401(&public).await;
    let bearer = person(&pool, true).await;

    let state = common::test_state_with_config(pool.clone(), |_| {}).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, temper_api::create_internal_app(state))
            .await
            .expect("serve");
    });
    let internal = common::TestApp {
        addr,
        pool,
        client: reqwest::Client::new(),
    };
    for path in [
        "/api/no-such",
        "/no-such",
        "/internal/no-such",
        "/api/profile",
    ] {
        assert_eq!(
            get(&internal, path, None).await,
            expected,
            "{path} anonymous"
        );
        assert_eq!(
            get(&internal, path, Some(&bearer)).await,
            expected,
            "{path} signed"
        );
    }
}

// Controls: an admitted caller keeps every answer it had.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admitted_callers_keep_their_answers(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let approved = person(&pool, true).await;
    let (status, _, body) = get(&app, "/api/no-such", Some(&approved)).await;
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("No route matches GET /api/no-such"), "{body}");

    let email = format!("fallback-deactivated-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'deactivated') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
    )
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("deactivate");
    let deactivated = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let (status, _, body) = get(&app, "/api/no-such", Some(&deactivated)).await;
    assert_eq!(status, 401, "{body}");
    assert!(body.contains("account is deactivated"), "{body}");

    let unapproved = person(&pool, false).await;
    let (status, _, body) = get(&app, "/api/resources", Some(&unapproved)).await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("SYSTEM_ACCESS_REQUIRED"), "{body}");
    let (status, _, body) = get(&app, "/api/no-such", Some(&unapproved)).await;
    assert_eq!(
        status, 404,
        "an unapproved but signed caller still gets the 404: {body}"
    );
}
