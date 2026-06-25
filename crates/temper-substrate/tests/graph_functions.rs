#![cfg(feature = "artifact-tests")]
//! WS6 collapse: `graph_traverse` / `graph_subgraph_nodes` resolve against the substrate.
//!
//! These functions are ported from the legacy `public` graph functions onto the `temper_next` shape
//! (context via `kb_resource_homes`→`kb_contexts`; doc_type/stage via `kb_properties`; first_chunk via
//! `kb_chunks`→`kb_content_blocks`→`kb_chunk_content`; edges via `kb_edges`; slug derived from title).
//! Additive — nothing calls them until the surface ports land; the legacy copies are untouched.
//!
//! Owns the `temper_next` namespace (resets 01+02, seeds a context + two edged concept resources), so
//! it is serialized via the `temper-substrate-write` nextest group.

mod common;

use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::{affinity::EdgeKind, scenario::bootseed, substrate};
use uuid::Uuid;

// ── shared helpers (mirrors write_path_mutations.rs) ────────────────────────────

/// Reset the artifact (01+02), connect, boot-seed the system actor. The standard write-path preamble.
async fn setup() -> sqlx::PgPool {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    pool
}

/// The boot-seeded canonical `system` profile + entity (owner + emitter for fixture writes).
async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

/// A profile-owned context to home resources in (the §2-amended owner-scoped shape). `name` == slug.
async fn make_context(pool: &sqlx::PgPool, owner: ProfileId, name: &str) -> ContextId {
    let id = common::insert_context(pool, "kb_profiles", owner.uuid(), name, name)
        .await
        .unwrap();
    ContextId::from(id)
}

/// One prepared block, one chunk, a fixed non-degenerate 768-d unit embedding (ONNX-free — structure,
/// not embedding quality). `content` becomes the chunk prose (and the body text).
fn one_chunk_block(content: &str) -> PreparedBlock {
    let mut embedding = vec![0.0_f32; 768];
    embedding[0] = 1.0;
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(),
            embedding,
            header_path: None,
            heading_depth: None,
        }],
    }
}

/// Create one `concept` resource homed in `ctx`, returning its id. Own tx with the search_path
/// discipline.
async fn make_resource(
    pool: &sqlx::PgPool,
    ctx: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    origin_uri: &str,
    body: &str,
) -> ResourceId {
    let blocks = vec![one_chunk_block(body)];
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title,
            origin_uri,
            resource_id: None,
            home: AnchorRef::context(ctx),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// Assert one edge `src → tgt`, returning its id. Own tx with the search_path discipline.
async fn assert_edge(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    home: ContextId,
    emitter: EntityId,
) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src,
            tgt,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("relates"),
            weight: 1.0,
            home: temper_substrate::events::EdgeHome::Context(home),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap();
    tx.commit().await.unwrap();
    id.uuid()
}

// ── graph_subgraph_nodes returns the substrate-homed aggregator resources ────────

/// Seed a context with two `concept` resources joined by one edge, then assert
/// `graph_subgraph_nodes($profile, $context, ['concept'], depth)` returns the seeded aggregators with
/// the legacy output shape (resource_id / slug / doc_type / edge_count …). Proves the ported functions
/// resolve against the `temper_next` shape.
#[tokio::test]
async fn subgraph_nodes_returns_seeded_resources() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "temper").await;
    let a = make_resource(
        &pool,
        ctx,
        owner,
        emitter,
        "Alpha Concept",
        "temper://g/a",
        "alpha body",
    )
    .await;
    let b = make_resource(
        &pool,
        ctx,
        owner,
        emitter,
        "Beta Concept",
        "temper://g/b",
        "beta body",
    )
    .await;
    let _edge = assert_edge(&pool, a, b, ctx, emitter).await;

    let rows = sqlx::query(
        "SELECT resource_id, slug, title, doc_type, edge_count, session_count, first_chunk, stage_raw \
         FROM graph_subgraph_nodes($1, $2, $3::text[], $4::int)",
    )
    .bind(owner.uuid())
    .bind("temper")
    .bind(vec!["concept".to_string()])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("graph_subgraph_nodes runs against the substrate");

    assert!(!rows.is_empty(), "seeded aggregator resources are returned");

    // The presentational slug is derived from the title (slug is §7-dissolved in the substrate).
    use sqlx::Row;
    let slugs: Vec<String> = rows.iter().map(|r| r.get::<String, _>("slug")).collect();
    assert!(
        slugs.iter().any(|s| s == "alpha-concept"),
        "slug derived from title (lowercased, dash-joined): got {slugs:?}"
    );
}
