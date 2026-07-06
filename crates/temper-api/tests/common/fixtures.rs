//! Test data setup helpers, ported to the substrate schema (WS6 collapse).
//!
//! `#[sqlx::test]` gives every test an isolated database with `migrations/`
//! already applied — including the canonical system seed (the `handle='system'`
//! actor, `kb_system_settings(access_mode='open')`, the event-type registry,
//! and the global lenses). So there is no shared state to scrub: `clean_and_seed`
//! is a no-op kept only so existing call sites compile.
//!
//! `create_test_profile_with_context` builds a fully-provisioned substrate
//! profile: the `kb_profiles` row (whose AFTER-INSERT trigger mints the
//! `personal-<handle>` team + membership), the `kb_profile_auth_links` row the
//! JWT auth path resolves (`auth_provider='test-provider'`,
//! `auth_provider_user_id='test|<id>'`), the per-surface emitter entities the
//! write path's `resolve_emitter` requires (`pete@web|cli|mcp`), and a
//! profile-owned `temper` context (ownership alone confers read/modify via
//! `resources_visible_to` / `can_modify_resource`).

use sqlx::PgPool;

/// No-op: `#[sqlx::test]` already provisions an isolated, seeded database.
///
/// Retained so existing call sites (`setup_test_app`, direct callers) keep
/// compiling without edits. The legacy body scrubbed a shared DB and seeded a
/// fixed-UUID System resource against tables/columns the substrate retired.
pub async fn clean_and_seed(_pool: &PgPool) {}

/// Grant a profile explicit `can_write` on a cognitive map — the post-Q-A authoring capability
/// (`cogmap_authorable_by_profile` = an explicit `kb_access_grants` write row; membership confers
/// read, not write). Used where a test principal must AUTHOR a map it can otherwise only read
/// (e.g. opening a self-attributed invocation on L0, which read-only root-join does not permit).
/// `granted_by` is the grantee itself (a fixture bootstrap standing in for a real delegated grant).
pub async fn grant_cogmap_write(pool: &PgPool, cogmap: uuid::Uuid, profile: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant cogmap write");
}

/// Create a test profile and return its UUID.
pub async fn create_test_profile(pool: &PgPool, email: &str) -> uuid::Uuid {
    let (profile_id, _context_id) = create_test_profile_with_context(pool, email).await;
    profile_id
}

/// Create a fully-provisioned substrate profile plus a profile-owned `temper`
/// context. Returns `(profile_id, context_id)`.
///
/// The profile is reachable by a test JWT whose `sub` is `test|<profile_id>`
/// (auth provider `test-provider`). HTTP create payloads address the context by
/// UUID (`kb_context_id`), resolved through `context_service::resolve_context_ref`
/// (visibility-gated). The owner sees and can modify its own resources via the
/// ownership branch of `resources_visible_to` / `can_modify_resource`, so no
/// team-share is needed.
pub async fn create_test_profile_with_context(
    pool: &PgPool,
    email: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let sub = format!("test|{profile_id}");
    let local = email.split('@').next().unwrap_or("test-user");
    // handle is UNIQUE in the substrate; suffix with the id head to avoid
    // cross-test collisions. The personal-team trigger keys off it.
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);

    // 1. The profile. The AFTER-INSERT trigger `trg_sync_personal_team` mints
    //    the `personal-<handle>` team and adds this profile as its owner.
    sqlx::query(
        r#"INSERT INTO kb_profiles (id, handle, display_name, email)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(profile_id)
    .bind(&handle)
    .bind(email)
    .bind(email)
    .execute(pool)
    .await
    .expect("create test profile");

    // 2. The auth link the JWT path resolves (provider + external user id).
    sqlx::query(
        r#"INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default)
           VALUES ($1, $2, 'test-provider', $3, $4, true)"#,
    )
    .bind(uuid::Uuid::now_v7())
    .bind(profile_id)
    .bind(&sub)
    .bind(email)
    .execute(pool)
    .await
    .expect("create test auth link");

    // 3. The per-surface emitter entities the write path requires. `resolve_emitter`
    //    looks up `<handle>@<surface>` (web=ApiHttp, cli=CliCloud, mcp=Mcp), mirroring
    //    production provisioning (profile_service::resolve_from_claims), for the
    //    surfaces a test may drive.
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            r#"INSERT INTO kb_entities (profile_id, name, metadata)
               VALUES ($1, $2, '{}'::jsonb)"#,
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("create emitter entity");
    }

    // 4. A profile-owned `temper` context (owner_table/owner_id + per-owner slug).
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
           VALUES ($1, 'kb_profiles', $2, 'temper', 'temper')"#,
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("create test profile-owned temper context");

    (profile_id, context_id)
}
