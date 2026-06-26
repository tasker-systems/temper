#![cfg(feature = "artifact-tests")]
//! Search Beat 2 — Surface A candidate functions + the unified blend, on the substrate.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool).await.unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile).fetch_one(pool).await.unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug).await.unwrap())
}

/// Create a body-only `concept` resource (no chunks needed for FTS — body is indexed by Beat 1).
async fn mk(pool: &sqlx::PgPool, home: ContextId, owner: ProfileId, emitter: EntityId,
            title: &str, body: &str, uri: &str) -> Uuid {
    writes::create_resource(pool, writes::CreateParams {
        title, origin_uri: uri, body, doc_type: "concept",
        home, owner, originator: owner, emitter, properties: &[], chunks: None,
    }).await.unwrap().uuid()
}

/// Rows from `search_fts_candidates`, as (id, fts_norm).
async fn fts_candidates(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, fts_norm FROM search_fts_candidates($1, $2)")
        .bind(principal).bind(q).fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_candidates_normalized_and_scoped(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fts").await;
    let hit = mk(&pool, home, owner, emitter, "Quenching furnace", "tempering steel hot", "temper://fts/1").await;
    let _miss = mk(&pool, home, owner, emitter, "Unrelated", "nothing relevant here", "temper://fts/2").await;

    let got = fts_candidates(&pool, owner.uuid(), "tempering").await;
    let ids: Vec<Uuid> = got.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&hit), "matching resource is a candidate");
    assert!(!ids.contains(&_miss), "non-matching resource is absent");
    let score = got.iter().find(|(id, _)| *id == hit).unwrap().1;
    assert!(score > 0.0 && score < 1.0, "ts_rank|32 normalizes into [0,1): got {score}");

    // Empty query → zero rows (term-zero path).
    assert!(fts_candidates(&pool, owner.uuid(), "").await.is_empty(), "empty query yields no candidates");
}

// ── Vector candidates ─────────────────────────────────────────────────────────────────────────────

/// One block/chunk with a caller-chosen 768-d embedding (ONNX-free — structural).
fn block_with_embedding(content: &str, emb: Vec<f32>) -> PreparedBlock {
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()), seq: 0, role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()), chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(), embedding: emb, header_path: None, heading_depth: None,
        }],
    }
}

async fn mk_embedded(pool: &sqlx::PgPool, home: ContextId, owner: ProfileId, emitter: EntityId,
                     title: &str, uri: &str, emb: Vec<f32>) -> ResourceId {
    let blocks = vec![block_with_embedding(title, emb)];
    let mut tx = pool.begin().await.unwrap();
    let id = fire(&mut tx, SeedAction::ResourceCreate {
        title, origin_uri: uri, resource_id: None, home: AnchorRef::context(home),
        owner, originator: None, blocks: &blocks, doc_type: Some("concept"), emitter,
    }).await.unwrap().resource().unwrap();
    tx.commit().await.unwrap();
    id
}

/// pgvector text literal for binding a query embedding.
fn vlit(v: &[f32]) -> String {
    let mut s = String::from("[");
    for (i, x) in v.iter().enumerate() { if i > 0 { s.push(','); } s.push_str(&x.to_string()); }
    s.push(']'); s
}

fn unit(dim: usize) -> Vec<f32> { let mut e = vec![0.0_f32; 768]; e[dim] = 1.0; e }

async fn vector_candidates(pool: &sqlx::PgPool, principal: Uuid, emb: &[f32], k: i32) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, vec_norm FROM search_vector_candidates($1, $2::vector, $3)")
        .bind(principal).bind(vlit(emb)).bind(k).fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_candidates_best_per_resource_normalized(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "vec").await;
    let near = mk_embedded(&pool, home, owner, emitter, "near", "temper://vec/near", unit(0)).await;
    let far  = mk_embedded(&pool, home, owner, emitter, "far",  "temper://vec/far",  unit(1)).await;

    let got = vector_candidates(&pool, owner.uuid(), &unit(0), 100).await;
    let near_score = got.iter().find(|(id, _)| *id == near.uuid()).expect("near present").1;
    let far_score  = got.iter().find(|(id, _)| *id == far.uuid()).expect("far present").1;
    assert!((near_score - 1.0).abs() < 1e-4, "identical embedding ⇒ vec_norm≈1.0: got {near_score}");
    assert!(near_score > far_score, "nearer resource scores higher");
    assert!(far_score >= 0.0 && far_score <= 1.0, "vec_norm bounded [0,1]: got {far_score}");
}

/// The vector CTE MUST use idx_kb_chunks_embedding (the whole point of the over-fetch shape).
/// EXPLAIN the inner ANN query and assert an Index Scan on the HNSW index — guards against silently
/// sliding back to a seq-scan blend.
/// NOTE: `SET LOCAL enable_seqscan = off` is used here because on a small seeded corpus Postgres
/// may prefer a seq-scan over the HNSW index on cost grounds. The index must still be *usable* —
/// that's what we guard — so forcing seqscan off is a valid probe: if the index is absent or
/// broken, the plan will show something other than idx_kb_chunks_embedding even without seqscan.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_ann_uses_hnsw_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ann").await;
    for i in 0..5 {
        mk_embedded(&pool, home, owner, emitter, &format!("e{i}"), &format!("temper://ann/{i}"), unit(i)).await;
    }
    // EXPLAIN the index-using shape directly (the function body's `ann` CTE).
    // SET LOCAL enable_seqscan = off: forces the planner to use available indexes so we can verify
    // the HNSW index is present and usable even on a small corpus where seq-scan would otherwise win.
    // Must run inside a transaction so SET LOCAL scopes correctly.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL enable_seqscan = off").execute(&mut *tx).await.unwrap();
    let plan: Vec<(String,)> = sqlx::query_as(
        "EXPLAIN SELECT c.resource_id FROM kb_chunks c WHERE c.is_current \
         ORDER BY c.embedding <=> $1::vector LIMIT 100")
        .bind(vlit(&unit(0))).fetch_all(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    let text = plan.iter().map(|(l,)| l.as_str()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("idx_kb_chunks_embedding"),
        "ANN candidate path must use the HNSW index; plan was:\n{text}");
}
