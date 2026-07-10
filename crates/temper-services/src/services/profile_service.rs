use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, PrincipalKind, Profile, ProfileAuthLink};
use temper_workflow::operations::Surface;

use crate::error::{ApiError, ApiResult};

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
    match claims.principal_kind {
        PrincipalKind::Human => resolve_human_from_claims(pool, claims).await,
        PrincipalKind::Machine => resolve_machine_from_claims(pool, claims).await,
    }
}

/// Human path: link lookup → email reconcile → new profile.
async fn resolve_human_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    // 1 & 2: direct lookup by provider + external user id; load linked profile.
    // A verified sign-in refreshes the link's stored email + verification flag
    // (the self-heal path for rows that predate the email_verified column, and
    // for accounts verified at the provider after first sign-in).
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        refresh_link_verification(pool, &link, claims).await?;
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

/// Machine path: link lookup under the `auth0-m2m` namespace → on first sight,
/// provision a dedicated agent profile. NEVER enters email reconciliation — a
/// machine has no verified email.
async fn resolve_machine_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        return get_by_id(pool, ProfileId::from(link.profile_id)).await;
    }

    let (profile_id, handle) = create_agent_profile_and_link(pool, claims).await?;
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
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, email_verified,
               is_default, linked_at
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

/// Refresh a link's stored email + verification on a verified sign-in: when the
/// incoming claims carry `email_verified: true` and the stored row disagrees
/// (unverified, or a different email), persist the provider's current truth.
/// No-op for unverified/missing claims — the flag never flips false→true here
/// without the provider's say-so, and never true→false at all (a later
/// unverified token doesn't un-verify an email the provider once verified).
async fn refresh_link_verification(
    pool: &PgPool,
    link: &ProfileAuthLink,
    claims: &AuthClaims,
) -> ApiResult<()> {
    if claims.email_verified != Some(true) {
        return Ok(());
    }
    let email_changed = link.email.as_deref() != Some(claims.email.as_str());
    if link.email_verified && !email_changed {
        return Ok(());
    }
    sqlx::query!(
        "UPDATE kb_profile_auth_links SET email = $2, email_verified = true WHERE id = $1",
        link.id,
        &claims.email as &str,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Phase 3 & 4: email reconciliation. Requires verification on BOTH sides: the
/// incoming claims must carry `email_verified: true`, and the matched stored link
/// must itself be verified — an unverified stored email is an attacker-controllable
/// claim (a pre-created account holding someone else's address must not capture
/// that person's first verified sign-in). When a verified existing link shares the
/// email, a new auth link for this provider is created pointing at that profile
/// and the profile is returned. Returns `None` otherwise (caller falls through to
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
            SELECT id, profile_id, auth_provider, auth_provider_user_id, email, email_verified,
                   is_default, linked_at
              FROM kb_profile_auth_links
             WHERE email = $1
               AND email_verified
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
                    (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
                VALUES ($1, $2, $3, $4, $5, $6, false, now())
                "#,
        new_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
        &claims.email as &str,
        claims.email_verified == Some(true),
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
            (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
        VALUES ($1, $2, $3, $4, $5, $6, true, now())
        "#,
        auth_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
        &claims.email as &str,
        claims.email_verified == Some(true),
    )
    .execute(pool)
    .await?;

    Ok((profile_id, handle))
}

/// Create a brand-new agent profile and its default machine auth link. Email is
/// SQL NULL (a machine has none); display name / handle derive from the client id.
async fn create_agent_profile_and_link(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<(Uuid, String)> {
    let display_name = format!("agent-{}", claims.external_user_id);
    let handle = generate_profile_handle(pool, &display_name).await?;
    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, NULL, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
    )
    .execute(pool)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, email_verified, is_default, linked_at)
        VALUES ($1, $2, $3, $4, NULL, false, true, now())
        "#,
        auth_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
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
    //
    // Driven off `Surface::ALL` so a new surface variant provisions its emitter here by
    // construction. Existing profiles still need an additive backfill migration — see
    // `20260709000030_backfill_sdk_emitter_entities.sql` for the shape.
    //
    // Each emitter is still its own auto-commit statement, so two concurrent first-authenticated
    // requests for the same new profile both run this loop. `ON CONFLICT DO NOTHING` — inferring
    // the unique index added by `20260709000040_kb_entities_unique_profile_name.sql` — is what
    // makes that a no-op rather than a duplicate. It also makes provisioning re-runnable, so a
    // profile left half-provisioned by a failed request is repaired by calling this again.
    for surface in Surface::ALL {
        sqlx::query!(
            r#"
            INSERT INTO kb_entities (profile_id, name, metadata)
            VALUES ($1, $2, '{}'::jsonb)
            ON CONFLICT (profile_id, name) DO NOTHING
            "#,
            profile_id,
            format!("{handle}@{}", surface.marker()),
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
/// `is_active` is a real deactivation flag read from the `kb_profiles` column —
/// it is the authn lever for soft-deleted/deactivated accounts. `require_auth`
/// rejects the request when it comes back `false`; every `resolve_from_claims`
/// path routes through here, so the flag surfaces everywhere.
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
               is_active,
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
        SELECT id, profile_id, auth_provider, auth_provider_user_id, email, email_verified,
               is_default, linked_at
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
            principal_kind: PrincipalKind::Human,
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
            principal_kind: PrincipalKind::Human,
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-verified-a".to_string(),
            email: "recon-verified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            principal_kind: PrincipalKind::Human,
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
            principal_kind: PrincipalKind::Human,
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-unverified-a".to_string(),
            email: "recon-unverified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            principal_kind: PrincipalKind::Human,
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
            principal_kind: PrincipalKind::Human,
            provider: "provider_a".to_string(),
            external_user_id: "user-recon-none-a".to_string(),
            email: "recon-none@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            principal_kind: PrincipalKind::Human,
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn unverified_stored_link_does_not_capture_verified_signin(pool: PgPool) {
        // Profile A signed up with an UNVERIFIED email — its stored link is
        // unverified. A later VERIFIED sign-in with the same email (e.g. the
        // address's real owner) must NOT reconcile onto A's profile: an
        // unverified stored email is an attacker-controllable claim.
        let claims_a = AuthClaims {
            principal_kind: PrincipalKind::Human,
            provider: "provider_a".to_string(),
            external_user_id: "user-stored-unverified-a".to_string(),
            email: "stored-unverified@example.com".to_string(),
            email_verified: Some(false),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_a = resolve_from_claims(&pool, &claims_a).await.unwrap();

        let claims_b = AuthClaims {
            principal_kind: PrincipalKind::Human,
            provider: "provider_b".to_string(),
            external_user_id: "user-stored-unverified-b".to_string(),
            email: "stored-unverified@example.com".to_string(),
            email_verified: Some(true),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile_b = resolve_from_claims(&pool, &claims_b).await.unwrap();

        assert_ne!(
            profile_a.id, profile_b.id,
            "a verified sign-in must not attach to a profile whose stored email is unverified"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn verified_signin_refreshes_stored_link(pool: PgPool) {
        // First sign-in unverified → link stored unverified. Second sign-in on
        // the SAME provider identity, now verified → the stored flag self-heals.
        let mut claims = AuthClaims {
            principal_kind: PrincipalKind::Human,
            provider: "provider_a".to_string(),
            external_user_id: "user-refresh".to_string(),
            email: "refresh@example.com".to_string(),
            email_verified: Some(false),
            exp: 9_999_999_999,
            iat: 1_000_000_000,
        };
        let profile = resolve_from_claims(&pool, &claims).await.unwrap();

        let stored: bool = sqlx::query_scalar(
            "SELECT email_verified FROM kb_profile_auth_links \
             WHERE auth_provider = 'provider_a' AND auth_provider_user_id = 'user-refresh'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!stored, "first (unverified) sign-in stores unverified");

        claims.email_verified = Some(true);
        let same = resolve_from_claims(&pool, &claims).await.unwrap();
        assert_eq!(profile.id, same.id);

        let stored: bool = sqlx::query_scalar(
            "SELECT email_verified FROM kb_profile_auth_links \
             WHERE auth_provider = 'provider_a' AND auth_provider_user_id = 'user-refresh'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(stored, "verified sign-in refreshes the stored flag");
    }

    fn machine_claims(client_id: &str) -> AuthClaims {
        AuthClaims {
            principal_kind: PrincipalKind::Machine,
            provider: crate::auth::MACHINE_PROVIDER_TAG.to_string(),
            external_user_id: client_id.to_string(),
            email: String::new(),
            email_verified: None,
            exp: 0,
            iat: 0,
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn machine_first_sight_provisions_agent_profile(pool: PgPool) {
        let c = machine_claims("agent-client-xyz");
        let p = resolve_from_claims(&pool, &c)
            .await
            .expect("provision agent");

        // Link created under the machine namespace with NULL email.
        let link = sqlx::query!(
            "SELECT auth_provider, email FROM kb_profile_auth_links \
             WHERE auth_provider = $1 AND auth_provider_user_id = $2",
            "auth0-m2m",
            "agent-client-xyz",
        )
        .fetch_one(&pool)
        .await
        .expect("link row");
        assert_eq!(link.auth_provider, "auth0-m2m");
        assert!(link.email.is_none(), "machine link email must be NULL");
        assert!(p.is_active);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn machine_resolution_is_idempotent(pool: PgPool) {
        let c = machine_claims("agent-idem");
        let a = resolve_from_claims(&pool, &c).await.expect("first");
        let b = resolve_from_claims(&pool, &c).await.expect("second");
        assert_eq!(a.id, b.id, "same agent profile on second sight");

        let n = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
            "agent-idem",
        )
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(n, Some(1), "exactly one link, no duplicate");
    }

    async fn seed_bare_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .expect("seed profile")
    }

    async fn emitter_count(pool: &PgPool, profile_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM kb_entities WHERE profile_id = $1")
            .bind(profile_id)
            .fetch_one(pool)
            .await
            .expect("count emitters")
    }

    /// Two first-authenticated requests for the same new profile can run concurrently. Each
    /// emitter is its own auto-commit statement, so before the unique constraint on
    /// `(profile_id, name)` both writers observed no row and both inserted — silently splitting
    /// one logical emitter across two `entity_id`s.
    #[sqlx::test(migrations = "../../migrations")]
    async fn provision_profile_entities_is_safe_under_concurrent_invocation(pool: PgPool) {
        let handle = "concurrent-provision";
        let profile_id = seed_bare_profile(&pool, handle).await;

        let (a, b) = tokio::join!(
            provision_profile_entities(&pool, profile_id, handle),
            provision_profile_entities(&pool, profile_id, handle),
        );
        a.expect("first concurrent provision");
        b.expect("second concurrent provision");

        assert_eq!(
            emitter_count(&pool, profile_id).await,
            Surface::ALL.len() as i64,
            "concurrent provisioning must yield exactly one emitter per surface",
        );
    }

    /// The sequential case the constraint also unlocks: provisioning is now safe to re-run, so a
    /// profile left half-provisioned by a failed request can be repaired by calling it again.
    #[sqlx::test(migrations = "../../migrations")]
    async fn provision_profile_entities_is_idempotent(pool: PgPool) {
        let handle = "repeat-provision";
        let profile_id = seed_bare_profile(&pool, handle).await;

        provision_profile_entities(&pool, profile_id, handle)
            .await
            .expect("first provision");
        provision_profile_entities(&pool, profile_id, handle)
            .await
            .expect("second provision");

        assert_eq!(
            emitter_count(&pool, profile_id).await,
            Surface::ALL.len() as i64,
        );
    }
}
