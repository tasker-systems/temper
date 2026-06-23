#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! Dark-launch coverage for the substrate (`temper_next`) profile path.
//!
//! These exercise the `#[cfg(feature = "next-backend")]` `*_next` fns in
//! `profile_service`, which query qualified `temper_next.*` and return the
//! existing `temper_core::Profile` shape with the soon-to-be-dropped fields
//! synthesized (`slug = handle`, `avatar_url = None`, `vault_config = {}`,
//! `is_active = true`, `updated = created`). The legacy path is untouched; the
//! flip swaps call sites later.

use serde_json::json;
use sqlx::PgPool;

use temper_api::services::profile_service;
use temper_core::types::AuthClaims;

fn test_claims(provider: &str, external_user_id: &str, email: &str) -> AuthClaims {
    AuthClaims {
        provider: provider.to_string(),
        external_user_id: external_user_id.to_string(),
        email: email.to_string(),
        email_verified: Some(true),
        exp: 9_999_999_999,
        iat: 1_000_000_000,
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolve_then_get_roundtrips_over_substrate(pool: PgPool) {
    let claims = test_claims("auth0", "abc-resolve", "ada@x.dev");

    let p = profile_service::resolve_from_claims_next(&pool, &claims)
        .await
        .expect("resolve");

    // slug is synthesized from the substrate handle.
    assert_eq!(p.slug, "ada");
    assert_eq!(p.display_name, "ada");
    assert_eq!(p.email.as_deref(), Some("ada@x.dev"));
    // Dropped fields are synthesized to their §9-non-invariant defaults.
    assert!(p.is_active);
    assert_eq!(p.avatar_url, None);
    assert_eq!(p.vault_config, json!({}));
    assert_eq!(p.updated, p.created);

    // A 'default' context is auto-provisioned for the new profile.
    let ctx_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM temper_next.kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = 'default'",
    )
    .bind(p.id)
    .fetch_one(&pool)
    .await
    .expect("count contexts");
    assert_eq!(ctx_count, 1);

    // get_by_id_next reads it back with the same synthesized shape.
    let got = profile_service::get_by_id_next(&pool, p.id)
        .await
        .expect("get");
    assert_eq!(got.id, p.id);
    assert_eq!(got.display_name, "ada");
    assert_eq!(got.slug, "ada");
    assert!(got.is_active);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolve_is_idempotent_by_auth_link(pool: PgPool) {
    let claims = test_claims("auth0", "abc-idem", "grace@x.dev");

    let first = profile_service::resolve_from_claims_next(&pool, &claims)
        .await
        .expect("first resolve");
    let second = profile_service::resolve_from_claims_next(&pool, &claims)
        .await
        .expect("second resolve");

    assert_eq!(
        first.id, second.id,
        "same auth identity resolves to the same substrate profile"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_next_writes_display_name_and_preferences(pool: PgPool) {
    let claims = test_claims("auth0", "abc-update", "linus@x.dev");
    let p = profile_service::resolve_from_claims_next(&pool, &claims)
        .await
        .expect("resolve");

    let new_prefs = json!({"theme": "dark"});
    let updated =
        profile_service::update_next(&pool, p.id, Some("Linus Torvalds"), Some(&new_prefs), None)
            .await
            .expect("update");

    assert_eq!(updated.display_name, "Linus Torvalds");
    assert_eq!(updated.preferences, new_prefs);
    // slug stays the original synthesized handle (handle is not regenerated on update).
    assert_eq!(updated.slug, "linus");
    assert!(updated.is_active);
}
