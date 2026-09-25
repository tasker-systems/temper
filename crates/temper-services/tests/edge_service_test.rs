#![cfg(feature = "test-db")]

//! `edge_service::list_resource_connections` — the bounded sibling read, probed against real
//! rows (spec D-F4). One fixture, six exclusion and bound facts:
//!
//! ```text
//!   a ──relates to──> b            VISIBLE        (resource peer, readable home)
//!   a ──relates to──> blob_v       VISIBLE        (blob peer, readable home)
//!   a ──relates to──> h            EXCLUDED       h is homed where p_in can't read (endpoint gate)
//!   a ──relates to──> d            EXCLUDED       the EDGE is homed in ctx_out (home gate)
//!   a ──relates to──> blob_u       EXCLUDED       the blob peer is unreadable (blob gate)
//!   a ──relates to──> b  [folded]  EXCLUDED       a retracted edge is not an edge
//! ```
//!
//! The load-bearing one is the first witness: an edge the gate excludes must be absent from
//! **both** `rows` and `total`. The count and the listing are two `query!` macros with one
//! predicate between them, held together by prose — this is the test that notices if they drift.

use sqlx::PgPool;
use uuid::Uuid;

use temper_services::error::ApiError;
use temper_services::services::edge_service;

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn personal_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A resource homed in `ctx`, owned by `owner` — visible to `owner` via the
/// personal/home arm of `resources_visible_to`.
async fn resource(pool: &PgPool, title: &str, ctx: Uuid, owner: Uuid) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), $1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// A blob homed in `ctx` with a readable `content_type` — so its visibility is decided
/// purely by whether the caller can read the home (`readable_blobs`' other conjunct).
async fn blob(pool: &PgPool, ctx: Uuid, owner: Uuid, event: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_blobs (id, content_hash, content_type, home_table, home_id, \
                               owner_profile_id, originator_profile_id, \
                               asserted_by_event_id, last_event_id) \
         VALUES (uuid_generate_v7(), 'fixture-' || gen_random_uuid()::text, \
                 'application/octet-stream', 'kb_contexts', $1, $2, $2, $3, $3) \
         RETURNING id",
    )
    .bind(ctx)
    .bind(owner)
    .bind(event)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// One dummy event to satisfy the `kb_edges`/`kb_blobs` NOT NULL event FKs.
async fn seed_event(pool: &PgPool, owner: Uuid) -> Uuid {
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (id, profile_id, name) \
         VALUES (uuid_generate_v7(), $1, 'emitter') RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_events (id, event_type_id, emitter_entity_id, payload) \
         SELECT uuid_generate_v7(), et.id, $1, '{}'::jsonb \
           FROM kb_event_types et WHERE et.name = 'resource_created' LIMIT 1 RETURNING id",
    )
    .bind(entity)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn edge(
    pool: &PgPool,
    source: Uuid,
    target_table: &str,
    target: Uuid,
    home_ctx: Uuid,
    event: Uuid,
    folded: bool,
) {
    sqlx::query(
        "INSERT INTO kb_edges \
           (id, source_table, source_id, target_table, target_id, edge_kind, polarity, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, $2, $3, 'leads_to', 'inverse', \
                 'relates to', 'kb_contexts', $4, $5, $5, $6)",
    )
    .bind(source)
    .bind(target_table)
    .bind(target)
    .bind(home_ctx)
    .bind(event)
    .bind(folded)
    .execute(pool)
    .await
    .unwrap();
}

struct Fixture {
    p_in: Uuid,
    p_out: Uuid,
    a: Uuid,
    b: Uuid,
    d: Uuid,
    h: Uuid,
    /// A resource with no edges at all.
    lonely: Uuid,
    blob_v: Uuid,
    blob_u: Uuid,
}

async fn build(pool: &PgPool) -> Fixture {
    let p_in = profile(pool, "conn-in").await;
    let p_out = profile(pool, "conn-out").await;
    let ctx_in = personal_context(pool, p_in, "ctx-in").await;
    let ctx_out = personal_context(pool, p_out, "ctx-out").await;
    let ev = seed_event(pool, p_in).await;

    let a = resource(pool, "Resource A", ctx_in, p_in).await;
    let b = resource(pool, "Resource B", ctx_in, p_in).await;
    let d = resource(pool, "Resource D", ctx_in, p_in).await; // readable — but its edge is not
    let h = resource(pool, "Resource H", ctx_out, p_out).await; // unreadable endpoint
    let lonely = resource(pool, "Resource Lonely", ctx_in, p_in).await;

    let blob_v = blob(pool, ctx_in, p_in, ev).await; // readable blob peer
    let blob_u = blob(pool, ctx_out, p_out, ev).await; // unreadable blob peer

    edge(pool, a, "kb_resources", b, ctx_in, ev, false).await; // visible
    edge(pool, a, "kb_blobs", blob_v, ctx_in, ev, false).await; // visible (blob peer)
    edge(pool, a, "kb_resources", h, ctx_in, ev, false).await; // endpoint gate
    edge(pool, a, "kb_resources", d, ctx_out, ev, false).await; // home gate
    edge(pool, a, "kb_blobs", blob_u, ctx_in, ev, false).await; // blob gate
    edge(pool, a, "kb_resources", b, ctx_in, ev, true).await; // folded

    Fixture {
        p_in,
        p_out,
        a,
        b,
        d,
        h,
        lonely,
        blob_v,
        blob_u,
    }
}

/// The trap witness: the gate excludes the edge from BOTH faces of the read. A count
/// restated over a wider predicate than the listing's would pass every other test here.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_the_gate_excludes_is_absent_from_rows_and_total(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    let page = edge_service::list_resource_connections(&pool, fx.p_in, fx.a, 50)
        .await
        .expect("A is visible to p_in");

    // The visible half: b and blob_v, and nothing else. Each exclusion gets its own
    // assertion naming the gate arm it rides, so a regression says which arm moved.
    let peers: Vec<(String, Uuid)> = page
        .rows
        .iter()
        .map(|r| (r.peer_table.clone(), r.peer_id))
        .collect();
    assert!(
        peers.contains(&("kb_resources".into(), fx.b)),
        "the readable resource edge is listed"
    );
    assert!(
        peers.contains(&("kb_blobs".into(), fx.blob_v)),
        "the readable blob edge is listed"
    );
    assert_eq!(
        page.total, 2,
        "the total counts exactly the edges the rows carry"
    );

    // Endpoint gate: H is invisible to p_in, so the edge naming it is nowhere.
    assert!(
        !page.rows.iter().any(|r| r.peer_id == fx.h),
        "endpoint gate: the edge to an unreadable resource is absent from rows"
    );
    // Home gate: D is readable, but the EDGE is homed in ctx_out.
    assert!(
        !page.rows.iter().any(|r| r.peer_id == fx.d),
        "home gate: an edge homed where the caller cannot read is absent from rows"
    );
    // Blob gate: the blob peer's home is unreadable.
    assert!(
        !page.rows.iter().any(|r| r.peer_id == fx.blob_u),
        "blob gate: the edge to an unreadable blob is absent from rows"
    );
    // Folded: a retracted edge is not an edge, on either face.
    assert_eq!(page.rows.len(), 2, "folded: only the two live edges render");
    assert_eq!(
        page.total, 2,
        "folded: the retracted edge is absent from the total too"
    );

    Ok(())
}

/// The limit bounds the page; the total stays whole — that split is the envelope's point.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_limit_bounds_rows_and_leaves_the_total_whole(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    let page = edge_service::list_resource_connections(&pool, fx.p_in, fx.a, 1)
        .await
        .expect("A is visible to p_in");

    assert_eq!(page.rows.len(), 1, "the page carries one row");
    assert_eq!(page.limit, 1, "the envelope echoes the applied limit");
    assert_eq!(page.total, 2, "the total is not clipped by the limit");
    assert_eq!(page.returned, 1);
    assert!(
        page.truncated,
        "rows are withheld, and the envelope says so"
    );

    Ok(())
}

/// `returned == total` means nothing is hidden — `truncated` is the page's own derivation,
/// false at the exact boundary, not a `total > returned` restatement wired to misfire.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_exact_boundary_is_not_truncated(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    let page = edge_service::list_resource_connections(&pool, fx.p_in, fx.a, 2)
        .await
        .expect("A is visible to p_in");

    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.total, 2);
    assert_eq!(
        page.returned, page.total,
        "the boundary page carries every visible edge"
    );
    assert!(!page.truncated, "nothing was withheld at the boundary");

    Ok(())
}

/// A visible resource with no edges answers a complete empty envelope — zero is an answer,
/// not a truncation and not a 404.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_visible_resource_with_no_edges_reports_a_complete_zero(
    pool: PgPool,
) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    let page = edge_service::list_resource_connections(&pool, fx.p_in, fx.lonely, 50)
        .await
        .expect("Lonely is visible to p_in");

    assert!(page.rows.is_empty());
    assert_eq!(page.total, 0);
    assert!(!page.truncated, "an empty set is complete, not clipped");

    Ok(())
}

/// 404 parity, verbatim with the incumbent listing: an invisible resource is NotFound on
/// the bounded read too, so neither face becomes an existence oracle.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_invisible_resource_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    for (who, target) in [(fx.p_out, fx.a), (fx.p_out, fx.b)] {
        let err = edge_service::list_resource_connections(&pool, who, target, 50)
            .await
            .expect_err("the target is invisible to p_out");
        assert!(
            matches!(err, ApiError::NotFound(_)),
            "invisible resource → NotFound, not an empty leak"
        );
    }

    Ok(())
}
