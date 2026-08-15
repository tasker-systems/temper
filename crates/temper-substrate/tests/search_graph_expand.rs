#![cfg(feature = "artifact-tests")]
//! The `follow-from` act's mechanic, on the substrate. Isolated ephemeral DB via `MIGRATOR`.
//!
//! **One walk, three signatures** since `20260814000030`: `search_graph_expand` (the incumbent, now
//! delegating at a byte-identical signature) → `query_follow_from` (gated, composable, carries
//! provenance) → `__temper_ungated_follow_from` (the body). So the four `graph_expand_*` tests are
//! no longer a suite for a separate function — they are this walk's regression suite, asserted
//! through the incumbent's shape, and the `follow_from_*` tests below cover what only the new
//! signature can express: the bound, the label axis, the limit, and `via`.
//!
//! This file was Search Beat 2's whole Surface A suite: the FTS arm
//! (`search_fts_candidates`), the vector arm (`search_vector_candidates`), and the blend over both
//! (`unified_search`). All three were dropped by `20260806000010`, so what is left here is the ONE
//! Surface A function that survived it — the bidirectional, decayed, MAX-over-paths edge walk.
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
use temper_substrate::ids::{BlockId, ChunkId, ContextId, EdgeId, EntityId, ProfileId, ResourceId};
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
) -> EdgeId {
    edge_full(
        pool,
        src,
        tgt,
        home,
        emitter,
        kind,
        weight,
        EdgePolarity::Forward,
        Some("rel"),
    )
    .await
}

/// The same assertion with the two fields a `via` entry reports and [`edge`] fixes: polarity, and a
/// label that may be absent. `label: None` is reachable here because `kb_edges.label` is nullable in
/// the DDL (`20260624000001:636`) even though prod carries one on every edge — the case the
/// `p_labels` axis must be shown to exclude.
#[expect(
    clippy::too_many_arguments,
    reason = "test fixture mirroring one SQL row"
)]
async fn edge_full(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    home: ContextId,
    emitter: EntityId,
    kind: EdgeKind,
    weight: f64,
    polarity: EdgePolarity,
    label: Option<&str>,
) -> EdgeId {
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src,
            tgt,
            kind,
            polarity,
            label,
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
    id
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

// ── The provenance sibling ──────────────────────────────────────────────────────────────────────
//
// `query_follow_from` / `__temper_ungated_follow_from` (`20260814000030`), design
// `docs/superpowers/specs/2026-08-14-follow-from-mechanic-design.md`.
//
// **The four tests above are now this walk's regression suite too**, and that is the point of the
// re-point rather than a side effect: `search_graph_expand` delegates here, so decay, MAX-over-paths,
// the folded-edge exclusion and the visibility scope are all asserted against the ONE body. If any
// of them needed editing to accommodate the sibling, the delegation would be wrong — the assertion
// is what is load-bearing, not its passing.

/// One row of the sibling's answer. `via` is held as raw JSON deliberately: these tests assert what
/// the SQL emits, and deserializing through a Rust type here would assert what the Rust type accepts.
struct Walked {
    id: Uuid,
    score: f32,
    via: serde_json::Value,
}

impl Walked {
    /// The `via` entries, as objects. Panics rather than returning empty on a null — "no provenance"
    /// and "this node was not reached" are different claims and a test that conflates them proves
    /// nothing.
    fn entries(&self) -> Vec<&serde_json::Map<String, serde_json::Value>> {
        self.via
            .as_array()
            .expect("via is an array on every returned row")
            .iter()
            .map(|e| e.as_object().expect("each via entry is an object"))
            .collect()
    }

    fn parents_of(&self, node: Uuid) -> Vec<Uuid> {
        // The parent is derivable from the edge as asserted — which is the point of reporting
        // source/target rather than a `from_id` plus a direction flag (spec §4). Deriving it HERE,
        // in the test, is the demonstration that it is derivable.
        self.entries()
            .iter()
            .map(|e| {
                let src: Uuid = e["source_id"].as_str().unwrap().parse().unwrap();
                let tgt: Uuid = e["target_id"].as_str().unwrap().parse().unwrap();
                if tgt == node {
                    src
                } else {
                    tgt
                }
            })
            .collect()
    }
}

#[expect(clippy::too_many_arguments, reason = "mirrors the SQL signature 1:1")]
async fn follow_from(
    pool: &sqlx::PgPool,
    principal: Uuid,
    seeds: &[Uuid],
    depth: i32,
    gamma: f64,
    edge_kinds: Option<&[&str]>,
    labels: Option<&[&str]>,
    bound: Option<&[Uuid]>,
    limit: Option<i32>,
) -> Vec<Walked> {
    use sqlx::Row;
    let kinds: Option<Vec<String>> =
        edge_kinds.map(|k| k.iter().map(|s| (*s).to_string()).collect());
    let labs: Option<Vec<String>> = labels.map(|l| l.iter().map(|s| (*s).to_string()).collect());
    let bnd: Option<Vec<Uuid>> = bound.map(<[Uuid]>::to_vec);
    sqlx::query(
        "SELECT resource_id, graph_score, via FROM query_follow_from(\
         $1, $2::uuid[], $3, $4, $5::text[], $6::text[], $7::uuid[], $8)",
    )
    .bind(principal)
    .bind(seeds)
    .bind(depth)
    .bind(gamma)
    .bind(kinds)
    .bind(labs)
    .bind(bnd)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| Walked {
        id: r.get::<Uuid, _>("resource_id"),
        score: r.get::<f32, _>("graph_score"),
        via: r.get::<serde_json::Value, _>("via"),
    })
    .collect()
}

/// **The witness for the `p_bound_ids` ruling** (spec §9): a bound constrains the WHOLE walk,
/// intermediates included — not the returned set.
///
/// `a — b — c`, bound `{a, c}`. Under the ruled reading `b` is not walkable, so `c` is unreachable
/// and does not return. Under the refused output-only reading `c` would return at hop 2 with the
/// walk having passed straight through a node the caller excluded.
///
/// The unbounded control is half the test: without it, a body that returned nothing for any reason
/// would pass.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_bound_constrains_intermediate_nodes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fb").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fb/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://fb/b", unit(1)).await;
    let c = mk_embedded(&pool, home, owner, emitter, "c", "temper://fb/c", unit(2)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, b, c, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let unbounded = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        unbounded.iter().any(|w| w.id == c.uuid()),
        "control: unbounded, c is reached at hop 2 through b"
    );

    let bounded = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        Some(&[a.uuid(), c.uuid()]),
        None,
    )
    .await;
    assert!(
        bounded.iter().all(|w| w.id != c.uuid()),
        "a bound constrains INTERMEDIATE nodes: b is excluded, so the a—b—c path cannot be walked \
         and c is unreachable. Returning c here is the output-only reading, refused in spec §9 \
         because it is CombineOp::Intersect and would be a second spelling of a combinator"
    );
    assert!(
        bounded.iter().all(|w| w.id != b.uuid()),
        "the excluded intermediate is itself absent"
    );
}

/// The two polarities of the bound, which are opposite and must both be asserted: NULL is
/// unbounded, `'{}'` returns zero rows. The empty case is the one that matters — an upstream stage
/// that produced nothing must not silently widen into a global walk (`20260808000030`).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_null_bound_is_unbounded_and_empty_bound_returns_nothing(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fp").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fp/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://fp/b", unit(1)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let unbounded = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        unbounded.iter().any(|w| w.id == b.uuid()),
        "NULL bound is UNBOUNDED — b is reached"
    );

    let empty = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        Some(&[]),
        None,
    )
    .await;
    assert!(
        empty.is_empty(),
        "an EMPTY bound returns zero rows — it never means unbounded, or a stage that produced \
         nothing would widen into a global walk"
    );
}

/// A `via` entry names the edge **as asserted** (spec §4): the edge's own `source_id`/`target_id`,
/// its `edge_kind`, `label` and `polarity`, plus the `seed_id` the node descends from.
///
/// The edge here is `Inverse` `contains` — the case the measurement singled out
/// (`[measured on prod]` 87% of `contains` edges are inverse), and the case a `from_id`-plus-
/// direction-flag shape would report backwards.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_via_names_the_edge_as_asserted(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fv").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fv/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://fv/b", unit(1)).await;
    edge_full(
        &pool,
        a,
        b,
        home,
        emitter,
        EdgeKind::Contains,
        1.0,
        EdgePolarity::Inverse,
        Some("holds"),
    )
    .await;

    let got = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        None,
        None,
    )
    .await;
    let hit = got.iter().find(|w| w.id == b.uuid()).expect("b reached");
    let entries = hit.entries();
    assert_eq!(entries.len(), 1, "one edge reached b, so one via entry");
    let e = entries[0];

    assert_eq!(
        e["seed_id"].as_str().unwrap().parse::<Uuid>().unwrap(),
        a.uuid(),
        "seed_id answers `means`' \"from any seed\""
    );
    assert_eq!(
        e["source_id"].as_str().unwrap().parse::<Uuid>().unwrap(),
        a.uuid(),
        "the edge's OWN source, as stored"
    );
    assert_eq!(
        e["target_id"].as_str().unwrap().parse::<Uuid>().unwrap(),
        b.uuid(),
        "the edge's OWN target, as stored"
    );
    assert_eq!(e["edge_kind"].as_str().unwrap(), "contains");
    assert_eq!(e["label"].as_str().unwrap(), "holds");
    assert_eq!(
        e["polarity"].as_str().unwrap(),
        "inverse",
        "polarity RIDES: the structural arrow and the semantic one disagree on 87% of contains \
         edges in prod, so a via entry omitting it reports the majority of containment \
         relationships backwards"
    );

    // **via carries NO numbers** (spec §3.2). A per-parent score is nearly free to compute at this
    // grain and is refused, because it would be a second ranking axis inside the row. Asserting the
    // key set — rather than the absence of one name — is what makes that hold against a field
    // added later under any name.
    let mut keys: Vec<&str> = e.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "edge_kind",
            "label",
            "polarity",
            "seed_id",
            "source_id",
            "target_id"
        ],
        "a via entry names the edge as asserted and carries no quantity"
    );
}

/// **Every parent, not the winning path's** (spec §3.2), and the non-correspondence that comes with
/// it: `graph_score` is a MAX over one path while the parent set is a union over ALL paths.
///
/// The diamond from `graph_expand_max_chooses_best_of_two_paths`, reused because it is already the
/// shape where the two disagree: `d` is reached by a strong 2-hop `a—b—d` (0.25, the winner) and a
/// weak direct `a—d` (0.2). One score, two parents.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_names_every_parent_while_the_score_is_one_path(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fd").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fd/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://fd/b", unit(1)).await;
    let d = mk_embedded(&pool, home, owner, emitter, "d", "temper://fd/d", unit(2)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, b, d, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, a, d, home, emitter, EdgeKind::LeadsTo, 0.4).await;

    let got = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        None,
        None,
    )
    .await;

    // One row per node — the fork spec §3 settles. Flat `(node, parent, …)` rows would put duplicate
    // ids into the next stage's seed slot and make `produced` report a PATH count.
    assert_eq!(
        got.iter().filter(|w| w.id == d.uuid()).count(),
        1,
        "one row per node, never one per path"
    );

    let hit = got.iter().find(|w| w.id == d.uuid()).unwrap();
    assert!(
        (hit.score - 0.25).abs() < 1e-5,
        "the score is the MAX over paths — the strong 2-hop path, not the weak direct one and not \
         their sum; got {}",
        hit.score
    );

    let mut parents = hit.parents_of(d.uuid());
    parents.sort_unstable();
    let mut expected = vec![a.uuid(), b.uuid()];
    expected.sort_unstable();
    assert_eq!(
        parents, expected,
        "EVERY parent, not the winning path's: d is reached from both b (the 0.25 path) and a (the \
         0.2 path), and the score corresponds to only one of them. The non-correspondence is \
         declared on the field rather than repaired"
    );
}

/// The label axis, which the walk has never had (spec §6) — and its nullable case, which the DDL
/// admits (`20260624000001:636`) and prod does not currently contain.
///
/// **A `p_labels` filter silently excludes unlabelled edges.** That is correct and is asserted here
/// rather than left to be discovered, per spec §4.3.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_label_filter_narrows_and_excludes_unlabelled_edges(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fl").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fl/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://fl/b", unit(1)).await;
    let c = mk_embedded(&pool, home, owner, emitter, "c", "temper://fl/c", unit(2)).await;
    let d = mk_embedded(&pool, home, owner, emitter, "d", "temper://fl/d", unit(3)).await;
    edge_full(
        &pool,
        a,
        b,
        home,
        emitter,
        EdgeKind::LeadsTo,
        1.0,
        EdgePolarity::Forward,
        Some("cites"),
    )
    .await;
    edge_full(
        &pool,
        a,
        c,
        home,
        emitter,
        EdgeKind::LeadsTo,
        1.0,
        EdgePolarity::Forward,
        Some("refutes"),
    )
    .await;
    edge_full(
        &pool,
        a,
        d,
        home,
        emitter,
        EdgeKind::LeadsTo,
        1.0,
        EdgePolarity::Forward,
        None,
    )
    .await;

    let all = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        1,
        0.5,
        None,
        None,
        None,
        None,
    )
    .await;
    assert_eq!(
        all.len(),
        3,
        "control: unfiltered, all three neighbours are reached — including the unlabelled one"
    );

    let narrowed = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        1,
        0.5,
        None,
        Some(&["cites"]),
        None,
        None,
    )
    .await;
    let ids: Vec<Uuid> = narrowed.iter().map(|w| w.id).collect();
    assert_eq!(ids, vec![b.uuid()], "the label axis narrows to `cites`");
    assert!(
        !ids.contains(&d.uuid()),
        "an UNLABELLED edge is excluded by any label filter — kb_edges.label is nullable and \
         `label = ANY(...)` is NULL for it, so the neighbour it reaches silently drops out. \
         Correct, and stated rather than discovered"
    );
}

/// The limit is applied to the ranked node set **before** provenance is described, which is what
/// bounds the payload (spec §5): the rows that would be expensive to describe are exactly the rows
/// that do not come back.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_limit_truncates_the_ranked_nodes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fm").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fm/a", unit(0)).await;
    let strong = mk_embedded(&pool, home, owner, emitter, "s", "temper://fm/s", unit(1)).await;
    let weak = mk_embedded(&pool, home, owner, emitter, "w", "temper://fm/w", unit(2)).await;
    edge(&pool, a, strong, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge(&pool, a, weak, home, emitter, EdgeKind::LeadsTo, 0.1).await;

    let got = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        1,
        0.5,
        None,
        None,
        None,
        Some(1),
    )
    .await;
    assert_eq!(got.len(), 1, "the limit truncates the node set");
    assert_eq!(
        got[0].id,
        strong.uuid(),
        "and it truncates the RANKED set — the weak neighbour is what the score sheds"
    );
}

/// The ungated core is ungated: handed a visible set, it applies no gate of its own.
///
/// This is the witness the audit script's header calls for — *"a test calling a core directly is
/// expected; that is how the cores' own witnesses prove they are ungated"* — and it is the negative
/// of `graph_expand_filters_and_scope`'s visibility assertion: the same corpus, reached by a
/// principal id that sees nothing, returns rows when the verdict is supplied by hand.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn follow_from_core_trusts_the_visible_set_it_is_handed(pool: sqlx::PgPool) {
    use sqlx::Row;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fu").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://fu/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://fu/b", unit(1)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let stranger = Uuid::now_v7();
    assert!(
        follow_from(&pool, stranger, &[a.uuid()], 2, 0.5, None, None, None, None)
            .await
            .is_empty(),
        "through the GATED wrapper, a principal who sees nothing gets nothing"
    );

    let rows: Vec<Uuid> = sqlx::query(
        "SELECT resource_id FROM __temper_ungated_follow_from(\
         $1::uuid[], $2::uuid[], 2, 0.5, NULL, NULL, NULL, NULL)",
    )
    .bind(vec![a.uuid(), b.uuid()])
    .bind(vec![a.uuid()])
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<Uuid, _>("resource_id"))
    .collect();
    assert_eq!(
        rows,
        vec![b.uuid()],
        "the core applies NO gate — it returns whatever the visible set it was handed admits. The \
         invariant that this set is the principal's verdict is held by the single emitter, not by \
         this body"
    );

    // And the opposite polarity from the bound, which is the pairing most likely to be got
    // backwards: a NULL visible set admits NOTHING, where a NULL bound is unbounded.
    let none: Vec<Uuid> = sqlx::query(
        "SELECT resource_id FROM __temper_ungated_follow_from(\
         NULL, $1::uuid[], 2, 0.5, NULL, NULL, NULL, NULL)",
    )
    .bind(vec![a.uuid()])
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<Uuid, _>("resource_id"))
    .collect();
    assert!(
        none.is_empty(),
        "p_visible_ids NULL admits NOTHING — fail-closed, and the opposite of p_bound_ids"
    );
}

// ── The third edge axis: property predicates that constrain a HOP (`20260815000010`) ────────────

/// Write one `kb_properties` row owned by an EDGE, through the shipped write path
/// (`property_set`, widened to a polymorphic owner by `20260727000030`).
///
/// **This helper is the point of the tests below.** Zero edge-owned properties exist in prod
/// `[measured — 2026-08-14]`, so a suite that only queried the corpus would assert an owner-agnostic
/// predicate against a population of nothing and go green — proving the SQL parses, not that it
/// narrows. Every assertion here stands on a row this function created.
async fn edge_property(
    pool: &sqlx::PgPool,
    edge: EdgeId,
    emitter: EntityId,
    key: &str,
    value: &str,
) {
    sqlx::query("SELECT property_set($1::jsonb, $2)")
        .bind(serde_json::json!({
            "property_id": Uuid::now_v7(),
            "owner": { "table": "kb_edges", "id": edge.uuid() },
            "property_key": key,
            "value": value,
            "weight": 1.0,
        }))
        .bind(emitter.uuid())
        .execute(pool)
        .await
        .expect("the edge-owned property write path is shipped and takes a polymorphic owner");
}

/// The walk, through the WIDENED arity — the only one that can carry an edge property predicate.
async fn follow_from_with_properties(
    pool: &sqlx::PgPool,
    principal: Uuid,
    seeds: &[Uuid],
    predicates: Option<serde_json::Value>,
) -> Vec<Walked> {
    use sqlx::Row;
    sqlx::query(
        "SELECT resource_id, graph_score, via FROM query_follow_from(\
         $1, $2::uuid[], 2, 0.5, NULL::text[], NULL::text[], NULL::uuid[], NULL::int, $3::jsonb)",
    )
    .bind(principal)
    .bind(seeds)
    .bind(predicates)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| Walked {
        id: r.get::<Uuid, _>("resource_id"),
        score: r.get::<f32, _>("graph_score"),
        via: r.get::<serde_json::Value, _>("via"),
    })
    .collect()
}

/// **THE witness for the edge half**, and the one the design declared could not be provided by
/// reasoning: *"zero edge-owned properties exist, so §5's owner-agnostic grain is
/// correct-by-construction and witnessed by nothing"*.
///
/// `a — b — c`, and only the `a—b` edge carries `confidence: high`. Narrowing on it must leave `b`
/// reachable and `c` NOT, because the hop that would have reached `c` is not traversable.
///
/// **`c`'s absence is the assertion that distinguishes this from the trap** (task
/// `01a000c2-033c-7451-8b13-b7aa7469d217`, design §8.2). The tempting implementation — admit nodes
/// that *participate in* a matching edge, then walk freely — returns `c` here: `b` participates in
/// the matching `a—b` edge, so it is admitted, and the walk then leaves it through the
/// non-matching `b—c` edge. That answer is plausible, ordered, and wrong, and the only thing that
/// separates it from a correct one is this row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_property_predicate_constrains_the_hop_not_merely_the_endpoint(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ep").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://ep/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://ep/b", unit(1)).await;
    let c = mk_embedded(&pool, home, owner, emitter, "c", "temper://ep/c", unit(2)).await;
    let ab = edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    let _bc = edge(&pool, b, c, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge_property(&pool, ab, emitter, "confidence", "high").await;

    // The control, and it is half the test: without it a body returning nothing for ANY reason —
    // a malformed jsonb path, a wrong owner_table, a typo in the key — would pass the assertions
    // below by accident.
    let unfiltered = follow_from_with_properties(&pool, owner.uuid(), &[a.uuid()], None).await;
    assert!(
        unfiltered.iter().any(|w| w.id == b.uuid()) && unfiltered.iter().any(|w| w.id == c.uuid()),
        "control: unfiltered, the walk reaches b at hop 1 and c at hop 2"
    );

    let narrowed = follow_from_with_properties(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        Some(
            serde_json::json!([{"key": "confidence", "op": {"op": "contains",
                                                             "values": ["high"]}}]),
        ),
    )
    .await;
    assert!(
        narrowed.iter().any(|w| w.id == b.uuid()),
        "b is reached through the edge that carries the property"
    );
    assert!(
        !narrowed.iter().any(|w| w.id == c.uuid()),
        "c is NOT reached: the b—c hop carries no matching property, and a predicate that only \
         admitted ENDPOINTS would have returned it — that is the trap this asserts against"
    );
}

/// `has_key` is a different operator and is witnessed separately: it must match the row whatever
/// the value, and must not match an edge carrying a different key.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn has_key_matches_on_presence_and_a_different_key_matches_nothing(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "hk").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://hk/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://hk/b", unit(1)).await;
    let ab = edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge_property(&pool, ab, emitter, "confidence", "high").await;

    let present = follow_from_with_properties(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        Some(serde_json::json!([{"key": "confidence", "op": {"op": "has_key"}}])),
    )
    .await;
    assert!(
        present.iter().any(|w| w.id == b.uuid()),
        "has_key matches on presence, whatever the value"
    );

    let absent = follow_from_with_properties(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        Some(serde_json::json!([{"key": "provenance", "op": {"op": "has_key"}}])),
    )
    .await;
    assert!(
        absent.is_empty(),
        "a key no edge carries narrows the walk to nothing, rather than being ignored"
    );
}

/// A malformed predicate list narrows to NOTHING rather than raising.
///
/// `jsonb_array_elements` raises on a non-array, and both levels are normalized ahead of it. This
/// is unreachable through the compiler — its input is a typed `Vec<PropertyPredicate>` — and these
/// functions are directly callable, so the alternative is a caller's malformed argument arriving as
/// a server fault.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_malformed_predicate_list_narrows_to_nothing_rather_than_raising(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "mal").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://mal/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://mal/b", unit(1)).await;
    let ab = edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;
    edge_property(&pool, ab, emitter, "confidence", "high").await;

    for malformed in [
        // not an array at the top level
        serde_json::json!({"key": "confidence", "op": {"op": "has_key"}}),
        // `values` is not an array
        serde_json::json!([{"key": "confidence", "op": {"op": "contains", "values": "high"}}]),
        // an operator the closed set does not contain
        serde_json::json!([{"key": "confidence", "op": {"op": "matches_regex"}}]),
    ] {
        let out =
            follow_from_with_properties(&pool, owner.uuid(), &[a.uuid()], Some(malformed.clone()))
                .await;
        assert!(
            out.is_empty(),
            "fail closed: {malformed} narrows to nothing and does not raise"
        );
    }
}

/// The incumbent 8-arity call still resolves, and still answers.
///
/// **This is the assertion that a `DEFAULT` on the added parameter would have broken**, and it
/// would have broken it at run time on a migration declaring itself additive: with a default, an
/// eight-argument call is ambiguous (`function ... is not unique`) rather than merely narrower.
/// The three `graph_expand_*` tests above exercise the same property through
/// `search_graph_expand`; this one names it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_incumbent_arity_is_still_unambiguously_callable(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ar").await;
    let a = mk_embedded(&pool, home, owner, emitter, "a", "temper://ar/a", unit(0)).await;
    let b = mk_embedded(&pool, home, owner, emitter, "b", "temper://ar/b", unit(1)).await;
    edge(&pool, a, b, home, emitter, EdgeKind::LeadsTo, 1.0).await;

    let via_incumbent = follow_from(
        &pool,
        owner.uuid(),
        &[a.uuid()],
        2,
        0.5,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        via_incumbent.iter().any(|w| w.id == b.uuid()),
        "the eight-argument form resolves to one function and returns the same walk"
    );
}
