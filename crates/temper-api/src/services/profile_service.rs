use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::{AuthClaims, Profile, ProfileAuthLink};

use crate::error::{ApiError, ApiResult};

/// Resolve a profile from JWT claims.
///
/// Lookup order:
/// 1. Find an existing `kb_profile_auth_links` row by `(auth_provider, auth_provider_user_id)`.
/// 2. If found, load the linked profile.
/// 3. If not found, check whether any auth link shares the same email (reconciliation).
/// 4. If email matches an existing link, create a new auth link pointing to that profile.
/// 5. Otherwise, create a new profile and a new auth link.
pub async fn resolve_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    // 1 & 2: direct lookup by provider + external user id
    let existing_link = sqlx::query_as::<_, ProfileAuthLink>(
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
          FROM kb_profile_auth_links
         WHERE auth_provider = $1
           AND auth_provider_user_id = $2
        "#,
    )
    .bind(&claims.provider)
    .bind(&claims.external_user_id)
    .fetch_optional(pool)
    .await?;

    if let Some(link) = existing_link {
        return get_by_id(pool, link.profile_id).await;
    }

    // 3: email reconciliation — find any existing link with the same email
    let reconciled_link = sqlx::query_as::<_, ProfileAuthLink>(
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
          FROM kb_profile_auth_links
         WHERE email = $1
         LIMIT 1
        "#,
    )
    .bind(&claims.email)
    .fetch_optional(pool)
    .await?;

    if let Some(existing) = reconciled_link {
        // 4: create new auth link for this provider pointing to the existing profile
        let new_link_id = Uuid::now_v7();
        sqlx::query(
            r#"
            INSERT INTO kb_profile_auth_links
                (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
            VALUES ($1, $2, $3, $4, $5, false, now())
            "#,
        )
        .bind(new_link_id)
        .bind(existing.profile_id)
        .bind(&claims.provider)
        .bind(&claims.external_user_id)
        .bind(&claims.email)
        .execute(pool)
        .await?;

        return get_by_id(pool, existing.profile_id).await;
    }

    // 5: brand new profile + auth link
    let display_name = claims.email.split('@').next().unwrap_or("user").to_string();

    let profile_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO kb_profiles
            (id, display_name, email, avatar_url, preferences, vault_config, is_active, created, updated)
        VALUES ($1, $2, $3, null, '{}', '{}', true, now(), now())
        "#,
    )
    .bind(profile_id)
    .bind(&display_name)
    .bind(&claims.email)
    .execute(pool)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
        VALUES ($1, $2, $3, $4, $5, true, now())
        "#,
    )
    .bind(auth_link_id)
    .bind(profile_id)
    .bind(&claims.provider)
    .bind(&claims.external_user_id)
    .bind(&claims.email)
    .execute(pool)
    .await?;

    get_by_id(pool, profile_id).await
}

/// Load a profile by ID. Returns `NotFound` if missing or inactive.
pub async fn get_by_id(pool: &PgPool, id: Uuid) -> ApiResult<Profile> {
    let profile = sqlx::query_as::<_, Profile>(
        r#"
        SELECT id, display_name, email, avatar_url, preferences, vault_config, is_active, created, updated
          FROM kb_profiles
         WHERE id = $1
           AND is_active = true
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(profile)
}

/// Update mutable profile fields. Only provided (Some) values are written.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    display_name: Option<&str>,
    preferences: Option<&Value>,
    vault_config: Option<&Value>,
) -> ApiResult<Profile> {
    let current = get_by_id(pool, id).await?;

    let new_display_name = display_name.unwrap_or(&current.display_name);
    let new_preferences = preferences.unwrap_or(&current.preferences);
    let new_vault_config = vault_config.unwrap_or(&current.vault_config);

    sqlx::query(
        r#"
        UPDATE kb_profiles
           SET display_name = $1,
               preferences  = $2,
               vault_config = $3,
               updated      = now()
         WHERE id = $4
           AND is_active = true
        "#,
    )
    .bind(new_display_name)
    .bind(new_preferences)
    .bind(new_vault_config)
    .bind(id)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// List all auth links attached to a profile.
pub async fn list_auth_links(pool: &PgPool, profile_id: Uuid) -> ApiResult<Vec<ProfileAuthLink>> {
    let links = sqlx::query_as::<_, ProfileAuthLink>(
        r#"
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
          FROM kb_profile_auth_links
         WHERE profile_id = $1
         ORDER BY linked_at ASC
        "#,
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;

    Ok(links)
}
