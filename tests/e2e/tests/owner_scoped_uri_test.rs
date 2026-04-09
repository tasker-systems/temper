#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn create_test_profile(pool: &PgPool, display_name: &str, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, display_name, slug, email, is_active, created, updated)
         VALUES ($1, $2, $3, $4, true, now(), now())",
    )
    .bind(id)
    .bind(display_name)
    .bind(slug)
    .bind(format!("{slug}@test.example.com"))
    .execute(pool)
    .await
    .expect("create_test_profile");
    id
}

async fn create_test_team(pool: &PgPool, slug: &str, created_by: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_teams (id, name, slug, description, is_active, created_by_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, true, $5, now(), now())",
    )
    .bind(id)
    .bind(slug) // name = slug for simplicity
    .bind(slug)
    .bind(format!("Test team {slug}"))
    .bind(created_by)
    .execute(pool)
    .await
    .expect("create_test_team");

    // Add creator as owner member
    sqlx::query(
        "INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
         VALUES ($1, $2, $3, 'owner', now())",
    )
    .bind(Uuid::now_v7())
    .bind(id)
    .bind(created_by)
    .execute(pool)
    .await
    .expect("add team owner member");

    id
}

async fn create_test_context(pool: &PgPool, name: &str, owner_table: &str, owner_id: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created, updated)
         VALUES ($1, $2, $3, $4, now(), now())",
    )
    .bind(id)
    .bind(name)
    .bind(owner_table)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("create_test_context");
    id
}

async fn create_test_resource(
    pool: &PgPool,
    context_id: Uuid,
    doc_type_name: &str,
    slug: Option<&str>,
    profile_id: Uuid,
) -> Uuid {
    let doc_type_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = $1")
        .bind(doc_type_name)
        .fetch_one(pool)
        .await
        .expect("lookup doc_type_id");

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, title, slug, origin_uri,
             originator_profile_id, owner_profile_id, is_active, created, updated)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())",
    )
    .bind(id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(slug.unwrap_or("Test Resource"))
    .bind(slug)
    .bind(format!("test://{id}")) // unique origin_uri
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("create_test_resource");
    id
}

// ---------------------------------------------------------------------------
// 1. kb_resource_uri includes profile owner (@slug)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_resource_uri_includes_profile_owner(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Alice Test", "alice-test").await;
    let ctx_id = create_test_context(&pool, "my-vault", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", Some("my-note"), profile_id).await;

    let uri: String = sqlx::query_scalar("SELECT kb_resource_uri($1)")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("kb_resource_uri query");

    assert_eq!(uri, "kb://@alice-test/my-vault/research/my-note");
}

// ---------------------------------------------------------------------------
// 2. kb_resource_uri includes team owner (+slug)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_resource_uri_includes_team_owner(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Bob Test", "bob-test").await;
    let team_id = create_test_team(&pool, "acme-team", profile_id).await;
    let ctx_id = create_test_context(&pool, "shared-vault", "kb_teams", team_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", Some("team-note"), profile_id).await;

    let uri: String = sqlx::query_scalar("SELECT kb_resource_uri($1)")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("kb_resource_uri query");

    assert_eq!(uri, "kb://+acme-team/shared-vault/research/team-note");
}

// ---------------------------------------------------------------------------
// 3. kb_resource_uri falls back to UUID when no slug
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_resource_uri_falls_back_to_uuid_when_no_slug(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Carol Test", "carol-test").await;
    let ctx_id = create_test_context(&pool, "my-vault", "kb_profiles", profile_id).await;
    let resource_id = create_test_resource(&pool, ctx_id, "research", None, profile_id).await;

    let uri: String = sqlx::query_scalar("SELECT kb_resource_uri($1)")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("kb_resource_uri query");

    let expected_suffix = resource_id.to_string();
    assert!(
        uri.ends_with(&expected_suffix),
        "URI should end with UUID when slug is NULL, got: {uri}"
    );
    assert!(
        uri.starts_with("kb://@carol-test/my-vault/research/"),
        "URI should have correct prefix, got: {uri}"
    );
}

// ---------------------------------------------------------------------------
// 4. resource_for_uri resolves new format (slug-based)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_for_uri_resolves_new_format(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Dave Test", "dave-test").await;
    let ctx_id = create_test_context(&pool, "my-vault", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", Some("my-note"), profile_id).await;

    let uri = "kb://@dave-test/my-vault/research/my-note";

    let resolved: Option<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resource_for_uri($1, $2)")
            .bind(profile_id)
            .bind(uri)
            .fetch_optional(&pool)
            .await
            .expect("resource_for_uri query");

    assert_eq!(
        resolved,
        Some(resource_id),
        "resource_for_uri should resolve new-format URI to the correct resource"
    );
}

// ---------------------------------------------------------------------------
// 5. resource_for_uri rejects legacy no-sigil URIs (post drop-legacy migration)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_for_uri_rejects_legacy_no_sigil_uri(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Grace Test", "grace-test").await;
    let ctx_id = create_test_context(&pool, "my-vault", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", Some("rejected-note"), profile_id).await;

    // Legacy format: kb://context/type/uuid (no owner sigil).
    // Before the drop-legacy migration this resolved via the ELSE branch.
    // After the migration it must return an empty result — clients are expected
    // to upgrade to owner-scoped URIs.
    let uri = format!("kb://my-vault/research/{resource_id}");

    let resolved: Option<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resource_for_uri($1, $2)")
            .bind(profile_id)
            .bind(&uri)
            .fetch_optional(&pool)
            .await
            .expect("resource_for_uri query");

    assert_eq!(
        resolved, None,
        "legacy no-sigil URIs must return empty after drop-legacy migration"
    );
}

// ---------------------------------------------------------------------------
// 6. resource_for_uri resolves UUID in new format
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_for_uri_resolves_by_uuid_in_new_format(pool: PgPool) {
    let profile_id = create_test_profile(&pool, "Frank Test", "frank-test").await;
    let ctx_id = create_test_context(&pool, "my-vault", "kb_profiles", profile_id).await;
    let resource_id =
        create_test_resource(&pool, ctx_id, "research", Some("uuid-note"), profile_id).await;

    // New format but with UUID as identifier instead of slug
    let uri = format!("kb://@frank-test/my-vault/research/{resource_id}");

    let resolved: Option<Uuid> =
        sqlx::query_scalar("SELECT resource_id FROM resource_for_uri($1, $2)")
            .bind(profile_id)
            .bind(&uri)
            .fetch_optional(&pool)
            .await
            .expect("resource_for_uri query");

    assert_eq!(
        resolved,
        Some(resource_id),
        "resource_for_uri should resolve new-format URI with UUID to the correct resource"
    );
}
