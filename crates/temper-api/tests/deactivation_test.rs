#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;

/// A deactivated profile (principal standing `'deactivated'`) must be rejected on
/// every subsequent authenticated request, even with an otherwise-valid JWT.
///
/// This is the general-purpose account-deactivation gate — a sibling to (not
/// part of) the SAML reconcile flow. It applies regardless of which auth
/// provider resolved the claims.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_deactivated_profile_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = "deact-user";
    let email = "deact-user@example.com";
    let token = common::generate_test_jwt(sub, email);

    // First request auto-provisions the profile and must succeed.
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "active profile must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // Deactivate the profile directly in the database — a `deactivated` principal standing (Phase 2
    // dropped `is_active`; the Level-1 gate reads standing). Runtime query (not the query! macro) —
    // a trivial test-fixture write needs no .sqlx cache entry, so it compiles under SQLX_OFFLINE.
    sqlx::query(
        r#"
        INSERT INTO kb_principal_standing (profile_id, state)
        SELECT profile_id, 'deactivated'
          FROM kb_profile_auth_links
         WHERE auth_provider_user_id = $1
        ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'
        "#,
    )
    .bind(sub)
    .execute(&app.pool)
    .await
    .expect("failed to deactivate profile");

    // The same (still otherwise-valid) token must now be rejected.
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("second request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "deactivated profile must return 401; body: {}",
        resp.text().await.unwrap_or_default()
    );
}
