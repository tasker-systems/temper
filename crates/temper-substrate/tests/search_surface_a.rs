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

/// Create a body-only `concept` resource (no chunks needed for FTS — body is indexed by Beat 1).
async fn mk(
    pool: &sqlx::PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
    uri: &str,
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            sources: vec![],
            title,
            origin_uri: uri,
            body,
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// Rows from `search_fts_candidates`, as (id, fts_norm).
async fn fts_candidates(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, fts_norm FROM search_fts_candidates($1, $2)")
        .bind(principal)
        .bind(q)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm")))
        .collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_candidates_normalized_and_scoped(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fts").await;
    let hit = mk(
        &pool,
        home,
        owner,
        emitter,
        "Quenching furnace",
        "tempering steel hot",
        "temper://fts/1",
    )
    .await;
    let _miss = mk(
        &pool,
        home,
        owner,
        emitter,
        "Unrelated",
        "nothing relevant here",
        "temper://fts/2",
    )
    .await;

    let got = fts_candidates(&pool, owner.uuid(), "tempering").await;
    let ids: Vec<Uuid> = got.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&hit), "matching resource is a candidate");
    assert!(!ids.contains(&_miss), "non-matching resource is absent");
    let score = got.iter().find(|(id, _)| *id == hit).unwrap().1;
    assert!(
        score > 0.0 && score < 1.0,
        "ts_rank|32 normalizes into [0,1): got {score}"
    );

    // Empty query → zero rows (term-zero path).
    assert!(
        fts_candidates(&pool, owner.uuid(), "").await.is_empty(),
        "empty query yields no candidates"
    );
}

/// Issue #356: `search_fts_candidates` now parses with `websearch_to_tsquery`, so a quoted phrase
/// forces adjacency. Two resources share both terms; only the one with them ADJACENT is a candidate
/// for the quoted query, while the unquoted query still surfaces both (backward compatible).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_candidates_supports_quoted_phrase(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "phrase").await;
    let adjacent = mk(
        &pool,
        home,
        owner,
        emitter,
        "Doc adjacent",
        "the quench hardening process strengthens steel",
        "temper://phrase/adjacent",
    )
    .await;
    let split = mk(
        &pool,
        home,
        owner,
        emitter,
        "Doc split",
        "quench the billet, then begin hardening it slowly",
        "temper://phrase/split",
    )
    .await;

    // Quoted phrase → only the adjacent resource is a candidate.
    let quoted: Vec<Uuid> = fts_candidates(&pool, owner.uuid(), "\"quench hardening\"")
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        quoted.contains(&adjacent),
        "quoted phrase yields the adjacent-terms resource as a candidate"
    );
    assert!(
        !quoted.contains(&split),
        "quoted phrase excludes the non-adjacent resource (plainto_tsquery would not have)"
    );

    // Unquoted → both (AND semantics preserved).
    let unquoted: Vec<Uuid> = fts_candidates(&pool, owner.uuid(), "quench hardening")
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        unquoted.contains(&adjacent) && unquoted.contains(&split),
        "unquoted query surfaces both resources (backward compatible)"
    );
}

// ── Vector candidates ─────────────────────────────────────────────────────────────────────────────

/// One block/chunk with a caller-chosen 768-d embedding (ONNX-free — structural).
fn block_with_embedding(content: &str, emb: Vec<f32>) -> PreparedBlock {
    PreparedBlock {
        incorporated: vec![],
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
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// pgvector text literal for binding a query embedding.
fn vlit(v: &[f32]) -> String {
    let mut s = String::from("[");
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&x.to_string());
    }
    s.push(']');
    s
}

fn unit(dim: usize) -> Vec<f32> {
    let mut e = vec![0.0_f32; 768];
    e[dim] = 1.0;
    e
}

async fn vector_candidates(
    pool: &sqlx::PgPool,
    principal: Uuid,
    emb: &[f32],
    k: i32,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, vec_norm FROM search_vector_candidates($1, $2::vector, $3)")
        .bind(principal)
        .bind(vlit(emb))
        .bind(k)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm")))
        .collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_candidates_best_per_resource_normalized(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "vec").await;
    let near = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "near",
        "temper://vec/near",
        unit(0),
    )
    .await;
    let far = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "far",
        "temper://vec/far",
        unit(1),
    )
    .await;

    let got = vector_candidates(&pool, owner.uuid(), &unit(0), 100).await;
    let near_score = got
        .iter()
        .find(|(id, _)| *id == near.uuid())
        .expect("near present")
        .1;
    let far_score = got
        .iter()
        .find(|(id, _)| *id == far.uuid())
        .expect("far present")
        .1;
    assert!(
        (near_score - 1.0).abs() < 1e-4,
        "identical embedding ⇒ vec_norm≈1.0: got {near_score}"
    );
    assert!(near_score > far_score, "nearer resource scores higher");
    assert!(
        (0.0..=1.0).contains(&far_score),
        "vec_norm bounded [0,1]: got {far_score}"
    );
}

/// The scope-aware 5-arg form (issue #358). `None`/`None` is the global path (== the 3-arg call
/// via defaults); a `Some` context or scope routes through the exact within-scope branch.
async fn vector_candidates_scoped(
    pool: &sqlx::PgPool,
    principal: Uuid,
    emb: &[f32],
    k: i32,
    context_id: Option<ContextId>,
    scope_ids: Option<&[Uuid]>,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query(
        "SELECT resource_id, vec_norm FROM search_vector_candidates($1, $2::vector, $3, $4, $5)",
    )
    .bind(principal)
    .bind(vlit(emb))
    .bind(k)
    .bind(context_id)
    .bind(scope_ids)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm")))
    .collect()
}

/// Issue #358: the global top-k ANN starves a context whose chunks aren't globally nearest, so a
/// context-scoped search loses its vector arm. Scoping the ANN by context/scope recovers those
/// candidates — perfect recall within scope, independent of the global k. `k = 1` is the sharpest
/// form of the production `k = 100` starvation: the globally-nearest resource lives in another
/// context, so the global top-1 excludes the target the scoped caller actually wants.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_candidates_scope_beats_global_topk_starvation(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let query = unit(0);

    // Target context: one resource, embedding FAR from the query (unit(5)) — never globally top-1.
    let target_ctx = ctx(&pool, owner, "target").await;
    let target = mk_embedded(
        &pool,
        target_ctx,
        owner,
        emitter,
        "target",
        "temper://t/target",
        unit(5),
    )
    .await;

    // Noise context: a resource whose embedding IS the query — globally nearest, wins any global top-k.
    let noise_ctx = ctx(&pool, owner, "noise").await;
    let noise = mk_embedded(
        &pool,
        noise_ctx,
        owner,
        emitter,
        "noise",
        "temper://t/noise",
        unit(0),
    )
    .await;

    // Global k=1 STARVES the target: only the globally-nearest (noise) survives (the #358 bug).
    let global = vector_candidates(&pool, owner.uuid(), &query, 1).await;
    assert!(
        global.iter().any(|(id, _)| *id == noise.uuid()),
        "global top-1 is the noise resource"
    );
    assert!(
        !global.iter().any(|(id, _)| *id == target.uuid()),
        "global top-1 starves the target context — the #358 bug this test pins"
    );

    // Scoped by context: same k=1, target recovered because the ANN is bounded to its context.
    let by_context =
        vector_candidates_scoped(&pool, owner.uuid(), &query, 1, Some(target_ctx), None).await;
    assert!(
        by_context.iter().any(|(id, _)| *id == target.uuid()),
        "#358: context-scoped ANN returns the in-context resource despite the global top-k"
    );
    assert!(
        !by_context.iter().any(|(id, _)| *id == noise.uuid()),
        "context scoping excludes the other context's resource"
    );

    // Scoped by explicit resource-id allowlist: same recovery via p_scope_ids.
    let by_scope =
        vector_candidates_scoped(&pool, owner.uuid(), &query, 1, None, Some(&[target.uuid()]))
            .await;
    assert!(
        by_scope.iter().any(|(id, _)| *id == target.uuid()),
        "#358: scope-id-bounded ANN returns the allowlisted resource despite the global top-k"
    );
    assert!(
        !by_scope.iter().any(|(id, _)| *id == noise.uuid()),
        "scope-id allowlist excludes the non-listed resource"
    );
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
        mk_embedded(
            &pool,
            home,
            owner,
            emitter,
            &format!("e{i}"),
            &format!("temper://ann/{i}"),
            unit(i),
        )
        .await;
    }
    // EXPLAIN the index-using shape directly (the function body's `ann` CTE).
    // SET LOCAL enable_seqscan = off: forces the planner to use available indexes so we can verify
    // the HNSW index is present and usable even on a small corpus where seq-scan would otherwise win.
    // Must run inside a transaction so SET LOCAL scopes correctly.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .unwrap();
    let plan: Vec<(String,)> = sqlx::query_as(
        "EXPLAIN SELECT c.resource_id FROM kb_chunks c WHERE c.is_current \
         ORDER BY c.embedding <=> $1::vector LIMIT 100",
    )
    .bind(vlit(&unit(0)))
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    let text = plan
        .iter()
        .map(|(l,)| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("idx_kb_chunks_embedding"),
        "ANN candidate path must use the HNSW index; plan was:\n{text}"
    );
}

/// Vector over-fetch survives a POST-ANN visibility drop. The nearest chunk may belong to a
/// resource the principal cannot see; with `p_k=100 » limit` it sits inside the ANN window, gets
/// pulled by the index ORDER BY, then the visibility join (applied AFTER the ANN) drops it — while a
/// farther-but-visible resource still survives. This guards the over-fetch shape: visibility is a
/// post-ANN filter, not an ANN-time predicate.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vector_over_fetch_survives_visibility_drop(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "vov").await;

    // Visible (caller-owned) resource: near the query but not identical (dims 0 and 1 set ⇒ cosine
    // distance ≈ 0.293 from unit(0)).
    let mut visible_emb = vec![0.0_f32; 768];
    visible_emb[0] = 1.0;
    visible_emb[1] = 1.0;
    let visible = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "visible",
        "temper://vov/visible",
        visible_emb,
    )
    .await;

    // A SECOND owner (its own profile + entity + context) whose resource is NOT visible to `owner`.
    // Its embedding is EVEN NEARER (identical to the query ⇒ distance 0, i.e. the top ANN hit), so a
    // pre-filter ANN would have surfaced it first — yet the post-ANN visibility join must drop it.
    let stranger = ProfileId::from(common::insert_profile(&pool, "stranger").await);
    let stranger_entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'stranger', '{}'::jsonb) RETURNING id",
    )
    .bind(stranger.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let stranger_home = ctx(&pool, stranger, "vov-stranger").await;
    let hidden = mk_embedded(
        &pool,
        stranger_home,
        stranger,
        EntityId::from(stranger_entity),
        "hidden",
        "temper://vov/hidden",
        unit(0),
    )
    .await;

    let got = vector_candidates(&pool, owner.uuid(), &unit(0), 100).await;
    let ids: Vec<Uuid> = got.iter().map(|(id, _)| *id).collect();
    assert!(
        ids.contains(&visible.uuid()),
        "the farther-but-visible resource survives the post-ANN visibility join"
    );
    assert!(
        !ids.contains(&hidden.uuid()),
        "the nearer non-visible resource (top ANN hit) is dropped by the post-ANN visibility join"
    );
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

use temper_substrate::readback::{self, UnifiedSearchQuery};

fn q<'a>(principal: ProfileId) -> UnifiedSearchQuery<'a> {
    UnifiedSearchQuery {
        principal,
        query: None,
        embedding: None,
        seed_ids: &[],
        depth: 2,
        edge_types: &[],
        context_id: None,
        doc_type: None,
        graph_expand: true,
        limit: 10,
        offset: 0,
        scope_ids: None,
        seed_only: false,
    }
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn blend_term_zeroing_and_either_or_dissolved(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "bl").await;
    let r = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "tempering steel",
        "temper://bl/r",
        unit(0),
    )
    .await;

    // Text-only: vector term is 0, fts term drives the score.
    let text_only = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: Some("tempering"),
            ..q(owner)
        },
    )
    .await
    .unwrap();
    let hit = text_only
        .iter()
        .find(|h| h.resource_id == r)
        .expect("found by text");
    assert!(
        hit.fts_score > 0.0 && hit.vector_score == 0.0,
        "text-only ⇒ vector term zero"
    );

    // Vector-only: fts term is 0.
    let vec_only = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            embedding: Some(&unit(0)),
            ..q(owner)
        },
    )
    .await
    .unwrap();
    let hit = vec_only
        .iter()
        .find(|h| h.resource_id == r)
        .expect("found by vector");
    assert!(
        hit.vector_score > 0.0 && hit.fts_score == 0.0,
        "vector-only ⇒ fts term zero"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn blend_self_seeding_boosts_structural_neighbor(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ss").await;
    // `core` matches the query; `neighbor` does NOT match text but is edged to `core`.
    let core = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "tempering furnace",
        "temper://ss/core",
        unit(0),
    )
    .await;
    let neighbor = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "unrelated wording",
        "temper://ss/nbr",
        unit(1),
    )
    .await;
    edge(&pool, core, neighbor, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    // Control: like `neighbor` it does NOT match the text, but it is edged to NOTHING. With no
    // FTS/vector/graph signal it never enters the candidate set — so "neighbor ranks above the
    // control" is the strongest form: present vs absent. This proves the EDGE (not some artifact)
    // is what surfaces `neighbor`.
    let control = mk_embedded(
        &pool,
        home,
        owner,
        emitter,
        "totally disconnected wording",
        "temper://ss/ctrl",
        unit(2),
    )
    .await;

    let on = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: Some("tempering"),
            graph_expand: true,
            ..q(owner)
        },
    )
    .await
    .unwrap();
    assert!(
        on.iter().any(|h| h.resource_id == neighbor),
        "graph recall-expansion pulls in the structurally-near non-text-matching neighbor"
    );
    assert!(
        on.iter().all(|h| h.resource_id != control),
        "the no-connection / no-text control never surfaces — neighbor ranks above it (present vs absent)"
    );

    let off = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: Some("tempering"),
            graph_expand: false,
            ..q(owner)
        },
    )
    .await
    .unwrap();
    assert!(
        off.iter().all(|h| h.resource_id != neighbor),
        "graph_expand=false ⇒ pure FTS∪vector, neighbor absent"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn blend_context_and_doctype_filters(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "flt").await;
    let r = mk(
        &pool,
        home,
        owner,
        emitter,
        "tempering one",
        "body tempering",
        "temper://flt/r",
    )
    .await;

    // doc_type filter that excludes 'concept' ⇒ no hits.
    let none = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: Some("tempering"),
            doc_type: Some("session"),
            ..q(owner)
        },
    )
    .await
    .unwrap();
    assert!(
        none.iter().all(|h| h.resource_id.uuid() != r),
        "doc_type filter restricts the corpus"
    );

    // doc_type='concept' keeps it.
    let some = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: Some("tempering"),
            doc_type: Some("concept"),
            ..q(owner)
        },
    )
    .await
    .unwrap();
    assert!(
        some.iter().any(|h| h.resource_id.uuid() == r),
        "matching doc_type passes the filter"
    );
}

/// When `scope_ids` is `Some([id_a])`, only `id_a` surfaces — `id_b` is filtered out even though
/// both share the same FTS term, visibility, and context.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn scope_ids_restricts_corpus(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "sc").await;
    // Both resources share the term "zscopeword" so FTS matches both.
    let id_a = mk(
        &pool,
        home,
        owner,
        emitter,
        "scope resource a",
        "body contains zscopeword alfa",
        "temper://sc/a",
    )
    .await;
    let id_b = mk(
        &pool,
        home,
        owner,
        emitter,
        "scope resource b",
        "body contains zscopeword beta",
        "temper://sc/b",
    )
    .await;

    let hits = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: Some("zscopeword"),
            scope_ids: Some(&[id_a]),
            ..q(owner)
        },
    )
    .await
    .unwrap();
    let ids: Vec<uuid::Uuid> = hits.iter().map(|h| h.resource_id.uuid()).collect();
    assert!(ids.contains(&id_a), "in-scope A should be present");
    assert!(!ids.contains(&id_b), "out-of-scope B must be filtered");
}

/// Issue #357 Option 2: `seed_only` suppresses the auto-seed union so only the caller's explicit
/// seeds define the graph neighborhood.
///
/// `core` and `other` both match the text (so both are auto-seeds); each has a text-mismatching
/// neighbor edged to it. With `seed_ids=[core]`:
///   • seed_only=false → seeds = [core] ∪ auto-seeds{core, other} → BOTH neighbors surface.
///   • seed_only=true  → seeds = [core] only → only core's neighbor surfaces; other's neighbor does not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn seed_only_suppresses_auto_seed_union(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "so").await;
    // Two text-matching resources (both become auto-seeds), each with a non-matching neighbor.
    let core = mk(
        &pool,
        home,
        owner,
        emitter,
        "core",
        "tempering core",
        "temper://so/core",
    )
    .await;
    let core_nbr = mk(
        &pool,
        home,
        owner,
        emitter,
        "cn",
        "unrelated alpha",
        "temper://so/cn",
    )
    .await;
    let other = mk(
        &pool,
        home,
        owner,
        emitter,
        "other",
        "tempering other",
        "temper://so/other",
    )
    .await;
    let other_nbr = mk(
        &pool,
        home,
        owner,
        emitter,
        "on",
        "unrelated beta",
        "temper://so/on",
    )
    .await;
    edge(
        &pool,
        ResourceId::from(core),
        ResourceId::from(core_nbr),
        home,
        emitter,
        EdgeKind::LeadsTo,
        1.0,
    )
    .await;
    edge(
        &pool,
        ResourceId::from(other),
        ResourceId::from(other_nbr),
        home,
        emitter,
        EdgeKind::LeadsTo,
        1.0,
    )
    .await;

    let seeds = [ResourceId::from(core)];
    let run = |seed_only: bool| {
        let pool = pool.clone();
        async move {
            let hits = readback::unified_search(
                &pool,
                UnifiedSearchQuery {
                    query: Some("tempering"),
                    seed_ids: &seeds,
                    seed_only,
                    ..q(owner)
                },
            )
            .await
            .unwrap();
            hits.iter()
                .map(|h| h.resource_id.uuid())
                .collect::<Vec<_>>()
        }
    };

    // Auto-seed union active → both neighbors reached (core's AND other's).
    let all = run(false).await;
    assert!(
        all.contains(&core_nbr),
        "seed_only=false: core's neighbor surfaces"
    );
    assert!(
        all.contains(&other_nbr),
        "seed_only=false: the auto-seed `other`'s neighbor also surfaces (union active)"
    );

    // Seed-only → only core's neighborhood expands; other's neighbor is not pulled in.
    let only = run(true).await;
    assert!(
        only.contains(&core_nbr),
        "seed_only=true: core's neighbor still surfaces"
    );
    assert!(
        !only.contains(&other_nbr),
        "seed_only=true: the auto-seed union is suppressed, so other's neighbor is absent"
    );
}

/// Issue #357 regression (e2e `search_context_ref_scopes_and_unknown_errors`): removing the hop-0
/// self-score must NOT make an explicit seed vanish. A seed with NO FTS/vector signal (query and
/// embedding both absent) still surfaces because it stays a candidate — but with graph_score 0, so it
/// gets no +0.5 self-bonus (the whole point of the de-bias). Old behavior gave it graph_score 1.0.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn explicit_seed_surfaces_without_hop0_self_score(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "es").await;
    // A lone resource, edged to nothing. It matches no query and has no embedding query below.
    let anchor = mk(
        &pool,
        home,
        owner,
        emitter,
        "anchor",
        "anchor body",
        "temper://es/anchor",
    )
    .await;
    let seeds = [ResourceId::from(anchor)];

    let hits = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            query: None,     // no FTS signal
            embedding: None, // no vector signal
            seed_ids: &seeds,
            graph_expand: true,
            ..q(owner)
        },
    )
    .await
    .unwrap();

    let hit = hits
        .iter()
        .find(|h| h.resource_id.uuid() == anchor)
        .expect("the explicit seed still surfaces (candidate), even with no FTS/vector signal");
    assert_eq!(
        hit.graph_score, 0.0,
        "the seed earns NO hop-0 self-score (was 1.0 before #357) — de-bias without vanishing"
    );
    assert_eq!(
        hit.combined_score, 0.0,
        "no signal at all ⇒ combined 0; present but unboosted"
    );
}
