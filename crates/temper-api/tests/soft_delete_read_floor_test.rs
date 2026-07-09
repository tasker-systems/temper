//! Soft-delete READ floor (SQL function audit 2026-07-08, chunk 2): a soft-deleted resource
//! (`_project_resource_deleted` flips only `kb_resources.is_active`) must stop being served by
//! every read surface. The fix centralizes an `is_active` gate inside `resources_visible_to`
//! (profile axis) and `resources_accessible_to_cogmap` (cogmap axis), so `resources_readable_by`
//! and all its consumers — `resource_blocks`, `resource_block_provenance`, `cogmap_regulation`,
//! the scope functions, element trails, `edges_visible_to` — inherit the floor instead of each
//! remembering its own `r.is_active` filter (the graph/search surfaces did; these did not).
#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// A synthetic, already-embedded chunk (constant vector) so the test-db tier needs no ONNX.
fn synthetic_chunk(index: u32, content: &str, hash_seed: &str) -> PackedChunk {
    PackedChunk {
        chunk_index: index,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.5; 768],
    }
}

/// Build a profile + JWT, return `(token, profile_id, context_id)`.
async fn auth(pool: &PgPool) -> (String, Uuid, Uuid) {
    let email = format!("soft-delete-floor-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, profile_id, context_id)
}

/// `resources_visible_to(profile)` membership for one resource.
async fn is_visible(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("resources_visible_to probe")
}

/// Row count a content surface returns for `(resource, profile)`.
async fn surface_rows(pool: &PgPool, sql: &str, resource: Uuid, profile: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(resource)
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("content surface probe")
}

const BLOCKS_COUNT: &str = "SELECT count(*) FROM resource_blocks($1, 'profile', $2, NULL)";
const PROVENANCE_COUNT: &str = "SELECT count(*) FROM resource_block_provenance($1, 'profile', $2)";

/// The production delete path (DELETE /api/resources/{id} → soft delete) must remove the
/// resource from `resources_visible_to` and from the content surfaces that gate through
/// `resources_readable_by` (`resource_blocks`, `resource_block_provenance`) — previously they
/// kept serving the deleted resource's prose and provenance.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deleted_resource_leaves_visibility_and_content_surfaces(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, profile_id, context_id) = auth(&pool).await;

    // Create a resource WITH blocks via client-supplied chunks (no ONNX in this tier).
    let chunks = vec![synthetic_chunk(
        0,
        "Prose that must go unreadable on delete.",
        "ad",
    )];
    let payload = IngestPayload {
        segmented: None,
        title: "Soft Delete Floor".to_string(),
        origin_uri: format!("test://soft-delete-floor-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: "Prose that must go unreadable on delete.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack")),
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };
    let created: Value = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .expect("ingest failed")
        .json()
        .await
        .expect("ingest JSON");
    let resource_id = Uuid::parse_str(created["id"].as_str().expect("id missing")).unwrap();

    // Give the block a provenance row so the provenance assertion is non-vacuous (plain ingest
    // carries no `incorporated` sources; fixture-level insert, self-referential source).
    sqlx::query(
        "INSERT INTO kb_block_provenance (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq)
         SELECT b.id, 'resource', $1, (SELECT id FROM kb_events ORDER BY id DESC LIMIT 1), 0
           FROM kb_content_blocks b WHERE b.resource_id = $1",
    )
    .bind(resource_id)
    .execute(&pool)
    .await
    .expect("insert fixture provenance row");

    // Pre-delete: visible, blocks served, provenance served.
    assert!(
        is_visible(&pool, profile_id, resource_id).await,
        "owner must see the live resource"
    );
    assert!(
        surface_rows(&pool, BLOCKS_COUNT, resource_id, profile_id).await >= 1,
        "resource_blocks must serve the live resource"
    );
    assert!(
        surface_rows(&pool, PROVENANCE_COUNT, resource_id, profile_id).await >= 1,
        "resource_block_provenance must serve the live resource"
    );

    // Production delete path — soft delete via the handler.
    let resp = app
        .client
        .delete(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("DELETE failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "delete must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );
    let active: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(resource_id)
        .fetch_one(&pool)
        .await
        .expect("read is_active");
    assert!(
        !active,
        "delete must be a soft delete (row preserved, is_active=false)"
    );

    // Post-delete: the READ floor closes on every surface.
    assert!(
        !is_visible(&pool, profile_id, resource_id).await,
        "resources_visible_to must exclude the soft-deleted resource"
    );
    assert_eq!(
        surface_rows(&pool, BLOCKS_COUNT, resource_id, profile_id).await,
        0,
        "resource_blocks must stop serving the soft-deleted resource's prose"
    );
    assert_eq!(
        surface_rows(&pool, PROVENANCE_COUNT, resource_id, profile_id).await,
        0,
        "resource_block_provenance must stop serving the soft-deleted resource"
    );
}

/// The cogmap principal axis: `resources_accessible_to_cogmap` (own-interior branch — homes
/// anchored to the map) must likewise exclude a soft-deleted resource, so cogmap-principal
/// reads through `resources_readable_by('cogmap', …)` inherit the same floor.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deleted_resource_leaves_cogmap_accessible_set(pool: PgPool) {
    let (_token, profile_id, _context_id) = auth(&pool).await;

    // Minimal map fixture: a telos resource, the map, and one interior resource homed in it.
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('Telos', 'test://telos') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert telos");
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('floor-map', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(&pool)
    .await
    .expect("insert cogmap");
    let interior: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('Interior', 'test://interior') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert interior resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(interior)
    .bind(cogmap)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("home interior in cogmap");

    let accessible = |resource: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM resources_accessible_to_cogmap($1) a WHERE a.resource_id = $2)",
            )
            .bind(cogmap)
            .bind(resource)
            .fetch_one(&pool)
            .await
            .expect("resources_accessible_to_cogmap probe")
        }
    };

    assert!(
        accessible(interior).await,
        "live interior resource must be accessible to its map"
    );

    // Soft delete (the projector's whole effect is this flag — see `_project_resource_deleted`).
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(interior)
        .execute(&pool)
        .await
        .expect("soft delete interior");

    assert!(
        !accessible(interior).await,
        "resources_accessible_to_cogmap must exclude the soft-deleted resource"
    );
}
