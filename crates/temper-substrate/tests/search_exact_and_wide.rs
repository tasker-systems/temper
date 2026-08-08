#![cfg(feature = "artifact-tests")]
//! Phase 1 — `/api/search` as two arms that are never combined: `search_exact` (you can quote the
//! words) and `search_wide` (you have the idea, not the words). Each returns its own quantity under
//! its own name and nothing sums them.
//!
//! Under task `019fd25e-95f0-7373-9a6e-0574deea5ab3` and decision
//! `019fd25a-ef4c-7473-b72e-265a7d36dd65`. Isolated ephemeral DB via `MIGRATOR`.
//!
//! ## Re-homed from `search_graph_expand.rs`
//!
//! `unified_search`, `search_fts_candidates` and `search_vector_candidates` are dropped by the
//! commit that retires the blend. Their witnesses did not all die with them: the arms carry
//! `search_fts_candidates`' `websearch_to_tsquery` + ts_rank flag 33 and `search_vector_candidates`'
//! shrunk best-of-N, both branches and the `hnsw.ef_search` pin, VERBATIM (migration
//! `20260805000020` header: "Body derived from `search_fts_candidates`' live definition, INCLUDING
//! `websearch_to_tsquery` and ts_rank flag 33 … Re-deriving from an older migration silently reverts
//! both"). Those properties are re-asserted below against `search_exact` / `search_wide` rather than
//! deleted along with the function that used to carry them.
//!
//! One of them was a TRAP, not merely a move. `pinned_ef_search_covers_unified_search_vector_k`
//! read `SELECT prosrc FROM pg_proc WHERE proname='unified_search'` with `fetch_one(…).unwrap()`.
//! Once that function is dropped the read yields `RowNotFound` and the test PANICS instead of
//! failing its assertion — the invariant would have died by crash while still looking like a test.
//! `search_wide_pins_ef_search_at_or_above_the_k_it_is_asked_for` (below) is its re-homing and
//! predates this commit; what was still missing, and is added here, is the pin's SCOPE.
//!
//! ## Declared coverage loss
//!
//! Two Surface A properties have no home after the drop and are named rather than left to be
//! inferred from a shorter file:
//!
//! * **`p_scope_ids` as an explicit resource-id allowlist** (`search_vector_candidates`' 5th
//!   argument). `search_wide` scopes by anchor pair only; there is no per-resource allowlist to
//!   witness. Retired with the parameter, not pending.
//! * **The blend itself** — term zeroing, the dissolved either/or, `w_fts`/`w_vec`/`w_graph`
//!   weighting, `seed_only`, the hop-0 self-score de-bias, and the `doc_type` post-filter. All were
//!   properties of `unified_search`'s composition, which this phase exists to stop. The graph
//!   mechanic they composed (`search_graph_expand`) survives and keeps its own tests in
//!   `search_graph_expand.rs`; the composition does not.

mod common;

use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{
    BlockId, ChunkId, CogmapId, ContextId, EntityId, ProfileId, ResourceId,
};
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

/// A body-only `concept` resource. The body is FTS-indexed by Beat 1, so the exact arm needs no
/// chunks.
async fn mk_at(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://{title}"),
            body,
            doc_type: "concept",
            home,
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

/// [`mk`] with frontmatter properties attached, so the enrichment read has both metadata tiers to
/// tell apart: a `temper-*` key is managed, anything else is open.
async fn mk_with_props(
    pool: &sqlx::PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
    properties: &[(String, serde_json::Value)],
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://{title}"),
            body,
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: None,
        },
    )
    .await
    .unwrap()
    .uuid()
}

/// Context-homed shorthand for [`mk_at`].
async fn mk(
    pool: &sqlx::PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
) -> Uuid {
    mk_at(pool, AnchorRef::context(home), owner, emitter, title, body).await
}

/// Make `cogmap` READABLE by `profile`: a fresh team, joined to the map, with `profile` in it.
///
/// This is the only path `cogmap_readable_by_profile` recognizes short of an explicit grant, and
/// `common::genesis_cogmap` does NOT take it — a genesis'd map is joined to no team, so nobody
/// reads it. Any test that means "a cogmap anchor scopes to its members" must call this; a test
/// that omits it is asserting on a map it cannot read, which is a different (and much weaker)
/// claim. Raw inserts, in the same fixture idiom as `citation_audits.rs:join_principal_to_cogmap`.
async fn make_cogmap_readable(pool: &sqlx::PgPool, cogmap: Uuid, profile: ProfileId) {
    let team = common::create_team(pool, &format!("t-{}", &Uuid::now_v7().to_string()[..8])).await;
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("join cogmap to team");
    common::add_team_member(pool, team, profile.uuid()).await;
}

/// Rows from `search_exact`, as (id, fts_norm). `anchor` is the `(anchor_table, anchor_id)` pair —
/// `None` for unscoped.
async fn exact(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    q: &str,
    anchor: Option<(&str, Uuid)>,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    let (table, id) = match anchor {
        Some((t, i)) => (Some(t), Some(i)),
        None => (None, None),
    };
    sqlx::query("SELECT resource_id, fts_norm FROM search_exact($1, $2, $3, $4)")
        .bind(principal.uuid())
        .bind(q)
        .bind(table)
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm")))
        .collect()
}

/// A resource with one chunk per entry of `embs` — `None` meaning that chunk carries NO vector.
///
/// `None` is not a synthetic state: it is what `prepare_block_deferred` emits on every chunk of an
/// async-embedded create, and `content.rs:31-36` documents it mapping to a NULL
/// `kb_chunks.embedding` until the backfill runs (issue #299). Every resource passes through it.
/// A MIXED slice is the partially-drained case — some chunks embedded, some not yet.
async fn mk_chunked(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    embs: Vec<Option<Vec<f32>>>,
) -> Uuid {
    let blocks = vec![PreparedBlock {
        incorporated: vec![],
        raw_text: None,
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: embs
            .into_iter()
            .enumerate()
            .map(|(i, emb)| PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: i as i32,
                content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
                content: title.to_string(),
                embedding: emb,
                embedded_with: None,
                header_path: None,
                heading_depth: None,
            })
            .collect(),
    }];
    let mut tx = pool.begin().await.unwrap();
    let id = fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title,
            origin_uri: &format!("test://{title}"),
            resource_id: None,
            home,
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
    id.uuid()
}

/// A single-chunk resource carrying `emb`. The wide arm scores chunks, so these need embeddings
/// where the exact arm's needed only a body. Shorthand for [`mk_chunked`].
async fn mk_embedded(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    emb: Vec<f32>,
) -> Uuid {
    mk_chunked(pool, home, owner, emitter, title, vec![Some(emb)]).await
}

/// A single-chunk resource created but NOT yet embedded — [`mk_chunked`] with no vector.
async fn mk_unembedded(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
) -> Uuid {
    mk_chunked(pool, home, owner, emitter, title, vec![None]).await
}

/// pgvector text literal for binding a query embedding.
fn vlit(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(f32::to_string).collect();
    format!("[{}]", parts.join(","))
}

/// A 768-dim unit vector along one axis. Two distinct axes are orthogonal, which makes "near" and
/// "far" unambiguous without depending on a real embedding model.
fn unit(dim: usize) -> Vec<f32> {
    let mut e = vec![0.0_f32; 768];
    e[dim] = 1.0;
    e
}

/// A unit vector whose cosine similarity to [`unit(0)`] is exactly `c` (so `<=>` distance is
/// `1 - c`). Lets a fixture name the distance it wants instead of solving for a vector.
fn at_cos(c: f32) -> Vec<f32> {
    let mut e = vec![0.0_f32; 768];
    e[0] = c;
    e[1] = (1.0 - c * c).sqrt();
    e
}

/// Rows from `search_wide`, as (id, vec_norm).
async fn wide(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    emb: &[f32],
    k: i32,
    anchor: Option<(&str, Uuid)>,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    let (table, id) = match anchor {
        Some((t, i)) => (Some(t), Some(i)),
        None => (None, None),
    };
    sqlx::query("SELECT resource_id, vec_norm FROM search_wide($1, $2::vector, $3, $4, $5)")
        .bind(principal.uuid())
        .bind(vlit(emb))
        .bind(k)
        .bind(table)
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm")))
        .collect()
}

/// The exact arm's whole job: a resource whose body carries the queried term comes back with a
/// quantity named `fts_norm`, and one that does not carry it does not come back at all.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_scores_a_term_match_and_omits_a_non_match(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "exact").await;

    let hit = mk(
        &pool,
        home,
        owner,
        emitter,
        "Falconry",
        "The peregrine stoops at terminal velocity.",
    )
    .await;
    let miss = mk(
        &pool,
        home,
        owner,
        emitter,
        "Cartography",
        "A projection is a lie you agree to.",
    )
    .await;

    let rows = exact(&pool, owner, "peregrine", None).await;

    let ids: Vec<Uuid> = rows.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&hit), "the term-carrying resource must match");
    assert!(
        !ids.contains(&miss),
        "a resource that does not carry the term must not match"
    );

    let (_, score) = rows.iter().find(|(id, _)| *id == hit).unwrap();
    assert!(
        *score > 0.0 && *score < 1.0,
        "fts_norm is a ts_rank flag-33 quantity in [0,1): got {score}"
    );
}

/// An anchor pair narrows the arm to what is homed under that anchor. Both resources match the
/// term, so nothing but the scope can separate them — which is what makes this test about scoping
/// rather than about ranking.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_scopes_to_a_context_anchor(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let inside = ctx(&pool, owner, "inside").await;
    let outside = ctx(&pool, owner, "outside").await;

    let here = mk(
        &pool,
        inside,
        owner,
        emitter,
        "Inside",
        "The peregrine is here.",
    )
    .await;
    let there = mk(
        &pool,
        outside,
        owner,
        emitter,
        "Outside",
        "The peregrine is elsewhere.",
    )
    .await;

    let unscoped: Vec<Uuid> = exact(&pool, owner, "peregrine", None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        unscoped.contains(&here) && unscoped.contains(&there),
        "both resources carry the term, so an unscoped search must return both"
    );

    let scoped: Vec<Uuid> = exact(
        &pool,
        owner,
        "peregrine",
        Some(("kb_contexts", inside.uuid())),
    )
    .await
    .into_iter()
    .map(|(id, _)| id)
    .collect();

    assert_eq!(
        scoped,
        vec![here],
        "an anchor pair naming a context admits exactly that context's homed resources"
    );
}

/// The anchor KIND is a parameter, not a branch — the same pair scopes a cogmap with no separate
/// code path and, critically, with no materialized member array.
///
/// This is what retires `cogmap_scope_ids_multi` from the search path. Today a cogmap scope is
/// resolved by flattening the map's visible participants into a `uuid[]` in Rust
/// (`substrate_read.rs:575-590`) and passing it down as `p_scope_ids`; a context scope is an EXISTS.
/// One pair replaces both, and `idx_kb_resource_homes_anchor(anchor_table, anchor_id)` already
/// indexes it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_scopes_to_a_cogmap_anchor_by_the_same_pair(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "Falconry Map", "Know the birds").await;
    // The map must be READABLE for this test to be about anchor scoping at all. Without this the
    // arm's readability conjunct empties the scope and the test would assert nothing about the
    // anchor pair — see `an_unreadable_cogmap_anchor_scopes_nothing_in_either_arm`, which is the
    // same fixture MINUS this line and expects the opposite.
    make_cogmap_readable(&pool, cogmap, owner).await;
    let elsewhere = ctx(&pool, owner, "elsewhere").await;

    let in_map = mk_at(
        &pool,
        AnchorRef::cogmap(CogmapId::from(cogmap)),
        owner,
        emitter,
        "Mapped",
        "The peregrine is distilled here.",
    )
    .await;
    let in_context = mk(
        &pool,
        elsewhere,
        owner,
        emitter,
        "Unmapped",
        "The peregrine is raw here.",
    )
    .await;

    let scoped: Vec<Uuid> = exact(&pool, owner, "peregrine", Some(("kb_cogmaps", cogmap)))
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    assert!(
        scoped.contains(&in_map),
        "a cogmap-homed resource must be admitted by its own anchor pair"
    );
    assert!(
        !scoped.contains(&in_context),
        "a context-homed resource must not be admitted by a cogmap anchor"
    );
}

/// The wide arm's whole job: proximity in embedding space, under its own name `vec_norm`, with no
/// term ever matched. Neither resource's text shares a word with the other, and the query is a bare
/// vector — so only the embedding can separate them.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_wide_scores_by_proximity_under_its_own_name(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "wide").await;

    let near = mk_embedded(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "Near",
        unit(0),
    )
    .await;
    let far = mk_embedded(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "Far",
        unit(1),
    )
    .await;

    let rows = wide(&pool, owner, &unit(0), 100, None).await;

    let score = |id: Uuid| rows.iter().find(|(r, _)| *r == id).map(|(_, s)| *s);
    let (n, f) = (score(near), score(far));
    assert!(
        n.is_some() && f.is_some(),
        "both embedded resources are candidates: got near={n:?} far={f:?}"
    );
    let (n, f) = (n.unwrap(), f.unwrap());

    assert!(
        n > f,
        "the resource whose chunk sits on the query axis must outscore the orthogonal one: \
         near={n} far={f}"
    );
    assert!(
        (0.0..=1.0).contains(&n) && (0.0..=1.0).contains(&f),
        "vec_norm rescales the cosine DISTANCE (which spans [0,2]) as 1 - d/2, so it lies in \
         [0,1]: near={n} far={f}"
    );
}

/// `hnsw.ef_search` must be pinned on `search_wide` ITSELF.
///
/// `pg_proc.proconfig` binds to a signature, so the pin 20260804000030 put on
/// `search_vector_candidates` does not reach a new function. Without its own pin `search_wide` runs
/// at the server default of 40, which sits below any k a caller passes — `LIMIT p_k` becomes
/// unreachable and the draw truncates with no error and no symptom. Measured on prod before the
/// original fix: 33.4 chunks and 24.3 admitted resources per query against an exact scan's 66.2.
///
/// Asserted as `pin >= k`, never as `pin == 200`: a value assertion pins a number nobody may tune,
/// while the invariant is that the candidate list is at least as large as what the caller asks for.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_wide_pins_ef_search_at_or_above_the_k_it_is_asked_for(pool: sqlx::PgPool) {
    let cfg: Option<Vec<String>> =
        sqlx::query_scalar("SELECT proconfig FROM pg_proc WHERE proname = 'search_wide'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let pinned: i64 = cfg
        .unwrap_or_default()
        .iter()
        .find_map(|e| e.strip_prefix("hnsw.ef_search=").map(str::to_owned))
        .expect(
            "search_wide must pin hnsw.ef_search on the function itself; the pin on \
             search_vector_candidates binds to that signature and does not inherit",
        )
        .parse()
        .expect("ef_search pin is numeric");

    // The k the read path asks for, carried over from `unified_search`'s `vector_k` — the one
    // constant this phase keeps, because it is an admission parameter rather than a ranking one.
    let asked_for: i64 = 100;
    assert!(
        pinned >= asked_for,
        "hnsw.ef_search ({pinned}) must be >= the k search_wide is called with ({asked_for}); \
         below it the ANN cannot return the k rows requested and admission truncates silently"
    );
}

/// An interrupted ingest is not a document, and neither arm may present it as one.
///
/// This is the gate the phase must not lose. `unified_search` enforced `ingest_state = 'complete'`
/// centrally in its `corpus` CTE as well as inside each arm; that central copy exists for the graph
/// and seed arms, which carry no gate of their own, and it dies with them. What keeps the property
/// true afterwards is each arm's own inline predicate — so both are asserted here, together,
/// against one resource.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_in_progress_resource_appears_in_neither_arm(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "partial").await;

    let partial = mk_embedded(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "peregrine",
        unit(0),
    )
    .await;

    // Both arms see it while it is whole — otherwise the assertions below would pass for the wrong
    // reason (a resource nothing could ever match is excluded by every gate at once).
    assert!(
        exact(&pool, owner, "peregrine", None)
            .await
            .iter()
            .any(|(id, _)| *id == partial),
        "precondition: a complete resource is matched by the exact arm"
    );
    assert!(
        wide(&pool, owner, &unit(0), 100, None)
            .await
            .iter()
            .any(|(id, _)| *id == partial),
        "precondition: a complete resource is matched by the wide arm"
    );

    sqlx::query("UPDATE kb_resources SET ingest_state = 'in_progress' WHERE id = $1")
        .bind(partial)
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        !exact(&pool, owner, "peregrine", None)
            .await
            .iter()
            .any(|(id, _)| *id == partial),
        "an in_progress resource must not surface in the exact arm"
    );
    assert!(
        !wide(&pool, owner, &unit(0), 100, None)
            .await
            .iter()
            .any(|(id, _)| *id == partial),
        "an in_progress resource must not surface in the wide arm"
    );
}

/// A resource that is not embedded yet is ABSENT from the wide arm — not present with a low score,
/// and not a 500.
///
/// The corpus is deliberately TINY, because that is the only shape in which this bites. A NULL
/// `embedding <=> p_emb` sorts LAST, so an unembedded chunk reaches the top-k only once there are
/// fewer than `k` embedded chunks to fill it: a new tenant, a fresh context, the first search after
/// a deploy. One embedded chunk against `k = 100` is that corpus. On a warm database the NULLs are
/// crowded out and everything looks fine, which is exactly why this shipped.
///
/// Two failures are asserted at once because the guard fixes one by preventing the other:
///
/// 1. **No error.** `wide()` decodes `vec_norm` as a non-nullable `f32` (as `WideHit` does), so a
///    NULL aggregate panics here in the same place production returned
///    `500 search stage=search_wide: decoding column "vec_norm": unexpected null`. Reaching the
///    assertions below at all is the first half of the claim.
/// 2. **No hit.** The resource must not appear — an unembedded chunk is an absent opinion, not a
///    distant one, so `COALESCE(vec_norm, 0)` would have satisfied (1) while ranking a resource this
///    arm cannot yet have an opinion about.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_whose_chunks_are_not_embedded_yet_is_absent_from_the_wide_arm(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "unembedded").await;

    let embedded = mk_embedded(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "kestrel",
        unit(0),
    )
    .await;
    let pending = mk_unembedded(&pool, AnchorRef::context(home), owner, emitter, "merlin").await;

    // The fixture is what the test claims it is. Without this the assertions below could pass
    // because the write path quietly stored no chunk at all, which is a different (and vacuous)
    // fact than "its chunk carries no vector".
    let unembedded_current: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND is_current AND embedding IS NULL",
    )
    .bind(pending)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        unembedded_current, 1,
        "precondition: the pending resource has exactly one current chunk and it carries no vector"
    );

    // And it is otherwise a perfectly good, visible, complete resource — so its absence from the
    // wide arm below is about the missing vector and nothing else.
    assert!(
        exact(&pool, owner, "merlin", None)
            .await
            .iter()
            .any(|(id, _)| *id == pending),
        "precondition: the pending resource is visible and complete — the exact arm finds it"
    );

    // `k` far exceeds the one embedded chunk in the corpus, so the NULL-distance chunks are not
    // crowded out of the draw. This call panics on the unfixed body.
    let rows = wide(&pool, owner, &unit(0), 100, None).await;

    assert!(
        !rows.iter().any(|(id, _)| *id == pending),
        "a resource whose current chunks carry no embedding must not appear in the wide arm at \
         all — not even scored 0, which would assert a maximal distance the data does not support"
    );
    assert!(
        rows.iter().any(|(id, _)| *id == embedded),
        "the embedded resource must still be found — otherwise the assertion above passes \
         vacuously because the arm returned nothing"
    );

    // The SCOPED branch is a different query with its own guard, not a filtered version of the one
    // above: it pre-filters into `scoped_res`, carries no top-k, and aggregates over its own `ann`.
    // An unguarded copy of it returns the same NULL, so the property is asserted on both branches
    // rather than assumed to transfer.
    let scoped = wide(
        &pool,
        owner,
        &unit(0),
        100,
        Some(("kb_contexts", home.uuid())),
    )
    .await;
    assert!(
        !scoped.iter().any(|(id, _)| *id == pending),
        "the scoped branch must exclude the unembedded resource too — it is a separate query with \
         its own guard, and adding a scope must not resurrect a resource the arm has no opinion on"
    );
    assert!(
        scoped.iter().any(|(id, _)| *id == embedded),
        "precondition: the scoped branch reaches this context's embedded resource at all"
    );
}

/// A partially embedded resource is scored on its embedded chunks ALONE.
///
/// The second, quieter half of the same flaw. `MIN`/`AVG` skip NULLs but `count(*)` does not, so an
/// unembedded chunk left in the final aggregate inflates the shrinkage denominator
/// `1 - 1/sqrt(count(*))` and drags the score toward a mean the chunk never contributed to. That is
/// a wrong NUMBER rather than a NULL — no error, no symptom — so a guard placed only on the `ann`
/// draw would fix the 500 and leave this behind.
///
/// Asserted against the twin that carries the SAME two embedded chunks and nothing else, rather
/// than against a hardcoded constant: the claim is "the unembedded chunks did not count", which is
/// an equality between two resources, not a particular float. That also keeps the test honest if
/// the shrinkage formula is ever retuned.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unembedded_chunks_do_not_dilute_a_partially_embedded_resources_score(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "partial-embed").await;

    // Two chunks embedded on orthogonal axes, so MIN and AVG genuinely differ and the shrinkage
    // term is not multiplied by zero — with MIN == AVG the dilution would be unobservable.
    let drained = mk_chunked(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "harrier",
        vec![Some(unit(0)), Some(unit(1))],
    )
    .await;
    let draining = mk_chunked(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "goshawk",
        vec![Some(unit(0)), Some(unit(1)), None, None],
    )
    .await;

    let rows = wide(&pool, owner, &unit(0), 100, None).await;
    let score = |id: Uuid| rows.iter().find(|(r, _)| *r == id).map(|(_, s)| *s);
    let (d, p) = (score(drained), score(draining));
    assert!(
        d.is_some() && p.is_some(),
        "both resources carry embedded chunks, so both are candidates: drained={d:?} draining={p:?}"
    );
    let (d, p) = (d.unwrap(), p.unwrap());

    assert_eq!(
        d, p,
        "a resource with two embedded chunks and two unembedded ones must score exactly as one \
         with the same two embedded chunks and nothing else; a difference means the unembedded \
         chunks entered count(*) and diluted the score toward a mean they never contributed to"
    );
}

/// Enrichment answers in the ONE resource shape, for a whole page of hits, in ONE round trip.
///
/// Three things are asserted together because they are the three ways this read can regress:
///
/// 1. **Every id comes back.** Ten ids in, ten views out — the batched call is what replaced the
///    per-hit `resource_row` (`readback/mod.rs`: "50 results meant 51 queries"), so a widened
///    SELECT that dropped rows or needed a follow-up query per row would fail here.
/// 2. **`managed_meta` is populated from the property tier, and only the managed half of it.**
///    Each resource carries a `temper-*` key and an open key. `ManagedMeta` is
///    `deny_unknown_fields`, so an open key leaking into the managed tier is a hard
///    deserialization failure, not a soft one — which is what makes the negative half of this
///    conjunct self-enforcing.
/// 3. **`content` is absent.** The body is a section this read never fetches; `None` means "not
///    requested", and a body arriving here would mean the enrichment read had started
///    reconstructing markdown for every search hit.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn hit_identities_is_one_round_trip_and_carries_both_meta_tiers(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "enriched").await;

    let mut ids: Vec<ResourceId> = Vec::new();
    for n in 0..10_i64 {
        let id = mk_with_props(
            &pool,
            home,
            owner,
            emitter,
            &format!("Falconry {n}"),
            "The peregrine stoops at terminal velocity.",
            &[
                ("temper-stage".to_string(), serde_json::json!("doing")),
                ("temper-seq".to_string(), serde_json::json!(n)),
                // An OPEN key. It must not reach `ManagedMeta`, which rejects unknown fields.
                ("falcon".to_string(), serde_json::json!("peregrine")),
            ],
        )
        .await;
        ids.push(ResourceId::from(id));
    }

    let views = temper_substrate::readback::hit_identities(&pool, owner, &ids)
        .await
        .unwrap();

    assert_eq!(
        views.len(),
        ids.len(),
        "every visible id is enriched by the one batched call"
    );

    for view in &views {
        assert_eq!(
            view.managed_meta.stage.as_deref(),
            Some("doing"),
            "the managed tier is joined in, not left empty: {:?}",
            view.managed_meta
        );
        assert!(
            view.managed_meta.seq.is_some(),
            "a numeric managed key survives the join as a number: {:?}",
            view.managed_meta
        );
        assert!(
            view.content.is_none(),
            "the body is a section this read never fetches"
        );
        assert!(
            view.r#ref.ends_with(&view.id.uuid().to_string()),
            "the decorated ref is derived, not selected: {}",
            view.r#ref
        );
        assert_eq!(
            view.context_ref.as_deref(),
            Some("@system/enriched"),
            "the context ref is derived from the owner ref + slug beside it"
        );
    }

    let mut seqs: Vec<i64> = views
        .iter()
        .map(|v| v.managed_meta.seq.expect("temper-seq"))
        .collect();
    seqs.sort_unstable();
    assert_eq!(
        seqs,
        (0..10_i64).collect::<Vec<_>>(),
        "each view carries ITS OWN managed meta — not one resource's smeared across the page"
    );
}

/// The enrichment read is visibility-gated: a resource the principal cannot see is simply absent,
/// never an error and never a leaked title.
///
/// The precondition half matters as much as the assertion — without it the test would pass for a
/// resource that no principal could see, which is the wrong reason.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn hit_identities_omits_a_resource_the_principal_cannot_see(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "private").await;

    let hidden = ResourceId::from(
        mk(
            &pool,
            home,
            owner,
            emitter,
            "Falconry",
            "The peregrine stoops at terminal velocity.",
        )
        .await,
    );

    let mine = temper_substrate::readback::hit_identities(&pool, owner, &[hidden])
        .await
        .unwrap();
    assert_eq!(
        mine.len(),
        1,
        "precondition: the owner of the home sees their own resource"
    );

    let stranger = ProfileId::from(common::insert_profile(&pool, "stranger").await);
    let theirs = temper_substrate::readback::hit_identities(&pool, stranger, &[hidden])
        .await
        .unwrap();
    assert!(
        theirs.is_empty(),
        "a principal with no grant, no team and no home ownership must be enriched nothing: {:?}",
        theirs.iter().map(|v| &v.title).collect::<Vec<_>>()
    );
}

/// The gate discriminates WITHIN a batch: a mixed set of ids comes back filtered per row.
///
/// `hit_identities` binds `$2` twice — once in its `WHERE` and once on the `resources_visible_to`
/// join, where the second copy is what lets the planner push the id set into the gate's `UNION`
/// arms instead of materializing the principal's whole visible set (13× on production, and the
/// reason the redundant-looking predicate must not be deleted).
///
/// **That predicate is logically implied, so it cannot change the answer — this test is what makes
/// "cannot" a measurement rather than an argument.** The sibling above asks a batch of one that is
/// wholly invisible, which a gate that had fallen open would fail; this asks a batch where the
/// principal may see *some* ids, which is the case the pushed predicate actually creates and the
/// one a per-row mistake would show up in. A gate that returned its input unfiltered, or that
/// dropped the visible row along with the invisible one, both pass the single-id test and fail
/// here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_redundant_gate_predicate_does_not_change_who_can_see_what(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;

    let owner_home = ctx(&pool, owner, "owners-private").await;
    let theirs_only = ResourceId::from(
        mk(
            &pool,
            owner_home,
            owner,
            emitter,
            "Mews",
            "Where the hawk is kept between flights.",
        )
        .await,
    );

    // A second principal with a home of their own, so the batch below spans both sides of the
    // gate rather than being uniformly visible or uniformly not.
    let stranger = ProfileId::from(common::insert_profile(&pool, "mixed-batch-stranger").await);
    let stranger_home = ctx(&pool, stranger, "strangers-private").await;
    let mine = ResourceId::from(
        mk(
            &pool,
            stranger_home,
            stranger,
            emitter,
            "Jesses",
            "The straps by which a hawk is held.",
        )
        .await,
    );

    let views = temper_substrate::readback::hit_identities(&pool, stranger, &[theirs_only, mine])
        .await
        .unwrap();

    let ids: Vec<ResourceId> = views.iter().map(|v| v.id).collect();
    assert_eq!(
        ids,
        vec![mine],
        "asked for two ids, the stranger is enriched exactly the one they own — the other is \
         absent, not empty-shaped: {:?}",
        views.iter().map(|v| &v.title).collect::<Vec<_>>()
    );
}

/// LAYER 1 WITNESS — the readability conjunct on each arm, isolated from the Rust pre-check.
///
/// An anchor you cannot READ scopes nothing, even when the resources homed in it are visible to you
/// by some other path. `resources_visible_to` does NOT imply map-readability: here the principal
/// OWNS the homed resource, so the row stays visible, and the unscoped assertions below prove that
/// empirically rather than assuming it. Only `cogmap_readable_by_profile` can empty the scoped
/// result — which is exactly the conjunct `cogmap_scope_ids` carried and this migration's first
/// draft dropped when it inlined the anchor pair.
///
/// This lives at the SQL layer ON PURPOSE. `resolve_search_anchor` now refuses an unreadable cogmap
/// before either function is called, so no request-level test can reach these predicates with an
/// unreadable anchor — an HTTP test would witness the pre-check and leave the SQL unwitnessed.
/// Deleting the Rust pre-check does not affect this test; deleting either arm's conjunct fails it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unreadable_cogmap_anchor_scopes_nothing_in_either_arm(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    // Genesis and NO team join: `cogmap_readable_by_profile` is false for everyone, including the
    // profile that owns the map's contents.
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "Sealed Map", "Know the birds").await;

    let homed = mk_at(
        &pool,
        AnchorRef::cogmap(CogmapId::from(cogmap)),
        owner,
        emitter,
        "Sealed",
        "The gyrfalcon is distilled here.",
    )
    .await;

    let unreadable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(owner.uuid())
        .bind(cogmap)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !unreadable,
        "precondition: a genesis'd map joined to no team is readable by nobody"
    );

    // Precondition, and the whole reason this bites: the resource IS visible to this principal
    // (they own it), so an unscoped search finds it. Anything the scoped search then withholds is
    // withheld by readability alone.
    let unscoped: Vec<Uuid> = exact(&pool, owner, "gyrfalcon", None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        unscoped.contains(&homed),
        "precondition: the owner must see their own resource unscoped, or this test proves nothing"
    );

    let scoped: Vec<Uuid> = exact(&pool, owner, "gyrfalcon", Some(("kb_cogmaps", cogmap)))
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        scoped.is_empty(),
        "search_exact must scope to nothing through an unreadable cogmap anchor; got {scoped:?}"
    );
}

/// LAYER 1 WITNESS, wide arm. Same claim as the exact-arm witness above, against the guard clause
/// in `search_wide`'s scoped branch — a separate predicate in a separate function, so it needs its
/// own witness rather than riding on the exact arm's.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_wide_returns_nothing_through_an_unreadable_cogmap_anchor(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "Sealed Wide", "Know the birds").await;

    let axis = unit(7);
    let homed = mk_embedded(
        &pool,
        AnchorRef::cogmap(CogmapId::from(cogmap)),
        owner,
        emitter,
        "SealedWide",
        axis.clone(),
    )
    .await;

    // Unscoped first: the owner sees their own chunk, so the vector arm can reach it at all.
    let unscoped: Vec<Uuid> = wide(&pool, owner, &axis, 50, None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        unscoped.contains(&homed),
        "precondition: the owner must reach their own embedded resource unscoped"
    );

    let scoped: Vec<Uuid> = wide(&pool, owner, &axis, 50, Some(("kb_cogmaps", cogmap)))
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        scoped.is_empty(),
        "search_wide must return nothing through an unreadable cogmap anchor; got {scoped:?}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════════════════════════
// RE-HOMED FROM `search_graph_expand.rs`
//
// Everything below asserts a property that was witnessed against `search_fts_candidates`,
// `search_vector_candidates` or `unified_search` before those were dropped, and that the two arms
// carry forward verbatim. Each is re-pointed at the surviving function rather than deleted with the
// one that used to host it. See this file's header for the two that could NOT be re-homed.
// ═════════════════════════════════════════════════════════════════════════════════════════════════

/// The `ef_search` pin is FUNCTION-scoped, not a session or database default.
///
/// The sibling half of `search_wide_pins_ef_search_at_or_above_the_k_it_is_asked_for`: that one says
/// the pin is big enough, this one says it is contained. A pin that leaked to the session would
/// re-plan every other ANN caller in the same backend — including ones written after this decision —
/// which is the reason migration `20260805000020` uses `ALTER FUNCTION … SET` rather than
/// `ALTER DATABASE … SET`. Re-homed from `search_graph_expand.rs`'s
/// `ef_search_pin_is_function_scoped_not_session_wide`, which read the pin off
/// `search_vector_candidates`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_ef_search_pin_on_search_wide_does_not_leak_to_the_session(pool: sqlx::PgPool) {
    // BOTH statements must run on ONE connection. pgvector registers its GUCs in `_PG_init`, which
    // runs per backend on first use of a vector type, so `current_setting` raises "unrecognized
    // configuration parameter" on any connection that has not yet touched a vector. Against the pool
    // these land on different backends and the test fails for a reason unrelated to its claim —
    // which it did, on the original's first run.
    let mut conn = pool.acquire().await.unwrap();
    let _: f64 = sqlx::query_scalar("SELECT '[1,2]'::vector <=> '[1,3]'::vector")
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    let session: String = sqlx::query_scalar("SELECT current_setting('hnsw.ef_search')")
        .fetch_one(&mut *conn)
        .await
        .unwrap();

    assert_eq!(
        session, "40",
        "the session must keep the server default; a pin that leaked to the session would \
         silently re-plan every other ANN caller"
    );

    let cfg: Option<Vec<String>> =
        sqlx::query_scalar("SELECT proconfig FROM pg_proc WHERE proname = 'search_wide'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let pinned: i64 = cfg
        .unwrap_or_default()
        .iter()
        .find_map(|e| e.strip_prefix("hnsw.ef_search=").map(str::to_owned))
        .expect("hnsw.ef_search must be pinned on search_wide")
        .parse()
        .expect("ef_search pin is numeric");
    assert!(
        pinned > session.parse::<i64>().unwrap(),
        "the function-scoped pin ({pinned}) must actually exceed the session default ({session}), \
         or this test would pass vacuously against an unpinned function"
    );
}

/// `search_wide`'s unscoped draw must reach `idx_kb_chunks_embedding`.
///
/// The whole point of the over-fetch shape is that the ANN is an index scan and admission lands
/// after it. Re-homed unchanged from `search_graph_expand.rs`'s `vector_ann_uses_hnsw_index` — the
/// index and the ORDER BY shape are the subject, and both outlived `search_vector_candidates`.
///
/// `SET LOCAL enable_seqscan = off` because on a small seeded corpus Postgres prefers a seq-scan on
/// cost grounds. The index must still be *usable* — that is what is guarded — so forcing seqscan off
/// is a valid probe: if the index were absent or broken, the plan would show something other than
/// `idx_kb_chunks_embedding` even with seqscan off.
///
/// **The `c.embedding IS NOT NULL` guard that `43f864e8` added to the real `ann` CTE is
/// deliberately NOT in this query**, so the probe stays the one already known green. The migration
/// claims that guard cannot cost a scan and MEASURED it at 20k embedded + 50 unembedded chunks
/// ("THE HNSW INDEX IS NOT DEFEATED … the partial index is `WHERE is_current`, which the guarded
/// predicate still implies, and pgvector's HNSW never indexes a NULL vector"). A five-chunk
/// ephemeral corpus cannot witness a claim measured at 20k — it would only be re-testing the
/// planner's tie-breaking on a corpus production never sees, which is the failure mode
/// `vec_norm_does_not_pay_for_chunk_count` already paid for once. That measurement stands on the
/// migration's own evidence and is NOT witnessed here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_wide_arms_ann_draw_uses_the_hnsw_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ann").await;
    for i in 0..5 {
        mk_embedded(
            &pool,
            AnchorRef::context(home),
            owner,
            emitter,
            &format!("e{i}"),
            unit(i),
        )
        .await;
    }

    // Must run inside a transaction so `SET LOCAL` scopes correctly.
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
        "the wide arm's ANN draw must use the HNSW index; plan was:\n{text}"
    );
}

/// Visibility is a POST-ANN filter on the unscoped branch, and the over-fetch survives it.
///
/// The nearest chunk may belong to a resource the principal cannot see. With `k = 100 » limit` it
/// sits inside the draw, gets pulled by the index `ORDER BY`, and is then dropped by the `admitted`
/// CTE's `resources_visible_to` join — while a farther-but-visible resource still survives. This is
/// the asymmetry the migration carries forward deliberately ("the unscoped branch applies `LIMIT
/// p_k` inside `ann` and lands visibility/active/complete AFTER it, because applying them inside
/// forces a seq-scan and defeats idx_kb_chunks_embedding"). Re-homed from
/// `vector_over_fetch_survives_visibility_drop`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_wide_arm_drops_a_nearer_invisible_resource_after_the_ann(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "postann").await;

    // Visible (caller-owned) and NEAR but not identical: dims 0 and 1 set ⇒ cosine distance ≈ 0.293
    // from unit(0).
    let mut visible_emb = vec![0.0_f32; 768];
    visible_emb[0] = 1.0;
    visible_emb[1] = 1.0;
    let visible = mk_embedded(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "visible",
        visible_emb,
    )
    .await;

    // A SECOND owner whose resource is NOT visible to `owner`, embedded IDENTICALLY to the query
    // (distance 0, i.e. the top ANN hit). A pre-filter ANN would have surfaced it first.
    let stranger = ProfileId::from(common::insert_profile(&pool, "stranger").await);
    let stranger_entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'stranger', '{}'::jsonb) RETURNING id",
    )
    .bind(stranger.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let stranger_home = ctx(&pool, stranger, "postann-stranger").await;
    let hidden = mk_embedded(
        &pool,
        AnchorRef::context(stranger_home),
        stranger,
        EntityId::from(stranger_entity),
        "hidden",
        unit(0),
    )
    .await;

    let ids: Vec<Uuid> = wide(&pool, owner, &unit(0), 100, None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        ids.contains(&visible),
        "the farther-but-visible resource survives the post-ANN visibility join"
    );
    assert!(
        !ids.contains(&hidden),
        "the nearer non-visible resource (the top ANN hit) is dropped by the post-ANN visibility \
         join"
    );
}

/// An ANCHOR-SCOPED wide search is exhaustive where an unscoped one is approximate.
///
/// The scoped branch carries no top-k at all — it pre-filters into `scoped_res` and aggregates over
/// every current embedded chunk of every scoped resource — so a resource that a global top-k starves
/// is recovered by naming its anchor. `k = 1` is the sharpest form of the production `k = 100`
/// starvation: the globally-nearest resource lives in another context, so the global top-1 excludes
/// the target the scoped caller actually wants.
///
/// The migration names this as a change of ALGORITHM rather than of filter ("scoped is exhaustive,
/// unscoped is approximate — so adding a scope can surface resources an unscoped search structurally
/// cannot. Carried forward deliberately and unchanged"), which is exactly why it needs a witness on
/// the new function and not just a comment. Re-homed from
/// `vector_candidates_scope_beats_global_topk_starvation`, whose scoping used `p_context_id` and the
/// now-absent `p_scope_ids` allowlist.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_anchor_scoped_wide_search_recovers_what_the_global_top_k_starves(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let query = unit(0);

    // Target context: one resource embedded FAR from the query — never globally top-1.
    let target_ctx = ctx(&pool, owner, "target").await;
    let target = mk_embedded(
        &pool,
        AnchorRef::context(target_ctx),
        owner,
        emitter,
        "target",
        unit(5),
    )
    .await;

    // Noise context: a resource whose embedding IS the query — wins any global top-k.
    let noise_ctx = ctx(&pool, owner, "noise").await;
    let noise = mk_embedded(
        &pool,
        AnchorRef::context(noise_ctx),
        owner,
        emitter,
        "noise",
        unit(0),
    )
    .await;

    let global: Vec<Uuid> = wide(&pool, owner, &query, 1, None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        global.contains(&noise),
        "precondition: the global top-1 is the noise resource"
    );
    assert!(
        !global.contains(&target),
        "precondition: the global top-1 starves the target context — otherwise the recovery below \
         proves nothing"
    );

    let scoped: Vec<Uuid> = wide(
        &pool,
        owner,
        &query,
        1,
        Some(("kb_contexts", target_ctx.uuid())),
    )
    .await
    .into_iter()
    .map(|(id, _)| id)
    .collect();
    assert!(
        scoped.contains(&target),
        "the anchor-scoped branch carries no top-k, so it returns the in-scope resource despite the \
         global k of 1"
    );
    assert!(
        !scoped.contains(&noise),
        "the anchor pair excludes the other context's resource"
    );
}

/// `vec_norm` must not pay for chunk count.
///
/// Best-of-N is an order statistic, so volume alone buys score that content did not earn. `long` is
/// *better on its single best chunk* than `short` (distance 0.28 vs 0.30) and overwhelmingly worse
/// everywhere else (19 chunks at 0.60). Under a plain `1 - MIN/2`, `long` scores 0.860 against
/// `short`'s 0.850 and wins on volume; under the shrunk statistic `search_wide` carries
/// (`MIN + (AVG - MIN)·(1 - 1/sqrt(count(*)))`) `long` falls to ≈0.742 while `short` is untouched,
/// because the correction is exactly 0 at N = 1.
///
/// Re-homed from `vec_norm_does_not_pay_for_chunk_count`. It also carries the N = 1 identity that
/// `vector_candidates_best_per_resource_normalized` asserted separately: a single-chunk resource is
/// still scored `1 - dist/2`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vec_norm_does_not_pay_for_chunk_count(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "veclen").await;

    let short = mk_chunked(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "short",
        vec![Some(at_cos(0.70))],
    )
    .await;

    let mut many = vec![Some(at_cos(0.72))];
    many.extend(std::iter::repeat_n(Some(at_cos(0.40)), 19));
    let long = mk_chunked(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "long",
        many,
    )
    .await;

    let rows = wide(&pool, owner, &unit(0), 100, None).await;
    let score = |id: Uuid| {
        rows.iter()
            .find(|(r, _)| *r == id)
            .map(|(_, s)| *s)
            .expect("candidate")
    };
    let (s, l) = (score(short), score(long));

    // The N = 1 identity: a single-chunk resource is scored EXACTLY as an unshrunk best-of-N would.
    assert!(
        (s - 0.85).abs() < 1e-4,
        "at N=1 the shrinkage factor is 0, so vec_norm is still 1 - dist/2 = 0.85; got {s}"
    );
    assert!(
        s > l,
        "a resource that is worse on 19 of 20 chunks must not out-score a short one on the strength \
         of its best draw alone: short={s} long={l}"
    );
}

/// The unscoped aggregate is taken over the resource's FULL current embedded chunk set, never over
/// `ann`.
///
/// `ann` is the top-k, so aggregating there conditions on the chunks that WON a slot — and
/// conditioning on winners cannot correct a selection effect when the selection *is* the bias being
/// corrected. Same fixture as above with `p_k = 2`, so the draw admits only `long`'s single best
/// chunk (0.28) and `short`'s only chunk (0.30); `long`'s 19 poor chunks are cut.
///
/// **The bite:** aggregating over `ann` gives `long` n = 1, where the shrinkage factor is 0 AND
/// `AVG` equals `MIN` identically — the correction is a no-op twice over and `long` keeps its
/// unpenalized 0.86 against `short`'s 0.85. This is the case the `k = 100` witness above cannot see,
/// because there every chunk is admitted and nothing is ever cut. Re-homed from
/// `vec_norm_aggregates_over_full_chunk_set_not_the_admitted_slice`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn vec_norm_aggregates_over_the_full_chunk_set_not_the_admitted_slice(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "vectopk").await;

    let short = mk_chunked(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "short",
        vec![Some(at_cos(0.70))],
    )
    .await;

    let mut many = vec![Some(at_cos(0.72))];
    many.extend(std::iter::repeat_n(Some(at_cos(0.40)), 19));
    let long = mk_chunked(
        &pool,
        AnchorRef::context(home),
        owner,
        emitter,
        "long",
        many,
    )
    .await;

    // k = 2: only long's best chunk and short's single chunk are admitted to the draw.
    let rows = wide(&pool, owner, &unit(0), 2, None).await;
    let score = |id: Uuid| {
        rows.iter()
            .find(|(r, _)| *r == id)
            .map(|(_, s)| *s)
            .expect("admitted by the top-k")
    };
    let (s, l) = (score(short), score(long));

    assert!(
        s > l,
        "the shrinkage must be computed over all 20 of long's chunks, not the 1 that won a top-k \
         slot — otherwise the correction is a no-op on exactly its target case: short={s} long={l}"
    );
}

/// `fts_norm` is LENGTH-normalized: ts_rank flag 33, not flag 32.
///
/// Flag 32 applies `rank/(rank+1)` and no length normalization at all, so two documents containing
/// the query term the same number of times score identically however much unrelated material one of
/// them carries. Flag 33 adds flag 1's `1 + log(document length)` division and separates them by
/// length alone.
///
/// **The bite:** under flag `32` both documents receive the SAME `ts_rank` (one match each, same
/// weight) and `short > long` fails on equality. This is one of the two things migration
/// `20260805000020` warns are silently reverted by re-deriving `search_exact` from an older
/// migration; the other is the phrase parser below. Re-homed from `fts_norm_is_length_normalized`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_norm_is_length_normalized(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "ftslen").await;

    // Both bodies contain "peregrine" exactly once, at the same weight (B). Only their length
    // differs, so only length can separate the two scores.
    let filler: String = (0..300).map(|i| format!("filler{i} ")).collect::<String>();

    let short = mk(&pool, home, owner, emitter, "short", "peregrine stoops").await;
    let long = mk(
        &pool,
        home,
        owner,
        emitter,
        "long",
        &format!("peregrine stoops {filler}"),
    )
    .await;

    let rows = exact(&pool, owner, "peregrine", None).await;
    let score = |id: Uuid| {
        rows.iter()
            .find(|(r, _)| *r == id)
            .map(|(_, s)| *s)
            .expect("both documents carry the term")
    };
    let (s, l) = (score(short), score(long));

    assert!(
        s > l,
        "one match in a short document must out-rank one match buried in a long one; \
         short={s} long={l}"
    );
    assert!(
        s > 0.0 && s < 1.0,
        "flag 33 retains 32's rank/(rank+1), so the score stays in [0,1): got {s}"
    );
}

/// `search_exact` parses with `websearch_to_tsquery`, so a quoted phrase forces adjacency.
///
/// Two resources carry both terms; only the one where they are ADJACENT matches the quoted query,
/// while the unquoted query still surfaces both. The parser is the second thing migration
/// `20260805000020` warns is silently reverted by re-deriving the arm from a pre-`20260801000010`
/// definition — `plainto_tsquery` would return both for the quoted form, and no other assertion in
/// this file could tell. Re-homed from `fts_candidates_supports_quoted_phrase` (issue #356).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_honours_a_quoted_phrase(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "phrase").await;

    let adjacent = mk(
        &pool,
        home,
        owner,
        emitter,
        "Adjacent",
        "The peregrine falcon stoops at terminal velocity.",
    )
    .await;
    let apart = mk(
        &pool,
        home,
        owner,
        emitter,
        "Apart",
        "The peregrine is a bird; a falcon is any of several raptors.",
    )
    .await;

    let unquoted: Vec<Uuid> = exact(&pool, owner, "peregrine falcon", None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        unquoted.contains(&adjacent) && unquoted.contains(&apart),
        "precondition: both documents carry both terms, so the unquoted query returns both"
    );

    let quoted: Vec<Uuid> = exact(&pool, owner, "\"peregrine falcon\"", None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        quoted.contains(&adjacent),
        "the quoted phrase matches the document whose terms are adjacent"
    );
    assert!(
        !quoted.contains(&apart),
        "the quoted phrase must NOT match a document that merely carries both terms apart — \
         plainto_tsquery would return it and websearch_to_tsquery does not"
    );
}

/// An empty query is a term-zero, not a match-everything.
///
/// `search_exact`'s `p_query IS NOT NULL AND p_query <> ''` guard: without it `websearch_to_tsquery`
/// on `''` yields an empty tsquery and the `@@` operator would decide the corpus, which is not a
/// decision this arm may make. Re-homed from `fts_candidates_normalized_and_scoped`'s empty-query
/// arm, which was the only assertion in that test not already covered here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_returns_nothing_for_an_empty_query(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "empty").await;

    let present = mk(
        &pool,
        home,
        owner,
        emitter,
        "Falconry",
        "The peregrine stoops at terminal velocity.",
    )
    .await;
    assert!(
        exact(&pool, owner, "peregrine", None)
            .await
            .iter()
            .any(|(id, _)| *id == present),
        "precondition: the corpus is non-empty and reachable, so an empty result below is the \
         guard's doing and not the fixture's"
    );

    assert!(
        exact(&pool, owner, "", None).await.is_empty(),
        "an empty query yields no candidates"
    );
}

// ── SQL-side paging and the doc_type filter (20260806000020) ─────────────────────────────────────
//
// Both arms used to return their entire match set, ordered and sliced in Rust only AFTER every row
// had been hydrated into a full `ResourceView` — measured at 1,883 rows for a ten-row request
// against a 3,402-resource production corpus. And `p_doc_type`, which `unified_search` applied,
// went with the blend while its CLI flag, `SearchParams` field, MCP parameter and OpenAPI schema
// all stayed. These are the witnesses for both.

/// [`mk`] with a caller-chosen doc type, so a doc_type filter has something to tell apart.
async fn mk_typed(
    pool: &sqlx::PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    body: &str,
    doc_type: &str,
) -> Uuid {
    writes::create_resource(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            sources: vec![],
            title,
            origin_uri: &format!("test://{title}"),
            body,
            doc_type,
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

/// `search_exact` over the full parameter set. `limit: None` is `LIMIT NULL` — unbounded.
async fn exact_full(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    q: &str,
    doc_type: Option<&str>,
    limit: Option<i32>,
    offset: i32,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, fts_norm FROM search_exact($1, $2, NULL, NULL, $3, $4, $5)")
        .bind(principal.uuid())
        .bind(q)
        .bind(doc_type)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm")))
        .collect()
}

/// The exact arm pages in SQL, and the page boundary is total.
///
/// Two claims, and the second is the one that is easy to lose. A `LIMIT` with a partial order lets
/// rows tied on score reshuffle between calls, so a caller walking offsets can see one row twice
/// and another never — which looks like a search bug and is a paging bug. Every resource here
/// carries the SAME body, so every `fts_norm` ties and the `resource_id` tiebreak is the ONLY thing
/// deciding the pages: if it were dropped, this test fails by set inequality rather than by order.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_pages_in_sql_and_the_tiebreak_makes_the_boundary_total(pool: sqlx::PgPool) {
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "paging").await;
    for i in 0..7 {
        mk(
            &pool,
            home,
            owner,
            emitter,
            &format!("r{i}"),
            "quorum quorum",
        )
        .await;
    }

    let all = exact_full(&pool, owner, "quorum", None, None, 0).await;
    assert_eq!(all.len(), 7, "unbounded must return the whole match set");

    // The pages partition the set, in order, with no repeat and no gap.
    let mut walked: Vec<Uuid> = Vec::new();
    for page in 0..4 {
        let rows = exact_full(&pool, owner, "quorum", None, Some(2), page * 2).await;
        walked.extend(rows.into_iter().map(|(id, _)| id));
    }
    let expected: Vec<Uuid> = all.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        walked, expected,
        "walking the arm two at a time must reproduce the unbounded order exactly - a repeat or a \
         gap here means the ORDER BY is not total"
    );

    // Past the end is empty, not an error and not a wrap.
    assert!(
        exact_full(&pool, owner, "quorum", None, Some(2), 100)
            .await
            .is_empty(),
        "an offset past the end yields nothing"
    );
}

/// `p_doc_type` filters the exact arm, and its absence does not.
///
/// The negative half alone would pass against an arm that returned nothing at all, so the unfiltered
/// call is the control that makes the filtered one mean something.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_exact_filters_by_doc_type(pool: sqlx::PgPool) {
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "doctype").await;
    let concept = mk_typed(
        &pool,
        home,
        owner,
        emitter,
        "c",
        "quorum sentinel",
        "concept",
    )
    .await;
    let decision = mk_typed(
        &pool,
        home,
        owner,
        emitter,
        "d",
        "quorum sentinel",
        "decision",
    )
    .await;

    let unfiltered: Vec<Uuid> = exact_full(&pool, owner, "quorum", None, None, 0)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(unfiltered.len(), 2, "control: both types match the term");
    assert!(unfiltered.contains(&concept) && unfiltered.contains(&decision));

    let only_decisions: Vec<Uuid> = exact_full(&pool, owner, "quorum", Some("decision"), None, 0)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        only_decisions,
        vec![decision],
        "a doc_type filter must admit that type and no other"
    );

    assert!(
        exact_full(&pool, owner, "quorum", Some("no-such-type"), None, 0)
            .await
            .is_empty(),
        "an unmatched doc_type filters everything out rather than being ignored - being ignored is \
         the defect this restores"
    );
}

/// `p_doc_type` filters the wide arm on BOTH of its branches.
///
/// The branches place the conjunct in different CTEs — after the top-k on the unscoped branch, so
/// the HNSW draw is not defeated, and with the row predicates on the exhaustive scoped one — so a
/// witness on one says nothing about the other.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn search_wide_filters_by_doc_type_on_both_branches(pool: sqlx::PgPool) {
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "wide-doctype").await;
    // `at_cos` is defined relative to axis 0, so that is the query vector both fixtures sit near.
    let q = unit(0);

    let anchor_ref = AnchorRef::context(home);
    let concept = mk_embedded(&pool, anchor_ref, owner, emitter, "wc", at_cos(0.9)).await;
    let decision = mk_embedded(&pool, anchor_ref, owner, emitter, "wd", at_cos(0.8)).await;
    // `mk_chunked` creates everything as `concept`; retype one so the filter has two kinds to tell
    // apart. Writing the property directly is the narrowest way to get there and leaves the
    // embedding untouched, which is what this arm actually retrieves on.
    sqlx::query(
        "UPDATE kb_properties SET property_value = to_jsonb($2::text)
          WHERE owner_table = 'kb_resources' AND owner_id = $1 AND property_key = 'doc_type'",
    )
    .bind(decision)
    .bind("decision")
    .execute(&pool)
    .await
    .expect("retype the second resource");

    for anchor in [None, Some(("kb_contexts", home.uuid()))] {
        let label = if anchor.is_none() {
            "unscoped"
        } else {
            "scoped"
        };

        let both: Vec<Uuid> = wide_full(&pool, owner, &q, 100, anchor, None)
            .await
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert!(
            both.contains(&concept) && both.contains(&decision),
            "{label}: control - both resources are in reach before any filter"
        );

        let filtered: Vec<Uuid> = wide_full(&pool, owner, &q, 100, anchor, Some("decision"))
            .await
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            filtered,
            vec![decision],
            "{label}: the doc_type filter must apply on this branch too"
        );
    }
}

/// `search_wide` over the full parameter set.
async fn wide_full(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    emb: &[f32],
    k: i32,
    anchor: Option<(&str, Uuid)>,
    doc_type: Option<&str>,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    let (table, id) = match anchor {
        Some((t, i)) => (Some(t), Some(i)),
        None => (None, None),
    };
    sqlx::query(
        "SELECT resource_id, vec_norm FROM search_wide($1, $2::vector, $3, $4, $5, $6, NULL, 0)",
    )
    .bind(principal.uuid())
    .bind(vlit(emb))
    .bind(k)
    .bind(table)
    .bind(id)
    .bind(doc_type)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm")))
    .collect()
}

// ─── The composable twins ───────────────────────────────────────────────────────────────────────
//
// `query_find_exact` / `query_find_wide` are the deployed arms with one extra parameter,
// `p_bound_ids uuid[]` — the same currency `search_graph_expand.p_seed_ids` already speaks, and the
// thing `/api/query` needs in order to narrow a find stage by what an upstream stage produced. The
// incumbents delegate to them with `p_bound_ids => NULL`, so there is ONE BODY PER ARM rather than
// two that can drift.

/// Rows from `query_find_exact`, as (id, fts_norm). `bound` of `None` is unbounded (SQL NULL).
async fn find_exact(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    q: &str,
    bound: Option<Vec<Uuid>>,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query(
        "SELECT resource_id, fts_norm FROM query_find_exact($1, $2, $3, NULL, NULL, NULL, NULL, 0)",
    )
    .bind(principal.uuid())
    .bind(q)
    .bind(bound)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm")))
    .collect()
}

/// Rows from `query_find_wide`, as (id, vec_norm).
async fn find_wide(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    emb: &[f32],
    k: i32,
    bound: Option<Vec<Uuid>>,
) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    // Nine parameters: principal, emb, k, bound_ids, anchor_table, anchor_id, doc_type, limit,
    // offset. Written out rather than leaning on DEFAULTs — passing eight silently bound `0` to
    // `p_limit` instead of `p_offset`, giving `LIMIT 0`, and the empty-bound assertion then passed
    // for entirely the wrong reason.
    sqlx::query(
        "SELECT resource_id, vec_norm \
           FROM query_find_wide($1, $2::vector, $3, $4, NULL, NULL, NULL, NULL, 0)",
    )
    .bind(principal.uuid())
    .bind(vlit(emb))
    .bind(k)
    .bind(bound)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("vec_norm")))
    .collect()
}

/// A bound set NARROWS: only ids in it come back, and it does not invent rows.
///
/// Fails against a twin that accepts `p_bound_ids` and ignores it — which is the whole failure mode,
/// since such a twin returns a superset that still contains everything the caller asked for and so
/// passes any assertion phrased only as "the expected rows are present".
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_bound_set_narrows_both_find_arms_to_its_members(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "bounded-arms").await;

    let kestrel = mk(
        &pool,
        home,
        owner,
        emitter,
        "Kestrel",
        "The kestrel hovers over the verge.",
    )
    .await;
    let merlin = mk(
        &pool,
        home,
        owner,
        emitter,
        "Merlin",
        "The merlin hunts the kestrel's ground.",
    )
    .await;
    let hobby = mk(
        &pool,
        home,
        owner,
        emitter,
        "Hobby",
        "The hobby takes kestrel-sized prey.",
    )
    .await;

    let unbounded: Vec<Uuid> = find_exact(&pool, owner, "kestrel", None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        unbounded.len(),
        3,
        "denominator check: all three resources must match unbounded, or narrowing to one proves \
         nothing"
    );

    let bounded: Vec<Uuid> = find_exact(&pool, owner, "kestrel", Some(vec![merlin]))
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        bounded,
        vec![merlin],
        "a bound set must narrow to exactly its members; kestrel={kestrel} hobby={hobby}"
    );
}

/// **Empty is not absent.** `'{}'` means bounded-to-nothing ⇒ zero rows; only NULL means unbounded.
///
/// Two assertions, deliberately not collapsed into one. An upstream stage that produced nothing must
/// never silently become an unbounded search — "an empty upstream is a disclosed disposition, never
/// a substitution." A twin that treats `'{}'` as "no bound supplied" (the natural implementation, if
/// the conjunct is written `cardinality(p_bound_ids) = 0 OR ...`) fails the first assertion while
/// passing the second, which is why both are here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_empty_bound_set_returns_nothing_while_null_is_unbounded(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "empty-vs-null").await;

    mk(
        &pool,
        home,
        owner,
        emitter,
        "Kestrel",
        "The kestrel hovers over the verge.",
    )
    .await;
    mk(
        &pool,
        home,
        owner,
        emitter,
        "Merlin",
        "The merlin hunts the kestrel's ground.",
    )
    .await;

    let empty = find_exact(&pool, owner, "kestrel", Some(vec![])).await;
    assert!(
        empty.is_empty(),
        "p_bound_ids = '{{}}' means bounded to NOTHING and must return zero rows, got {empty:?}"
    );

    let null = find_exact(&pool, owner, "kestrel", None).await;
    assert_eq!(
        null.len(),
        2,
        "p_bound_ids => NULL is unbounded and must return the full match set, got {null:?}"
    );

    // The same distinction on the wide arm, where getting it wrong is worse: an empty upstream
    // silently becoming unbounded would run a global ANN draw the caller never asked for.
    let e = unit(0);
    let empty_wide = find_wide(&pool, owner, &e, 10, Some(vec![])).await;
    assert!(
        empty_wide.is_empty(),
        "an empty bound set must not widen into a global draw, got {empty_wide:?}"
    );
}

/// **A bound set is a SCOPE, so it selects the exhaustive branch.**
///
/// The load-bearing witness of this migration. `search_wide`'s unscoped branch draws a global top-k
/// BEFORE the visibility gate and before any filter, so a bound applied to its output would filter
/// after truncation — the wide-then-filter defect, a correctness rule rather than a tuning choice.
/// The twin avoids it structurally by taking the branch that has no truncation to defeat.
///
/// Fails against a twin that adds `p_bound_ids` as a post-filter on the top-k branch: `far` sits
/// outside a k=3 draw crowded by the near cluster, so a post-filtering implementation returns
/// nothing while this one returns `far`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_bounded_wide_call_returns_what_a_top_k_would_have_crowded_out(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "crowded-out").await;
    let anchor = AnchorRef {
        table: temper_substrate::payloads::AnchorTable::Contexts,
        id: home.uuid(),
    };

    // A dense cluster at the query vector, then one resource far from it.
    for i in 0..8 {
        mk_embedded(
            &pool,
            anchor,
            owner,
            emitter,
            &format!("Near {i}"),
            at_cos(0.99),
        )
        .await;
    }
    let far = mk_embedded(&pool, anchor, owner, emitter, "Far", at_cos(0.10)).await;

    let q = unit(0);

    // First establish that k genuinely binds — without this the test is vacuous, because `far`
    // could be absent from the unbounded result for reasons other than truncation.
    let unbounded: Vec<Uuid> = find_wide(&pool, owner, &q, 3, None)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        !unbounded.contains(&far),
        "k=3 against an 8-strong near cluster must crowd `far` out, or the witness below proves \
         nothing; got {unbounded:?}"
    );

    let bounded: Vec<Uuid> = find_wide(&pool, owner, &q, 3, Some(vec![far]))
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        bounded,
        vec![far],
        "a bound set must select the EXHAUSTIVE branch, so a resource the top-k crowded out is \
         still reachable when the caller names it"
    );
}

/// The incumbents are unchanged by becoming delegating wrappers.
///
/// `search_exact` / `search_wide` must return exactly what a directly-called twin with NULL bounds
/// returns — that equality is what "one body per arm" MEANS, and it is the thing that silently stops
/// being true if a later edit touches one body and not the other.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_incumbents_agree_with_their_twins_at_null_bounds(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "delegation").await;
    let anchor = AnchorRef {
        table: temper_substrate::payloads::AnchorTable::Contexts,
        id: home.uuid(),
    };

    mk(
        &pool,
        home,
        owner,
        emitter,
        "Kestrel",
        "The kestrel hovers over the verge.",
    )
    .await;
    mk(
        &pool,
        home,
        owner,
        emitter,
        "Merlin",
        "The merlin hunts the kestrel's ground.",
    )
    .await;
    mk_embedded(&pool, anchor, owner, emitter, "Near", at_cos(0.95)).await;
    mk_embedded(&pool, anchor, owner, emitter, "Mid", at_cos(0.50)).await;

    let via_incumbent = exact(&pool, owner, "kestrel", None).await;
    let via_twin = find_exact(&pool, owner, "kestrel", None).await;
    assert_eq!(
        via_incumbent, via_twin,
        "search_exact must be query_find_exact at NULL bounds — same rows, same scores, same order"
    );
    assert!(
        !via_incumbent.is_empty(),
        "denominator: the comparison must span real rows"
    );

    let q = unit(0);
    let wide_incumbent = wide(&pool, owner, &q, 10, None).await;
    let wide_twin = find_wide(&pool, owner, &q, 10, None).await;
    assert_eq!(
        wide_incumbent, wide_twin,
        "search_wide must be query_find_wide at NULL bounds — same rows, same scores, same order"
    );
    assert!(
        !wide_incumbent.is_empty(),
        "denominator: the comparison must span real rows"
    );
}

/// Every function that draws ANN candidates pins `hnsw.ef_search` at or above the k it is asked for.
///
/// **Rederived over a SET rather than named at one function.** The incumbent test reads
/// `WHERE proname = 'search_wide'`, which cannot see a new door: `pg_proc.proconfig` binds to a
/// SIGNATURE, so `query_find_wide` inherits nothing from `search_wide` and would draw at the server
/// default of 40 — below any k a caller passes — truncating silently with no error. `/api/search`
/// stays safe because a pin on a wrapper does reach a nested call; `/api/query` calls the twin
/// DIRECTLY, which is exactly the door the incumbent test does not watch.
///
/// The set is DERIVED from the catalog (function bodies containing the pgvector distance operator
/// under an ORDER BY … LIMIT) rather than listed, so a future ANN-drawing function is covered
/// without editing this test — a hand-maintained list would rot, which is the repo's own lesson.
///
/// Asserted as `pin >= k`, never `pin == 200`: a value assertion pins a number nobody may tune,
/// while the invariant is that the candidate list is at least as large as what the caller asks for.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_ann_drawing_function_pins_ef_search_at_or_above_its_k(pool: sqlx::PgPool) {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT p.proname, p.proconfig
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
          WHERE n.nspname = 'public'
            AND p.prosrc LIKE '%<=>%'
            AND p.prosrc LIKE '%LIMIT p_k%'
          ORDER BY p.proname",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    // Vacuity guard with a named member. The derived set is what gives COVERAGE of a function added
    // later; naming one known member is what stops the test passing because the derivation silently
    // stopped matching anything. Note the expected count is ONE, not two: after the delegation
    // `search_wide` no longer contains an ANN body at all — one body per arm is exactly why.
    let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>("proname")).collect();
    assert!(
        names.iter().any(|n| n == "query_find_wide"),
        "the derivation must find query_find_wide, which demonstrably draws ANN candidates; it \
         found {names:?} — if that list is empty the predicate stopped matching and every \
         assertion below is vacuous"
    );

    let asked_for: i64 = 100;
    for r in &rows {
        let name: String = r.get("proname");
        let cfg: Option<Vec<String>> = r.get("proconfig");
        let pinned: i64 = cfg
            .unwrap_or_default()
            .iter()
            .find_map(|e| e.strip_prefix("hnsw.ef_search=").map(str::to_owned))
            .unwrap_or_else(|| {
                panic!(
                    "{name} draws ANN candidates but pins no hnsw.ef_search on itself; proconfig \
                     binds to a signature and does not inherit, so it will draw at the server \
                     default and truncate silently"
                )
            })
            .parse()
            .expect("ef_search pin is numeric");
        assert!(
            pinned >= asked_for,
            "{name} pins hnsw.ef_search at {pinned}, below the k it is called with ({asked_for}); \
             the ANN cannot return the k rows requested and admission truncates silently"
        );
    }
}
