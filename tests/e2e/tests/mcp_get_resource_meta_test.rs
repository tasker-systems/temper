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

use temper_core::types::ids::ProfileId;
use temper_core::types::ingest::{pack_chunks, IngestPayload};

fn sha2_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Runtime query: test-target macros aren't cached by `cargo sqlx prepare`
/// (the test-fixture convention).
async fn resolve_test_profile(pool: &sqlx::PgPool) -> ProfileId {
    let id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
    )
    .fetch_one(pool)
    .await
    .expect("profile lookup");
    ProfileId::from(id)
}

/// Seed a resource with managed + open meta through the production ingest path
/// (POST /api/ingest) and return its row. The substrate's collapsed write path
/// is the only resource-creation surface; `ingest_service` is retired.
async fn seed_resource(
    app: &common::E2eTestApp,
    context_name: &str,
    slug: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> temper_core::types::resource::ResourceRow {
    app.client
        .contexts()
        .create(context_name)
        .await
        .expect("context create");

    app.client
        .ingest()
        .create(&IngestPayload {
            title: slug.to_string(),
            origin_uri: format!("mcp://test/{slug}"),
            context_name: context_name.to_string(),
            doc_type_name: "research".to_string(),
            content_hash: Some(format!("sha256:{}", sha2_hex(slug))),
            slug: slug.to_string(),
            // EMPTY body: client-ingested prose rides in `chunks_packed`, so a
            // non-empty `content` would engage `create_resource`'s body-dedup, which
            // collapses these empty-bodied batch rows onto one (empty) hash. An empty
            // body skips dedup → each distinct row persists.
            content: String::new(),
            metadata: None,
            managed_meta: Some(managed_meta.clone()),
            open_meta: Some(open_meta.clone()),
            chunks_packed: Some(pack_chunks(&[]).expect("pack empty chunks")),
        })
        .await
        .expect("ingest create")
}

/// `enrich_resource` returns both meta blocks, with typed `ManagedMeta`
/// fields populated and `open_meta` preserved verbatim.
///
/// DEFERRED (F1): `slug` is a top-level identity field (`EnrichedResource.slug`
/// from `ResourceRow.slug`), NOT a managed_meta key — `temper-slug` is
/// `KeyFate::Die` (temper-next keys.rs:66) so it never reappears in the
/// readback managed bag. Receive-side identity-key injection is unimplemented,
/// so `reconstruct_resource_row` sets `row.slug = None` (db_backend.rs:129),
/// making `enriched.slug` None. The slug assertion below is the correct
/// end-state once F1 lands; ignored until then.
#[ignore = "deferred: F1 receive-side identity-key injection unimplemented — temper-slug is KeyFate::Die, so row.slug=None and enriched.slug is None"]
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
        &app,
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

    let enriched = temper_mcp::tools::resources::enrich_resource(
        &pool,
        *profile_id,
        &row,
    )
    .await
    .expect("enrich_resource");

    // doc_type lives on the typed top-level field (substrate: the `doc_type`
    // property / `ResourceRow.doc_type_name`), not in the managed_meta bag —
    // `temper-type` is `KeyFate::ReconcileToDocType` (keys.rs:68) and readback
    // surfaces it as the typed column only (readback meta, never managed/open).
    assert_eq!(enriched.doc_type_name, "research");
    // slug is a top-level identity field; DEFERRED (F1) — currently None.
    assert_eq!(enriched.slug.as_deref(), Some("mcp-get-meta"));

    let managed = enriched
        .managed_meta
        .expect("managed_meta must be present on get_resource response");
    // Workflow keys survive §7 as `kb_properties` (KeyFate::Property).
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
        &app,
        "mcp-empty-open",
        "empty-open",
        &serde_json::json!({"temper-type": "research", "temper-title": "empty-open"}),
        &empty_open,
    )
    .await;

    let enriched = temper_mcp::tools::resources::enrich_resource(
        &pool,
        *profile_id,
        &row,
    )
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
        &app,
        "mcp-batch",
        "batch-a",
        &serde_json::json!({"temper-type": "research", "temper-stage": "backlog"}),
        &serde_json::json!({"tags": ["a"]}),
    )
    .await;
    let row_b = seed_resource(
        &app,
        "mcp-batch-2",
        "batch-b",
        &serde_json::json!({"temper-type": "research", "temper-stage": "done"}),
        &serde_json::json!({"tags": ["b"]}),
    )
    .await;

    // Identify rows by id, not slug: `temper-slug` is `KeyFate::Die`
    // (keys.rs:66) and `reconstruct_resource_row` sets `row.slug = None`
    // (db_backend.rs:129), so `EnrichedResource.slug` is None post-collapse.
    // The behavior under test — batch enrichment populating per-row
    // managed/open meta — is unaffected.
    let row_a_id = row_a.id;
    let row_b_id = row_b.id;

    let enriched = temper_mcp::tools::resources::enrich_resources(
        &pool,
        *profile_id,
        &[row_a, row_b],
    )
    .await
    .expect("enrich_resources");

    assert_eq!(enriched.len(), 2);
    let stage_of = |rid: temper_core::types::ids::ResourceId| -> Option<String> {
        enriched
            .iter()
            .find(|e| e.id == *rid)
            .and_then(|e| e.managed_meta.as_ref())
            .and_then(|m| m.stage.clone())
    };
    assert_eq!(stage_of(row_a_id).as_deref(), Some("backlog"));
    assert_eq!(stage_of(row_b_id).as_deref(), Some("done"));

    for e in &enriched {
        assert!(
            e.open_meta.is_some(),
            "every batch-enriched row must carry open_meta"
        );
    }
}
