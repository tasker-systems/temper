#![cfg(feature = "artifact-tests")]
//! Phase 1 — `/api/search` as two arms that are never combined: `search_exact` (you can quote the
//! words) and `search_wide` (you have the idea, not the words). Each returns its own quantity under
//! its own name and nothing sums them.
//!
//! Under task `019fd25e-95f0-7373-9a6e-0574deea5ab3` and decision
//! `019fd25a-ef4c-7473-b72e-265a7d36dd65`. Isolated ephemeral DB via `MIGRATOR`, as
//! `search_surface_a.rs` — which tests the blend these two replace.

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

/// A single-chunk resource carrying `emb`. The wide arm scores chunks, so these need embeddings
/// where the exact arm's needed only a body.
async fn mk_embedded(
    pool: &sqlx::PgPool,
    home: AnchorRef,
    owner: ProfileId,
    emitter: EntityId,
    title: &str,
    emb: Vec<f32>,
) -> Uuid {
    let blocks = vec![PreparedBlock {
        incorporated: vec![],
        raw_text: None,
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: title.to_string(),
            embedding: Some(emb),
            embedded_with: None,
            header_path: None,
            heading_depth: None,
        }],
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
