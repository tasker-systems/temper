#![cfg(feature = "artifact-tests")]
//! WS6 4c write-path mutation functions — the new event-sourced mutations the `NextBackend` dispatches
//! to: the edge-uniqueness invariant (idempotent `relationship_assert`), `resource_delete`/`update`/
//! `rehome`, and `relationship_retype`/`reweight`. Each resets the artifact (01+02 via psql), boot-seeds
//! the system actor, and exercises the mutation through the `events::fire` surface. Serialized via the
//! `temper-next-write` nextest group (it owns the namespace).

mod common;

use temper_next::content::{PreparedBlock, PreparedChunk};
use temper_next::events::{fire, SeedAction};
use temper_next::ids::{BlockId, ChunkId, ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use temper_next::payloads::{AnchorRef, EdgePolarity};
use temper_next::{affinity::EdgeKind, scenario::bootseed, substrate};
use uuid::Uuid;

// ── shared helpers ────────────────────────────────────────────────────────────

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

/// A profile-owned context to home resources in (the §2-amended owner-scoped shape).
async fn make_context(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    let id = common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
        .await
        .unwrap();
    ContextId::from(id)
}

/// One prepared block, one chunk, a fixed non-degenerate 768-d unit embedding (ONNX-free — these tests
/// exercise structure, not embedding quality). `content` becomes the chunk prose (and the body text).
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

/// Create one resource homed in `ctx`, returning its id. Own tx with the search_path discipline.
async fn make_resource(
    pool: &sqlx::PgPool,
    ctx: ContextId,
    owner: ProfileId,
    emitter: EntityId,
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
            title: origin_uri,
            origin_uri,
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
#[allow(clippy::too_many_arguments)]
async fn assert_edge(
    pool: &sqlx::PgPool,
    src: ResourceId,
    tgt: ResourceId,
    kind: EdgeKind,
    label: Option<&str>,
    weight: f64,
    home: ContextId,
    emitter: EntityId,
) -> EdgeId {
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
            kind,
            polarity: EdgePolarity::Forward,
            label,
            weight,
            home: temper_next::events::EdgeHome::Context(home),
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

// ── Task 1.2: edge-uniqueness invariant ─────────────────────────────────────────

/// Re-asserting the same active (src,tgt,kind,label) updates the existing edge's weight rather than
/// creating a duplicate active edge (spec 4c: keep production's no-duplicate-active-edge invariant).
#[tokio::test]
async fn reassert_active_edge_is_idempotent() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "edges").await;
    let a = make_resource(&pool, ctx, owner, emitter, "temper://e/a", "alpha").await;
    let b = make_resource(&pool, ctx, owner, emitter, "temper://e/b", "beta").await;

    let e1 = assert_edge(
        &pool,
        a,
        b,
        EdgeKind::LeadsTo,
        Some("operationalized_by"),
        1.0,
        ctx,
        emitter,
    )
    .await;
    let e2 = assert_edge(
        &pool,
        a,
        b,
        EdgeKind::LeadsTo,
        Some("operationalized_by"),
        2.0,
        ctx,
        emitter,
    )
    .await;

    assert_eq!(e1, e2, "re-assert must return the SAME edge id");
    let (count, weight): (i64, f64) = sqlx::query_as(
        "SELECT count(*), max(weight) FROM temper_next.kb_edges \
         WHERE source_id=$1 AND target_id=$2 AND NOT is_folded",
    )
    .bind(a.uuid())
    .bind(b.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "exactly one active edge");
    assert_eq!(weight, 2.0, "weight updated to the re-asserted value");
}

/// A folded edge does NOT block a fresh active assert of the same relationship: the partial unique
/// index excludes folded rows, so the second assert mints a NEW active edge.
#[tokio::test]
async fn reassert_after_fold_creates_fresh_edge() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "edges").await;
    let a = make_resource(&pool, ctx, owner, emitter, "temper://e/a", "alpha").await;
    let b = make_resource(&pool, ctx, owner, emitter, "temper://e/b", "beta").await;

    let e1 = assert_edge(&pool, a, b, EdgeKind::Near, None, 1.0, ctx, emitter).await;

    // fold e1
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge: e1,
            reason: None,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let e2 = assert_edge(&pool, a, b, EdgeKind::Near, None, 1.0, ctx, emitter).await;
    assert_ne!(e1, e2, "a fresh active edge must be minted after the fold");

    let (active, folded): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE NOT is_folded), count(*) FILTER (WHERE is_folded) \
         FROM temper_next.kb_edges WHERE source_id=$1 AND target_id=$2",
    )
    .bind(a.uuid())
    .bind(b.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active, 1, "exactly one active edge");
    assert_eq!(folded, 1, "the folded edge remains");
}
