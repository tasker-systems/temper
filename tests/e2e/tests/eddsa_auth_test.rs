#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;

/// An EdDSA-signed token authenticates through `require_auth` and resolves a
/// profile end-to-end, proving the algorithm-aware verification path added in
/// Task 0.1 (`JwksKeyStore::with_static_key(key, algorithm)`,
/// `get_decoding_key`, `validation(issuer, audience, algorithm)`) against a
/// real (non-RS256) signature — the existing e2e harness is RS256-only.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn eddsa_token_authenticates_and_resolves_profile(pool: sqlx::PgPool) {
    let app = common::setup_eddsa(pool).await;
    let token = common::generate_test_jwt_eddsa("eddsa-user", "eddsa@test.example");

    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "EdDSA-authenticated /api/profile must succeed"
    );
}
