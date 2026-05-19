#![cfg(feature = "test-db")]

//! Gap 2 from `mcp-frontmatter-roundtrip-gaps`: `get_resource` must surface
//! the resource's `managed_meta` and `open_meta`. Previously the MCP tool
//! returned only core fields plus an optional body, so any frontmatter
//! written via `update_resource_meta` was write-blind from the MCP surface.

mod common;

use temper_api::services::{context_service, ingest_service, resource_service};
use temper_core::types::ids::ProfileId;

fn sha2_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

async fn resolve_test_profile(pool: &sqlx::PgPool) -> ProfileId {
    ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(pool)
        .await
        .expect("profile lookup"),
    )
}

/// Seeding a resource with both `managed_meta` and `open_meta` and then
/// fetching it through the MCP `get_resource` enrichment helper must
/// return both meta blocks, with typed `ManagedMeta` fields populated
/// and `open_meta` preserved verbatim.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enrich_resource_with_meta_round_trips_managed_and_open(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "mcp-get-meta")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "mcp-get-meta")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type resolve");

    let seeded_managed = serde_json::json!({
        "temper-type": "research",
        "temper-title": "MCP Get Meta",
        "temper-slug": "mcp-get-meta",
        "temper-stage": "in-progress",
    });
    let seeded_open = serde_json::json!({"tags": ["alpha", "mcp"], "weight": 3});

    let body = "Body for round-trip.";
    let body_hash = format!("sha256:{}", sha2_hex(body));

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "mcp-get-meta",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "MCP Get Meta",
            slug: Some("mcp-get-meta"),
            origin_uri: "mcp://test/get-meta",
            content_hash: &body_hash,
            managed_meta: &seeded_managed,
            open_meta: &seeded_open,
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    let row = resource_service::get_visible(&pool, *profile_id, *resource.id)
        .await
        .expect("get_visible");

    let enriched = temper_mcp::tools::resources::enrich_resource_with_meta(&pool, profile_id, &row)
        .await
        .expect("enrich_resource_with_meta");

    let managed = enriched
        .managed_meta
        .expect("managed_meta must be present on get_resource response");
    assert_eq!(managed.doc_type.as_deref(), Some("research"));
    assert_eq!(managed.title.as_deref(), Some("MCP Get Meta"));
    assert_eq!(managed.slug.as_deref(), Some("mcp-get-meta"));
    assert_eq!(managed.stage.as_deref(), Some("in-progress"));

    let open = enriched
        .open_meta
        .expect("open_meta must be present on get_resource response");
    assert_eq!(open, seeded_open);
}

/// A resource that has no open_meta (empty JSON object) should still
/// surface a present-but-empty `open_meta` block — the field must be
/// readable, not silently dropped.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enrich_resource_with_meta_surfaces_empty_open_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    context_service::create(&pool, profile_id, "mcp-empty-open")
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(&pool, profile_id, "mcp-empty-open")
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type resolve");

    let seeded_managed =
        serde_json::json!({"temper-type": "research", "temper-title": "Empty Open"});
    let empty_open = serde_json::json!({});

    let body_hash = format!("sha256:{}", sha2_hex("body"));
    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "mcp-empty-open",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: "Empty Open",
            slug: Some("empty-open"),
            origin_uri: "mcp://test/empty-open",
            content_hash: &body_hash,
            managed_meta: &seeded_managed,
            open_meta: &empty_open,
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    let row = resource_service::get_visible(&pool, *profile_id, *resource.id)
        .await
        .expect("get_visible");

    let enriched = temper_mcp::tools::resources::enrich_resource_with_meta(&pool, profile_id, &row)
        .await
        .expect("enrich_resource_with_meta");

    assert!(
        enriched.managed_meta.is_some(),
        "managed_meta should be present even on minimally-seeded resources"
    );
    assert_eq!(
        enriched.open_meta,
        Some(empty_open),
        "empty open_meta object must still surface (not be dropped to None)"
    );
}
