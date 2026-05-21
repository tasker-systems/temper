#![cfg(feature = "test-db")]

//! Gap 2 from `mcp-frontmatter-roundtrip-gaps`: `get_resource` must surface
//! the resource's `managed_meta` and `open_meta`. Previously the MCP tool
//! returned only core fields plus an optional body, so any frontmatter
//! written via `update_resource_meta` was write-blind from the MCP surface.
//!
//! Enrichment always carries meta — `enrich_resource` (single) and
//! `enrich_resources` (batch) both populate `managed_meta` / `open_meta`.
//! The batch path fetches every manifest in one query so the list surface
//! is not N+1.

mod common;

use temper_api::services::{context_service, ingest_service, resource_service};
use temper_core::types::ids::{ProfileId, ResourceId};

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

/// Seed a resource with managed + open meta and return its row.
async fn seed_resource(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    context_name: &str,
    slug: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> temper_core::types::resource::ResourceRow {
    context_service::create(pool, profile_id, context_name)
        .await
        .expect("context create");
    let context = context_service::resolve_by_name(pool, profile_id, context_name)
        .await
        .expect("context resolve");
    let doc_type_id = ingest_service::resolve_doc_type(pool, "research")
        .await
        .expect("doc_type resolve");

    let body_hash = format!("sha256:{}", sha2_hex("body"));
    let resource = ingest_service::create_resource_with_manifest(
        pool,
        &ingest_service::CreateResourceParams {
            id: ResourceId::new(),
            profile_id,
            device_id: "mcp-get-meta",
            context_id: context.id,
            doc_type_id,
            doc_type_name: "research",
            title: slug,
            slug: Some(slug),
            origin_uri: &format!("mcp://test/{slug}"),
            content_hash: &body_hash,
            managed_meta,
            open_meta,
            chunks_packed: None,
        },
    )
    .await
    .expect("create resource");

    resource_service::get_visible(pool, *profile_id, *resource.id)
        .await
        .expect("get_visible")
}

/// `enrich_resource` returns both meta blocks, with typed `ManagedMeta`
/// fields populated and `open_meta` preserved verbatim.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enrich_resource_round_trips_managed_and_open(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    let seeded_open = serde_json::json!({"tags": ["alpha", "mcp"], "weight": 3});
    let row = seed_resource(
        &pool,
        profile_id,
        "mcp-get-meta",
        "mcp-get-meta",
        &serde_json::json!({
            "temper-type": "research",
            "temper-title": "mcp-get-meta",
            "temper-slug": "mcp-get-meta",
            "temper-stage": "in-progress",
        }),
        &seeded_open,
    )
    .await;

    let enriched = temper_mcp::tools::resources::enrich_resource(&pool, profile_id, &row)
        .await
        .expect("enrich_resource");

    let managed = enriched
        .managed_meta
        .expect("managed_meta must be present on get_resource response");
    assert_eq!(managed.doc_type.as_deref(), Some("research"));
    assert_eq!(managed.slug.as_deref(), Some("mcp-get-meta"));
    assert_eq!(managed.stage.as_deref(), Some("in-progress"));

    let open = enriched
        .open_meta
        .expect("open_meta must be present on get_resource response");
    assert_eq!(open, seeded_open);
}

/// A resource whose open_meta is an empty object must still surface a
/// present-but-empty `open_meta` block — readable, not silently dropped.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enrich_resource_surfaces_empty_open_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    let empty_open = serde_json::json!({});
    let row = seed_resource(
        &pool,
        profile_id,
        "mcp-empty-open",
        "empty-open",
        &serde_json::json!({"temper-type": "research", "temper-title": "empty-open"}),
        &empty_open,
    )
    .await;

    let enriched = temper_mcp::tools::resources::enrich_resource(&pool, profile_id, &row)
        .await
        .expect("enrich_resource");

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

/// The batch enrichment path (used by `list_resources`) populates meta
/// for every row — the list surface is no longer meta-blind.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enrich_resources_includes_meta_for_every_row(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = resolve_test_profile(&pool).await;

    let row_a = seed_resource(
        &pool,
        profile_id,
        "mcp-batch",
        "batch-a",
        &serde_json::json!({"temper-type": "research", "temper-stage": "backlog"}),
        &serde_json::json!({"tags": ["a"]}),
    )
    .await;
    let row_b = seed_resource(
        &pool,
        profile_id,
        "mcp-batch-2",
        "batch-b",
        &serde_json::json!({"temper-type": "research", "temper-stage": "done"}),
        &serde_json::json!({"tags": ["b"]}),
    )
    .await;

    let enriched =
        temper_mcp::tools::resources::enrich_resources(&pool, profile_id, &[row_a, row_b])
            .await
            .expect("enrich_resources");

    assert_eq!(enriched.len(), 2);
    let stage_of = |slug: &str| -> Option<String> {
        enriched
            .iter()
            .find(|e| e.slug.as_deref() == Some(slug))
            .and_then(|e| e.managed_meta.as_ref())
            .and_then(|m| m.stage.clone())
    };
    assert_eq!(stage_of("batch-a").as_deref(), Some("backlog"));
    assert_eq!(stage_of("batch-b").as_deref(), Some("done"));

    for e in &enriched {
        assert!(
            e.open_meta.is_some(),
            "every batch-enriched row must carry open_meta"
        );
    }
}
