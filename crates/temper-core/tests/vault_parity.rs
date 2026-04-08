//! Cross-implementation parity: `Vault::canonical_uri` in Rust must produce
//! byte-identical output to the SQL `kb_resource_uri()` function in Postgres
//! for the same inputs.
//!
//! This guards against drift between the two implementations. If either the
//! Rust helper or the SQL function changes, this test fails.
//!
//! Behind the `test-db` feature since it requires a live database.

#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_core::vault::Vault;
use uuid::Uuid;

// Seeded by `migrations/20260330000002_seed.sql`
const SYSTEM_PROFILE_ID: &str = "00000000-0000-0000-0004-000000000001";
const TEMPER_CONTEXT_ID: &str = "00000000-0000-0000-0003-000000000001";
const RESEARCH_DOC_TYPE_ID: &str = "00000000-0000-0000-0001-000000000004";

#[sqlx::test(migrations = "../../migrations")]
async fn vault_canonical_uri_matches_sql_kb_resource_uri(pool: PgPool) {
    // Session 3's migration backfills kb_profiles.slug from display_name.
    // The System profile's display_name is "System" → slug "system".
    let system_slug: String =
        sqlx::query_scalar("SELECT slug FROM kb_profiles WHERE id = $1::uuid")
            .bind(SYSTEM_PROFILE_ID)
            .fetch_one(&pool)
            .await
            .expect("fetch system profile slug");

    assert_eq!(
        system_slug, "system",
        "unexpected System profile slug after Session 3 backfill"
    );

    // Insert a resource into the seeded `temper` context with a known slug.
    let resource_id = Uuid::now_v7();
    let resource_slug = "vault-parity-fixture";
    sqlx::query(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES ($1, $2::uuid, $3::uuid, $4, $5, $6, $7::uuid, $7::uuid,
                true, now(), now())
        "#,
    )
    .bind(resource_id)
    .bind(TEMPER_CONTEXT_ID)
    .bind(RESEARCH_DOC_TYPE_ID)
    .bind(format!("test://parity/{resource_id}"))
    .bind("Vault Parity Fixture")
    .bind(resource_slug)
    .bind(SYSTEM_PROFILE_ID)
    .execute(&pool)
    .await
    .expect("insert parity fixture resource");

    // Ask Postgres what URI it produces for this resource.
    let sql_uri: String = sqlx::query_scalar("SELECT kb_resource_uri($1)")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("call kb_resource_uri");

    // Compute the same URI in Rust.
    let rust_uri = Vault::canonical_uri("@system", "temper", "research", resource_slug);

    assert_eq!(
        sql_uri, rust_uri,
        "SQL kb_resource_uri and Rust Vault::canonical_uri diverged"
    );

    // Expected shape sanity check — if either side produced something unexpected,
    // this asserts more clearly than comparing two mystery strings.
    assert_eq!(sql_uri, "kb://@system/temper/research/vault-parity-fixture");
}

#[sqlx::test(migrations = "../../migrations")]
async fn vault_canonical_uri_matches_sql_for_uuid_ident(pool: PgPool) {
    // A resource with NULL slug should produce a URI with the resource UUID
    // as the identifier segment. Verify Rust and SQL agree on that too.
    let resource_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES ($1, $2::uuid, $3::uuid, $4, $5, NULL, $6::uuid, $6::uuid,
                true, now(), now())
        "#,
    )
    .bind(resource_id)
    .bind(TEMPER_CONTEXT_ID)
    .bind(RESEARCH_DOC_TYPE_ID)
    .bind(format!("test://parity-uuid/{resource_id}"))
    .bind("Vault Parity UUID Fixture")
    .bind(SYSTEM_PROFILE_ID)
    .execute(&pool)
    .await
    .expect("insert parity UUID fixture resource");

    let sql_uri: String = sqlx::query_scalar("SELECT kb_resource_uri($1)")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("call kb_resource_uri");

    let rust_uri = Vault::canonical_uri("@system", "temper", "research", &resource_id.to_string());

    assert_eq!(
        sql_uri, rust_uri,
        "SQL kb_resource_uri and Rust Vault::canonical_uri diverged on UUID ident"
    );
}
