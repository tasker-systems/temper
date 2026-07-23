#![cfg(feature = "test-db")]

mod common;

use temper_core::types::api::ProfileUpdateRequest;

/// First call to profile().get() auto-creates a profile.
/// Verify display_name is derived from the JWT email prefix ("e2e").
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn profile_auto_provision(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile get (auto-provision) failed");

    // Email is "e2e@test.example.com" so display_name should be the prefix "e2e"
    assert_eq!(
        profile.display_name, "e2e",
        "auto-provisioned display_name should be the email prefix"
    );
    // `is_active` was dropped from Profile in principal-admission Phase 2; deactivation is a
    // standing state now, gated at Level 1 (see deactivation_test / auth_seam_parity_e2e).
}

/// Update display_name, then get again and verify the change persisted.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn profile_update_display_name(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile exists first.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let updated = app
        .client
        .profile()
        .update(&ProfileUpdateRequest {
            display_name: Some("E2E Test User".to_string()),
            preferences: None,
            vault_config: None,
        })
        .await
        .expect("profile update failed");

    assert_eq!(updated.display_name, "E2E Test User");

    let fetched = app
        .client
        .profile()
        .get()
        .await
        .expect("profile get after update failed");

    assert_eq!(
        fetched.display_name, "E2E Test User",
        "updated display_name should persist"
    );
}
