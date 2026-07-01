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

/// M3 cross-stack proof: a SAML-shaped AS token (persistent NameID as `sub`, verified
/// email) minted via the EdDSA path drives just-in-time profile provisioning in
/// `temper-api`. `/api/profile` succeeds AND `resolve_from_claims` creates a `kb_profiles`
/// row + a `kb_profile_auth_links` row, the link namespaced under the instance's
/// `auth_provider_name` (`saml:test-idp`) — the SAML → AS → temper-api identity contract.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn saml_shaped_token_drives_profile_jit(pool: sqlx::PgPool) {
    let app = common::setup_eddsa_with_provider(pool.clone(), "saml:test-idp").await;
    let token = common::generate_test_jwt_eddsa("saml-persistent-id", "a@corp.io");

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
        "SAML-shaped AS token must resolve a profile"
    );

    // The auth link is JIT-created and namespaced under the instance's provider name.
    let auth_provider = sqlx::query_scalar::<_, String>(
        "SELECT auth_provider FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind("saml-persistent-id")
    .fetch_one(&app.pool)
    .await
    .expect("a kb_profile_auth_links row must be JIT-created for the SAML identity");
    assert_eq!(
        auth_provider, "saml:test-idp",
        "the auth link must be namespaced by the instance's auth_provider_name"
    );

    let email = sqlx::query_scalar::<_, Option<String>>(
        "SELECT email FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
    )
    .bind("saml-persistent-id")
    .fetch_one(&app.pool)
    .await
    .expect("query failed");
    assert_eq!(email.as_deref(), Some("a@corp.io"));

    // The linked kb_profiles row exists (JIT-provisioned).
    let profile_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM kb_profiles p \
         JOIN kb_profile_auth_links l ON l.profile_id = p.id \
         WHERE l.auth_provider_user_id = $1 AND l.auth_provider = $2",
    )
    .bind("saml-persistent-id")
    .bind("saml:test-idp")
    .fetch_one(&app.pool)
    .await
    .expect("query failed");
    assert_eq!(
        profile_count, 1,
        "a kb_profiles row linked to the SAML identity must be JIT-created"
    );
}
