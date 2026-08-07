#![cfg(feature = "artifact-tests")]
//! `search_graph_expand` — the `follow-from` act's mechanic, on the substrate.
//! Isolated ephemeral DB via `MIGRATOR`.
//!
//! This file was Search Beat 2's whole Surface A suite: the FTS arm
//! (`search_fts_candidates`), the vector arm (`search_vector_candidates`), and the blend over both
//! (`unified_search`). All three are dropped by this commit, so what is left here is the ONE
//! Surface A function that survives it — the bidirectional, decayed, MAX-over-paths edge walk.
//!
//! The arm-level properties that outlived their functions were re-homed onto `search_exact` /
//! `search_wide` in [`search_exact_and_wide.rs`](./search_exact_and_wide.rs), not deleted: the
//! ts_rank flag-33 length normalization, `websearch_to_tsquery`'s phrase adjacency, the empty-query
//! zero, the shrunk best-of-N `vec_norm` and its full-chunk-set aggregation, the post-ANN
//! visibility drop, the scoped branch's exhaustiveness, the HNSW index path, and the whole
//! `hnsw.ef_search` pin block. What could NOT be re-homed is named in that file's header rather
//! than left to be inferred from a shorter file.

mod common;

use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use uuid::Uuid;

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

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// One block/chunk with a caller-chosen 768-d embedding (ONNX-free — structural).
fn block_with_embedding(content: &str, emb: Vec<f32>) -> PreparedBlock {
    PreparedBlock {
        incorporated: vec![],
        raw_text: None,
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(),
            embedding: Some(emb),
            embedded_with: None,
            header_path: None,
            heading_depth: None,
        }],
    }
}

async fn mk_embedded(
    pool: &sqlx::PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    uri: &str,
    emb: Vec<f32>,
) -> ResourceId {
    let blocks = vec![block_with_embedding(title, emb)];
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title,
            origin_uri: uri,
            resource_id: None,
            home: AnchorRef::context(home),
            owner,
            originator: None,
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    tx.commit().await.unwrap();
    id
}

fn unit(dim: usize) -> Vec<f32> {
    let mut e = vec![0.0_f32; 768];
    e[dim] = 1.0;
    e
}

// ── Graph candidates ────────────────────────────────────────────────────────────────────────────

use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::EdgeHome;
use temper_substrate::payloads::EdgePolarity;

/// Assert one weighted edge src→tgt of `kind`, returning nothing.
async fn edge(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    home: ContextId,
    emitter: EntityId,
    kind: EdgeKind,
    weight: f64,
) {
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src,
            tgt,
            kind,
            polarity: EdgePolarity::Forward,
            label: Some("rel"),
            weight,
            home: EdgeHome::Context(home),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap();
    tx.commit().await.unwrap();
}

async fn graph_expand(
    pool: &sqlx::PgPool,
    principal: Uuid,
    seeds: &[Uuid],
    depth: i32,
    edge_types: &[&str],
    gamma: f64,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    let et: Vec<String> = edge_types.iter().map(|s| s.to_string()).collect();
    sqlx::query("SELECT resource_id, graph_score FROM search_graph_expand($1, $2::uuid[], $3, $4::text[], $5)")
        .bind(principal).bind(seeds).bind(depth).bind(et).bind(gamma)
        .fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("graph_score"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_expand_decay_and_max_over_paths(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "g").await;
    // a — b — c  (a is the seed; b at hop 1, c at hop 2), all weight 1.0
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://g/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://g/b", unit(1)).await;
    let c = mk_embedded(&pool, home, owner, emitter, "c", "temper://g/c", unit(2)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, b, c, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let got = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &[], 0.5).await;
    let score = |id: Uuid| got.iter().find(|(g, _)| *g == id).map(|(_, s)| *s);
    // Issue #357: the seed no longer self-scores 1.0 at hop 0 — it is absent from the output unless a
    // genuine ≥1-hop path reaches it (here nothing does; the walk never revisits a path node).
    assert_eq!(score(a.uuid()), None, "seed gets no hop-0 self-score");
    assert!(
        (score(b.uuid()).unwrap() - 0.5).abs() < 1e-5,
        "hop1: γ^1·w = 0.5"
    );
    assert!(
        (score(c.uuid()).unwrap() - 0.25).abs() < 1e-5,
        "hop2: γ^2·w = 0.25 (bidirectional walk reached c)"
    );
}

/// MAX-over-paths actually CHOOSES between competing paths (the linear-chain test above never does —
/// every node has exactly one path). Diamond: seed `a`; `d` is reachable two ways of DIFFERENT score
/// — a strong 2-hop path `a—b—d` (both edges weight 1.0 ⇒ γ²·1·1 = 0.25) and a weak direct `a—d`
/// (weight 0.4 ⇒ γ¹·0.4 = 0.2). Assert `d`'s graph_score == 0.25: the BETTER path wins, not the
/// direct-but-weaker 0.2, and not the sum 0.45.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_expand_max_chooses_best_of_two_paths(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "gd").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://gd/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://gd/b", unit(1)).await;
    let d = mk_embedded(&pool, home, owner, emitter, "d", "temper://gd/d", unit(2)).await;
    // Strong path: a—b—d, both weight 1.0 ⇒ d at hop2, score γ²·1·1 = 0.25.
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, b, d, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    // Weak path: direct a—d, weight 0.4 ⇒ d at hop1, score γ¹·0.4 = 0.2.
    edge(&pool, a, d, home, emitter, EdgeKind::LeadsTo, 0.4).await;

    let got = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &[], 0.5).await;
    let d_score = got
        .iter()
        .find(|(g, _)| *g == d.uuid())
        .map(|(_, s)| *s)
        .expect("d reached");
    assert!(
        (d_score - 0.25).abs() < 1e-5,
        "MAX over paths: the strong 2-hop path (0.25) wins over the weak direct path (0.2), \
         not their sum (0.45); got {d_score}"
    );
}

/// Folded edges are excluded from graph traversal (the `NOT e.is_folded` predicate in `adj`).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_expand_excludes_folded_edges(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "gfold").await;
    let a = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "a",
        "temper://gfold/a",
        unit(0),
    )
    .await;
    let b = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "b",
        "temper://gfold/b",
        unit(1),
    )
    .await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    // Sanity: with the edge live, b is reachable from the seed a.
    let before = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &[], 0.5).await;
    assert!(
        before.iter().any(|(id, _)| *id == b.uuid()),
        "unfolded edge reaches b"
    );

    // Fold the edge directly — a sanctioned fixture mutation (no edge-id plumbing needed).
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE source_id = $1 AND target_id = $2")
        .bind(a.uuid())
        .bind(b.uuid())
        .execute(&pool)
        .await
        .unwrap();

    let after = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &[], 0.5).await;
    assert!(
        after.iter().all(|(id, _)| *id != b.uuid()),
        "folded edge is excluded from `adj` — b is no longer reachable"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_expand_filters_and_scope(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "gf").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://gf/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://gf/b", unit(1)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    // edge_types filter excludes the only edge ⇒ b unreached.
    let filtered = graph_expand(&pool, owner.uuid(), &[a.uuid()], 2, &["depends_on"], 0.5).await;
    assert!(
        filtered.iter().all(|(id, _)| *id != b.uuid()),
        "edge_types filter excludes non-matching kinds"
    );

    // A second profile that cannot see these resources gets no neighbors (visibility scoping).
    let stranger = Uuid::now_v7();
    let unscoped = graph_expand(&pool, stranger, &[a.uuid()], 2, &[], 0.5).await;
    assert!(
        unscoped.is_empty(),
        "a principal who cannot see the seeds/neighbors gets nothing"
    );
}
