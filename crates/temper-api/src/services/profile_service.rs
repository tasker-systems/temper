use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, Profile, ProfileAuthLink};

use temper_services::error::{ApiError, ApiResult};

/// Maximum serialized size for the preferences JSON field (64KB).
const MAX_PREFERENCES_BYTES: usize = 65_536;

/// Validate that preferences JSON does not exceed the size limit.
pub fn validate_preferences_size(preferences: Option<&Value>) -> ApiResult<()> {
    if let Some(prefs) = preferences {
        let size = serde_json::to_string(prefs).map(|s| s.len()).unwrap_or(0);
        if size > MAX_PREFERENCES_BYTES {
            return Err(ApiError::BadRequest(format!(
                "preferences exceeds maximum size of {MAX_PREFERENCES_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

/// Generate a unique profile handle from a display name.
///
/// Slugifies the name (lowercase, non-alnum → dash, trim dashes), then appends
/// -2, -3, etc. if the handle already exists. `handle` is the substrate
/// addressing key (`slug` is §7-dissolved).
pub async fn generate_profile_handle(pool: &PgPool, display_name: &str) -> ApiResult<String> {
    let base: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive dashes (matches SQL backfill regex [^a-zA-Z0-9]+)
    let base: String = base
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };

    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE handle = $1) as \"exists!: bool\"",
        &base,
    )
    .fetch_one(pool)
    .await?;

    if !exists {
        return Ok(base);
    }

    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        let exists = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE handle = $1) as \"exists!: bool\"",
            &candidate,
        )
        .fetch_one(pool)
        .await?;

        if !exists {
            return Ok(candidate);
        }
        suffix += 1;
    }
}

/// Resolve a profile from JWT claims.
///
/// Lookup order:
/// 1. Find an existing `kb_profile_auth_links` row by `(auth_provider, auth_provider_user_id)`.
/// 2. If found, load the linked profile.
/// 3. If not found, check whether any auth link shares the same email (reconciliation).
/// 4. If email matches an existing link, create a new auth link pointing to that profile.
/// 5. Otherwise, create a new profile and a new auth link.
pub async fn resolve_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    // 1 & 2: direct lookup by provider + external user id; load linked profile.
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        return get_by_id(pool, ProfileId::from(link.profile_id)).await;
    }

    // 3 & 4: email reconciliation — attach this provider to an existing profile.
    if let Some(profile) = reconcile_by_email(pool, claims).await? {
        return Ok(profile);
    }

    // 5: brand new profile + auth link, then provision its emitter entities and
    // default context.
    let (profile_id, handle) = create_new_profile_and_link(pool, claims).await?;
    provision_profile_entities(pool, profile_id, &handle).await?;

    get_by_id(pool, ProfileId::from(profile_id)).await
}

/// Phase 1: direct lookup of an auth link by `(auth_provider, auth_provider_user_id)`.
async fn lookup_link_by_provider(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<Option<ProfileAuthLink>> {
    let link = sqlx::query_as!(
        ProfileAuthLink,
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
          FROM kb_profile_auth_links
         WHERE auth_provider = $1
           AND auth_provider_user_id = $2
        "#,
        &claims.provider,
        &claims.external_user_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(link)
}

/// Phase 3 & 4: email reconciliation. Only verified emails reconcile; when an
/// existing link shares the email, a new auth link for this provider is created
/// pointing at that profile and the profile is returned. Returns `None` when the
/// email is unverified or no existing link matches (caller falls through to
/// new-profile creation).
async fn reconcile_by_email(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Option<Profile>> {
    if claims.email_verified != Some(true) {
        tracing::warn!(
            provider = %claims.provider,
            external_user_id = %claims.external_user_id,
            "Skipping email reconciliation: email_verified is not true"
        );
        return Ok(None);
    }

    let reconciled_link = sqlx::query_as!(
        ProfileAuthLink,
        r#"
            SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
              FROM kb_profile_auth_links
             WHERE email = $1
             LIMIT 1
            "#,
        &claims.email,
    )
    .fetch_optional(pool)
    .await?;

    let Some(existing) = reconciled_link else {
        return Ok(None);
    };

    create_link_for_existing_profile(pool, existing.profile_id, claims).await?;

    Ok(Some(
        get_by_id(pool, ProfileId::from(existing.profile_id)).await?,
    ))
}

/// Phase 4: create a new (non-default) auth link for this provider pointing at
/// an existing profile.
async fn create_link_for_existing_profile(
    pool: &PgPool,
    profile_id: Uuid,
    claims: &AuthClaims,
) -> ApiResult<()> {
    let new_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
                INSERT INTO kb_profile_auth_links
                    (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
                VALUES ($1, $2, $3, $4, $5, false, now())
                "#,
        new_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
        &claims.email as &str,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Phase 5a: create a brand-new profile and its default auth link. Returns the
/// new profile id and the generated handle (the latter feeds emitter provisioning).
async fn create_new_profile_and_link(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<(Uuid, String)> {
    let display_name = claims.email.split('@').next().unwrap_or("user").to_string();
    let handle = generate_profile_handle(pool, &display_name).await?;

    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles
            (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, $4, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
        &claims.email as &str,
    )
    .execute(pool)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
        VALUES ($1, $2, $3, $4, $5, true, now())
        "#,
        auth_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
        &claims.email as &str,
    )
    .execute(pool)
    .await?;

    Ok((profile_id, handle))
}

/// Phase 5b: provision the per-surface emitter entities and the default context
/// a freshly created profile needs.
async fn provision_profile_entities(
    pool: &PgPool,
    profile_id: Uuid,
    handle: &str,
) -> ApiResult<()> {
    // Provision the per-surface emitter entities the write path resolves
    // (`<handle>@<surface>` — `writes::resolve_emitter`). The deleted synthesis
    // bootstrap used to create these; without them an auto-provisioned profile
    // could not emit events (every write would 500 on a missing emitter).
    for surface in ["web", "cli", "mcp"] {
        sqlx::query!(
            r#"
            INSERT INTO kb_entities (profile_id, name, metadata)
            VALUES ($1, $2, '{}'::jsonb)
            "#,
            profile_id,
            format!("{handle}@{surface}"),
        )
        .execute(pool)
        .await?;
    }

    // Auto-provision a "default" context for the new profile.
    // Ignore conflict — if the profile somehow already has one, that's fine.
    sqlx::query!(
        r#"
        INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
        VALUES ($1, 'kb_profiles', $2, 'default', 'default')
        ON CONFLICT (owner_table, owner_id, slug) DO NOTHING
        "#,
        Uuid::now_v7(),
        profile_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Load a profile by ID.
///
/// The substrate `kb_profiles` has no `is_active`, so there is no soft-delete
/// predicate (visibility lives elsewhere).
pub async fn get_by_id(pool: &PgPool, id: ProfileId) -> ApiResult<Profile> {
    let profile = sqlx::query_as!(
        Profile,
        r#"
        SELECT id,
               display_name,
               handle AS "slug!",
               email,
               NULL::text AS avatar_url,
               preferences as "preferences: serde_json::Value",
               '{}'::jsonb AS "vault_config!: serde_json::Value",
               true AS "is_active!",
               created,
               created AS "updated!"
          FROM kb_profiles
         WHERE id = $1
        "#,
        *id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(profile)
}

/// Update mutable profile fields. Only provided (`Some`) values are written.
///
/// `vault_config` is intentionally not a parameter: it is substrate-dropped
/// (synthesized on read), so there is nothing to persist.
pub async fn update(
    pool: &PgPool,
    id: ProfileId,
    display_name: Option<&str>,
    preferences: Option<&Value>,
) -> ApiResult<Profile> {
    let current = get_by_id(pool, id).await?;

    let new_display_name = display_name.unwrap_or(&current.display_name);
    let new_preferences = preferences.unwrap_or(&current.preferences);

    sqlx::query!(
        r#"
        UPDATE kb_profiles
           SET display_name = $1,
               preferences  = $2
         WHERE id = $3
        "#,
        new_display_name,
        new_preferences as &Value,
        *id,
    )
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// List all auth links attached to a profile.
pub async fn list_auth_links(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Vec<ProfileAuthLink>> {
    let links = sqlx::query_as!(
        ProfileAuthLink,
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
          FROM kb_profile_auth_links
         WHERE profile_id = $1
         ORDER BY linked_at ASC
        "#,
        *profile_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(links)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[test]
    fn oversized_preferences_rejected() {
        let large_value: serde_json::Value = serde_json::Value::String("x".repeat(65_537));
        let result = validate_preferences_size(Some(&large_value));
        assert!(result.is_err());
    }

    #[test]
    fn normal_preferences_accepted() {
        let small_value: serde_json::Value = serde_json::json!({"theme": "dark"});
        let result = validate_preferences_size(Some(&small_value));
        assert!(result.is_ok());
    }

    #[test]
    fn none_preferences_accepted() {
        let result = validate_preferences_size(None);
        assert!(result.is_ok());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_handle_from_display_name(pool: PgPool) {
        let handle = generate_profile_handle(&pool, "Pete Taylor").await.unwrap();
        assert_eq!(handle, "pete-taylor");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_handle_handles_special_chars(pool: PgPool) {
        let handle = generate_profile_handle(&pool, "José García-López")
            .await
            .unwrap();
        assert_eq!(handle, "jos-garc-a-l-pez");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_handle_handles_collision(pool: PgPool) {
        // Create a profile that will own the "collider" handle
        let claims = AuthClaims {
            provider: "test".to_string(),
            external_user_id: "slug-collision-1".to_string(),
            email: "collider@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile = resolve_from_claims(&pool, &claims).await.unwrap();
        assert_eq!(profile.slug, "collider");

        // Now generate a handle for the same display name — should get -2
        let handle = generate_profile_handle(&pool, "collider").await.unwrap();
        assert_eq!(handle, "collider-2");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn verified_email_reconciles_to_existing_profile(pool: PgPool) {
        let claims_a = AuthClaims {
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-verified-a".to_string(),
            email: "recon-verified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            provider: "provider_b".to_string(),
            external_user_id: "user-recon-verified-b".to_string(),
            email: "recon-verified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_eq!(
            profile_a.id, profile_b.id,
            "verified email should reconcile to same profile"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn unverified_email_creates_separate_profile(pool: PgPool) {
        let claims_a = AuthClaims {
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-unverified-a".to_string(),
            email: "recon-unverified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            provider: "provider_b".to_string(),
            external_user_id: "user-recon-unverified-b".to_string(),
            email: "recon-unverified@example.com".to_string(),
            email_verified: Some(false),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_ne!(
            profile_a.id, profile_b.id,
            "unverified email should create separate profile"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn missing_email_verified_creates_separate_profile(pool: PgPool) {
        let claims_a = AuthClaims {
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-none-a".to_string(),
            email: "recon-none@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            provider: "provider_b".to_string(),
            external_user_id: "user-recon-none-b".to_string(),
            email: "recon-none@example.com".to_string(),
            email_verified: None,
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_ne!(
            profile_a.id, profile_b.id,
            "None email_verified should create separate profile"
        );
    }
}
