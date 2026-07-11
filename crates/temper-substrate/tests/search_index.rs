#![cfg(feature = "artifact-tests")]
//! Search Beat 1 — the stored `kb_resource_search_index` is populated and maintained by the
//! event-sourced projection functions (create / block-edit / title-only update), and backfilled.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::CogmapId;
use temper_substrate::payloads::AnchorRef;
use temper_substrate::readback;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

/// The boot-seeded canonical `system` profile + entity.
async fn system_actor(
    pool: &sqlx::PgPool,
) -> (
    temper_substrate::ids::ProfileId,
    temper_substrate::ids::EntityId,
) {
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
    (
        temper_substrate::ids::ProfileId::from(profile),
        temper_substrate::ids::EntityId::from(entity),
    )
}

async fn ctx(
    pool: &sqlx::PgPool,
    owner: temper_substrate::ids::ProfileId,
    slug: &str,
) -> temper_substrate::ids::ContextId {
    temper_substrate::ids::ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// Does the stored vector match a query term? (`@@ plainto_tsquery`).
async fn index_matches(pool: &sqlx::PgPool, resource: Uuid, term: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE((SELECT search_vector @@ plainto_tsquery('english', $2)
           FROM kb_resource_search_index WHERE resource_id = $1), false)",
    )
    .bind(resource)
    .bind(term)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The stored vector's `ts_rank` for a query term (0.0 when the resource has no index row / no match).
async fn index_rank(pool: &sqlx::PgPool, resource: Uuid, term: &str) -> f32 {
    sqlx::query_scalar::<_, f32>(
        "SELECT COALESCE((SELECT ts_rank(search_vector, plainto_tsquery('english', $2))
           FROM kb_resource_search_index WHERE resource_id = $1), 0::real)",
    )
    .bind(resource)
    .bind(term)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_populates_index_with_title_and_body(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Salamander architecture",
            origin_uri: "temper://idx/r",
            body: "the quenching pipeline tempers steel",
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
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "salamander").await,
        "title term indexed (weight A)"
    );
    assert!(
        index_matches(&pool, r.uuid(), "quenching").await,
        "body term indexed (weight B)"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "unrelated").await,
        "non-present term does not match"
    );
}

// The async-embed round-trip (issue #299): a DEFERRED create persists chunk text + FTS immediately
// but leaves every chunk's vector NULL; the drain's per-resource backfill then fills them. This is the
// end-to-end proof that "FTS immediate, vector eventual" holds at the write/read boundary.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn deferred_create_is_fts_immediate_then_backfills_vectors(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "def").await;
    let r = writes::create_resource_deferred_with(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Deferred doc",
            origin_uri: "temper://def/r",
            body: "the async embedding pipeline defers vectors off the request path",
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
        EventContext::default(),
    )
    .await
    .unwrap();

    // FTS is immediate — the body term is indexed even with no vector yet.
    assert!(
        index_matches(&pool, r.uuid(), "embedding").await,
        "deferred create is FTS-searchable immediately"
    );

    // Every current chunk exists with a NULL embedding (text persisted, vector deferred).
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id=$1 AND is_current")
            .bind(r.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    let null_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id=$1 AND is_current AND embedding IS NULL",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(total >= 1, "deferred create writes chunks");
    assert_eq!(
        null_before, total,
        "no chunk is embedded yet (all deferred)"
    );

    // The drain's per-resource backfill embeds every deferred chunk.
    let embedded = temper_substrate::embed::embed_resource_chunks(&pool, r.uuid())
        .await
        .unwrap();
    assert_eq!(
        embedded, total as u64,
        "backfill embeds every deferred chunk"
    );
    let null_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id=$1 AND is_current AND embedding IS NULL",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(null_after, 0, "all deferred chunks now carry a vector");

    // Idempotent: a second backfill has nothing to do.
    assert_eq!(
        temper_substrate::embed::embed_resource_chunks(&pool, r.uuid())
            .await
            .unwrap(),
        0,
        "re-running the backfill embeds nothing"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn body_edit_updates_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Doc",
            origin_uri: "temper://idx/r",
            body: "original lexeme here",
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
    .unwrap();
    assert!(
        index_matches(&pool, r.uuid(), "original").await,
        "pre-edit body term present"
    );

    writes::update_resource(
        &pool,
        writes::UpdateParams {
            sources: vec![],
            resource: r,
            body: Some("revised distinctive wording"),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "distinctive").await,
        "new body term indexed after edit"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "original").await,
        "superseded body term gone from index"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn title_only_update_updates_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Aardvark",
            origin_uri: "temper://idx/r",
            body: "stable body",
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
    .unwrap();

    writes::update_resource(
        &pool,
        writes::UpdateParams {
            sources: vec![],
            resource: r,
            body: None,
            title: Some("Pangolin"),
            origin_uri: None,
            properties: &[],
            chunks: None,
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "pangolin").await,
        "new title term indexed (title-only update)"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "aardvark").await,
        "old title term gone"
    );
    assert!(
        index_matches(&pool, r.uuid(), "stable").await,
        "body unchanged still indexed"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn backfill_covers_preexisting_rows(pool: sqlx::PgPool) {
    // The migration's backfill (20260626000001) runs after 20260625000001, which seeds the L0 kernel
    // telos resource. This test asserts that both the pre-existing telos resource AND a freshly
    // created resource have index rows — i.e. the backfill covered all active resources, not just
    // those created after the migration ran.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Backfillable",
            origin_uri: "temper://idx/r",
            body: "corpus content word",
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
    .unwrap();

    let missing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources r
          WHERE r.is_active
            AND NOT EXISTS (SELECT 1 FROM kb_resource_search_index si WHERE si.resource_id = r.id)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(missing, 0, "every active resource has an index row");
    assert!(
        index_matches(&pool, r.uuid(), "corpus").await,
        "backfilled/maintained term matches"
    );
}

/// Soft-deleted resources are excluded from `fts_search` via the `WHERE r.is_active` clause added
/// in Beat 1. The index ROW persists (the delete-trigger does not remove it); the active-resource
/// JOIN filters it out at read time.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn soft_deleted_resource_excluded_from_fts_search(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "del").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Phlogiston study",
            origin_uri: "temper://del/r",
            body: "phlogiston theory explains combustion",
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
    .unwrap();

    // Pre-delete: resource is found by FTS.
    let before = readback::fts_search(&pool, owner, "phlogiston")
        .await
        .unwrap();
    assert!(
        before.contains(&r),
        "resource must appear in FTS results before soft-delete"
    );

    // Soft-delete via writes surface (sets kb_resources.is_active = false).
    writes::delete_resource(&pool, r, emitter).await.unwrap();

    // Post-delete: WHERE r.is_active filters the resource out of FTS results.
    let after = readback::fts_search(&pool, owner, "phlogiston")
        .await
        .unwrap();
    assert!(
        !after.contains(&r),
        "soft-deleted resource must be absent from FTS results (WHERE r.is_active)"
    );

    // The index row persists — delete is soft, not cascaded to the search index.
    // The filter happens at read time, not at delete time (stale-row-is-harmless property).
    let index_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_search_index WHERE resource_id = $1")
            .bind(r.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        index_count, 1,
        "index row persists after soft-delete (filtered at read time, not deleted)"
    );
}

/// The stored-index `fts_search` returns the SAME id set as the legacy inline build for title/body
/// terms. The inline query below is the pre-swap recipe verbatim (the behavior-preservation gate).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_search_parity_with_inline_recipe(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "p").await;
    for (t, b, u) in [
        (
            "Quenching furnace",
            "tempering steel at temperature",
            "temper://p/1",
        ),
        (
            "Annealing notes",
            "slow cooling relieves stress",
            "temper://p/2",
        ),
        ("Unrelated doc", "nothing about metal here", "temper://p/3"),
    ] {
        writes::create_resource(
            &pool,
            writes::CreateParams {
                sources: vec![],
                title: t,
                origin_uri: u,
                body: b,
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
        .unwrap();
    }

    // Legacy inline recipe, inlined here as the parity oracle. It deliberately stays on
    // `plainto_tsquery` even though `fts_search` now parses with `websearch_to_tsquery` (issue
    // #356): for the plain unquoted fixture queries below the two parse identically, so this
    // assertion doubles as the backward-compatibility proof — the websearch swap changes nothing
    // for unquoted input. (Quoted-phrase behavior — where they diverge — is covered by
    // `fts_search_supports_quoted_phrase`.)
    async fn inline_fts(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<Uuid> {
        let rows = sqlx::query(
            "WITH doc AS (
               SELECT r.id,
                      setweight(to_tsvector('english', r.title), 'A') ||
                      setweight(to_tsvector('english', COALESCE(string_agg(cc.content, ' '), '')), 'B')
                        AS search_vector
                 FROM kb_resources r
                 JOIN resources_visible_to($1) v ON v.resource_id = r.id
                 LEFT JOIN kb_chunks c ON c.resource_id = r.id AND c.is_current
                 LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
                GROUP BY r.id, r.title)
             SELECT id FROM doc
              WHERE search_vector @@ plainto_tsquery('english', $2)
              ORDER BY ts_rank(search_vector, plainto_tsquery('english', $2)) DESC")
            .bind(principal).bind(q).fetch_all(pool).await.unwrap();
        rows.iter()
            .map(|r| sqlx::Row::get::<Uuid, _>(r, "id"))
            .collect()
    }

    for q in [
        "tempering",
        "cooling",
        "metal",
        "quenching steel",
        "furnace",
    ] {
        let mut want = inline_fts(&pool, owner.uuid(), q).await;
        let mut got: Vec<Uuid> = readback::fts_search(&pool, owner, q)
            .await
            .unwrap()
            .into_iter()
            .map(|id| id.uuid())
            .collect();
        want.sort();
        got.sort();
        assert_eq!(
            got, want,
            "stored-index fts_search set parity for query {q:?}"
        );
    }
}

/// Issue #356: `websearch_to_tsquery` makes exact phrases expressible via `"quotes"`, plus `OR`
/// and `-negation`, while leaving plain unquoted input identical to `plainto_tsquery`.
///
/// Two resources share both terms `quench` and `hardening`, but only one has them ADJACENT.
/// A quoted-phrase query must match only the adjacent one (the whole point — `plainto_tsquery`
/// would have ANDed the terms and matched both). Unquoted input still matches both. Negation
/// (`-term`) excludes a resource carrying the negated term.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_search_supports_quoted_phrase(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "phrase").await;

    let mk = |title: &'static str, body: &'static str, uri: &'static str| {
        let pool = pool.clone();
        async move {
            writes::create_resource(
                &pool,
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
    };

    // Adjacent: "quench hardening" appears verbatim, terms next to each other.
    let adjacent = mk(
        "Doc adjacent",
        "the quench hardening process strengthens steel",
        "temper://phrase/adjacent",
    )
    .await;
    // Split: both terms present but far apart (never adjacent) — and carries the extra term
    // `billet` used by the negation assertion below.
    let split = mk(
        "Doc split",
        "quench the billet, then begin hardening it slowly",
        "temper://phrase/split",
    )
    .await;

    let ids = |hits: Vec<temper_substrate::ids::ResourceId>| -> Vec<Uuid> {
        hits.into_iter().map(|id| id.uuid()).collect()
    };

    // Quoted phrase → only the adjacent resource (the defect this issue fixes).
    let quoted = ids(readback::fts_search(&pool, owner, "\"quench hardening\"")
        .await
        .unwrap());
    assert!(
        quoted.contains(&adjacent),
        "quoted phrase matches the resource with adjacent terms"
    );
    assert!(
        !quoted.contains(&split),
        "quoted phrase EXCLUDES the resource whose terms are non-adjacent (plainto_tsquery would not have)"
    );

    // Unquoted → both (backward-compatible AND semantics, unchanged from plainto_tsquery).
    let unquoted = ids(readback::fts_search(&pool, owner, "quench hardening")
        .await
        .unwrap());
    assert!(
        unquoted.contains(&adjacent) && unquoted.contains(&split),
        "unquoted query still matches both resources (parity with plainto_tsquery)"
    );

    // Negation → `hardening -billet` keeps the adjacent doc, drops the one carrying `billet`.
    let negated = ids(readback::fts_search(&pool, owner, "hardening -billet")
        .await
        .unwrap());
    assert!(
        negated.contains(&adjacent),
        "negation keeps a resource lacking the negated term"
    );
    assert!(
        !negated.contains(&split),
        "negation drops the resource carrying the negated term `billet`"
    );
}

/// A resource can be homed directly to a cogmap (not a context). The L0 reserved cogmap
/// (born by migration 20260625000001_l0_kernel_cogmap.sql) is used as the anchor.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_resource_homes_in_cogmap(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    // The L0 kernel cogmap is born by the MIGRATOR; use its reserved id.
    let cogmap_id =
        CogmapId::from(uuid::Uuid::parse_str("00000000-0000-0000-0005-000000000001").unwrap());
    let id = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "concept",
            origin_uri: "",
            body: "body text",
            doc_type: "note",
            home: AnchorRef::cogmap(cogmap_id),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap();
    let (table, anchor): (String, Uuid) = sqlx::query_as(
        "SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = $1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(table, "kb_cogmaps");
    assert_eq!(anchor, cogmap_id.uuid());
}

/// SQLA audit chunk 3 (docs/code-reviews/2026-07-08-sql-function-audit.md, SQLA-3 /
/// folded-block-leaks-into-fts): a charter supersede folds the old blocks, but their
/// chunks stay `is_current` (the new charter arrives as fresh block ids). The rebuilt
/// search_vector must aggregate only chunks of LIVE blocks — mirroring
/// `_recompute_resource_body_hash` — so superseded charter prose stops matching FTS.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn charter_supersede_removes_superseded_prose_from_fts(pool: sqlx::PgPool) {
    use temper_substrate::content;
    use temper_substrate::events::{fire, SeedAction};

    fn sha256_hex(s: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(s.as_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
    /// One-block charter carrying `prose` as a synthetic, already-embedded chunk (ONNX-free).
    fn charter_of(prose: &str) -> Vec<content::PreparedBlock> {
        let chunk = content::IncomingChunk {
            chunk_index: 0,
            content_hash: sha256_hex(prose),
            content: prose.to_string(),
            embedding: vec![0.1f32; 768],
            header_path: String::new(),
            heading_depth: 0,
        };
        vec![content::prepare_block_from_chunks(
            0,
            Some("statement"),
            vec![chunk],
        )]
    }

    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;

    // Genesis a cogmap with an EMPTY telos (no embed call), then charter it twice.
    let mut conn = pool.acquire().await.unwrap();
    let (cogmap, telos) = fire(
        &mut conn,
        SeedAction::CogmapGenesis {
            name: "fts-supersede-cogmap",
            telos_title: "FTS supersede telos",
            charter: &[],
            cogmap_id: None,
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap()
    .cogmap_genesis()
    .unwrap();
    drop(conn);

    let mut tx = pool.begin().await.unwrap();
    writes::set_charter_in_tx(
        &mut tx,
        cogmap,
        &charter_of("the zirconium doctrine governs this map"),
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(
        index_matches(&pool, telos.uuid(), "zirconium").await,
        "first charter's prose is FTS-matchable"
    );

    // Supersede: new prose folds the zirconium block.
    let mut tx = pool.begin().await.unwrap();
    writes::set_charter_in_tx(
        &mut tx,
        cogmap,
        &charter_of("the vanadium doctrine governs this map"),
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    assert!(
        index_matches(&pool, telos.uuid(), "vanadium").await,
        "superseding charter's prose is FTS-matchable"
    );
    assert!(
        !index_matches(&pool, telos.uuid(), "zirconium").await,
        "superseded (folded) charter prose must no longer match FTS"
    );
}

/// Issue #359 — open_meta `keywords` (weight C) are searchable AND boost ranking.
/// Two resources share title + body verbatim; only one lists the query term as a keyword. The
/// keyword-only term matches ONLY the keyworded resource (searchable), and on a shared body term the
/// keyworded resource ranks strictly higher (the acceptance criterion: keywords lift an otherwise-
/// identical resource). Keywords are set as a create-time property — the projection order (block
/// rebuild BEFORE property_set) means this only passes because property_set now re-folds the vector.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn open_meta_keywords_are_searchable_and_boost_rank(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "kw").await;

    let mk = |uri: &'static str, props: Vec<(String, serde_json::Value)>| {
        let pool = pool.clone();
        async move {
            writes::create_resource(
                &pool,
                writes::CreateParams {
                    sources: vec![],
                    title: "Ocean layers",
                    origin_uri: uri,
                    body: "the thermocline separates warm surface water from the cold deep",
                    doc_type: "concept",
                    home: AnchorRef::context(home),
                    owner,
                    originator: owner,
                    emitter,
                    properties: &props,
                    chunks: None,
                },
            )
            .await
            .unwrap()
            .uuid()
        }
    };

    let plain = mk("temper://kw/plain", vec![]).await;
    let withkw = mk(
        "temper://kw/withkw",
        vec![(
            "keywords".to_string(),
            serde_json::json!(["halocline", "stratification"]),
        )],
    )
    .await;

    // A keyword-only term (absent from title/body) matches only the keyworded resource.
    assert!(
        index_matches(&pool, withkw, "halocline").await,
        "keyword term is indexed and searchable"
    );
    assert!(
        !index_matches(&pool, plain, "halocline").await,
        "keyword-less resource does not match a keyword-only term"
    );

    // Acceptance criterion: on a term present in BOTH bodies, the resource that ALSO lists it as a
    // keyword ranks strictly higher (extra weight-C lexeme). `thermocline` is in both bodies; add it
    // as a keyword on `withkw` at runtime to make the two identical-except-for-keywords.
    writes::set_property(
        &pool,
        temper_substrate::ids::ResourceId::from(withkw),
        "keywords",
        &serde_json::json!(["thermocline"]),
        emitter,
    )
    .await
    .unwrap();
    assert!(
        index_rank(&pool, withkw, "thermocline").await
            > index_rank(&pool, plain, "thermocline").await,
        "a resource whose keywords contain the query term ranks above an otherwise-identical one"
    );
}

/// Issue #359 — open_meta `descriptor` (weight D) is folded into the vector, so the section descriptor
/// truncated out of a title stays searchable.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn open_meta_descriptor_is_searchable(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "desc").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Section 4",
            origin_uri: "temper://desc/r",
            body: "generic prose",
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[(
                "descriptor".to_string(),
                serde_json::json!("centrifugal governor feedback dynamics"),
            )],
            chunks: None,
        },
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "centrifugal").await,
        "descriptor term (truncated out of the title) is indexed and searchable"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "unrelated").await,
        "a non-present term still does not match"
    );
}

/// Issue #359 — changing an indexed open_meta key at runtime re-folds the vector: the new keyword
/// becomes searchable and the superseded one drops out (property_set folds the prior row).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn updating_open_meta_keyword_reindexes(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "reidx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            sources: vec![],
            title: "Doc",
            origin_uri: "temper://reidx/r",
            body: "stable body",
            doc_type: "concept",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[("keywords".to_string(), serde_json::json!(["alpharhythm"]))],
            chunks: None,
        },
    )
    .await
    .unwrap();
    assert!(
        index_matches(&pool, r.uuid(), "alpharhythm").await,
        "create-time keyword is indexed"
    );

    writes::set_property(
        &pool,
        r,
        "keywords",
        &serde_json::json!(["betarhythm"]),
        emitter,
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "betarhythm").await,
        "updated keyword is indexed after property_set"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "alpharhythm").await,
        "superseded (folded) keyword no longer matches"
    );
    assert!(
        index_matches(&pool, r.uuid(), "stable").await,
        "body unchanged still indexed"
    );
}
