#![cfg(feature = "artifact-tests")]
//! WS6 4c write-path mutation functions — the new event-sourced mutations the `NextBackend` dispatches
//! to: the edge-uniqueness invariant (idempotent `relationship_assert`), `resource_delete`/`update`/
//! `rehome`, and `relationship_retype`/`reweight`. Each resets the artifact (01+02 via psql), boot-seeds
//! the system actor, and exercises the mutation through the `events::fire` surface. Serialized via the
//! `temper-substrate-write` nextest group (it owns the namespace).

mod common;

use temper_substrate::content::{PreparedBlock, PreparedChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{BlockId, ChunkId, ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::{affinity::EdgeKind, scenario::bootseed, substrate};
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

/// Fire one action in its own tx (search_path set), returning its `Fired` record.
async fn fire_one(pool: &sqlx::PgPool, action: SeedAction<'_>) -> temper_substrate::events::Fired {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    let fired = fire(&mut tx, action).await.unwrap();
    tx.commit().await.unwrap();
    fired
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
            home: temper_substrate::events::EdgeHome::Context(home),
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

// ── Task 1.3: resource_delete ───────────────────────────────────────────────────

#[tokio::test]
async fn resource_delete_sets_inactive() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "del").await;
    let r = make_resource(&pool, ctx, owner, emitter, "temper://d/r", "body").await;

    fire_one(
        &pool,
        SeedAction::ResourceDelete {
            resource: r,
            emitter,
        },
    )
    .await;

    let active: bool =
        sqlx::query_scalar("SELECT is_active FROM temper_next.kb_resources WHERE id=$1")
            .bind(r.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!active, "resource_delete must flip is_active to false");
}

// ── Task 1.4: resource_update ───────────────────────────────────────────────────

#[tokio::test]
async fn resource_update_changes_title_only() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "upd").await;
    // make_resource sets title == origin_uri == "temper://u/r"
    let r = make_resource(&pool, ctx, owner, emitter, "temper://u/r", "body").await;

    fire_one(
        &pool,
        SeedAction::ResourceUpdate {
            resource: r,
            title: Some("New Title"),
            origin_uri: None,
            emitter,
        },
    )
    .await;

    let (title, uri): (String, String) =
        sqlx::query_as("SELECT title, origin_uri FROM temper_next.kb_resources WHERE id=$1")
            .bind(r.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(title, "New Title", "title updated");
    assert_eq!(
        uri, "temper://u/r",
        "origin_uri unchanged (None ⇒ COALESCE to current)"
    );
}

// ── Task 1.5: resource_rehome ───────────────────────────────────────────────────

#[tokio::test]
async fn resource_rehome_moves_to_destination_context() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx_a = make_context(&pool, owner, "ctx-a").await;
    let ctx_b = make_context(&pool, owner, "ctx-b").await;
    let r = make_resource(&pool, ctx_a, owner, emitter, "temper://m/r", "body").await;

    fire_one(
        &pool,
        SeedAction::ResourceRehome {
            resource: r,
            home: AnchorRef::context(ctx_b),
            emitter,
        },
    )
    .await;

    let anchor: Uuid = sqlx::query_scalar(
        "SELECT anchor_id FROM temper_next.kb_resource_homes WHERE resource_id=$1",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        anchor,
        ctx_b.uuid(),
        "home re-anchored to the destination context"
    );
}

// ── Task 1.6: relationship_retype ───────────────────────────────────────────────

#[tokio::test]
async fn relationship_retype_changes_kind() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "rt").await;
    let a = make_resource(&pool, ctx, owner, emitter, "temper://rt/a", "alpha").await;
    let b = make_resource(&pool, ctx, owner, emitter, "temper://rt/b", "beta").await;
    let edge = assert_edge(&pool, a, b, EdgeKind::LeadsTo, Some("x"), 1.0, ctx, emitter).await;

    fire_one(
        &pool,
        SeedAction::RelationshipRetype {
            edge,
            kind: EdgeKind::Contains,
            polarity: EdgePolarity::Forward,
            emitter,
        },
    )
    .await;

    let kind: String =
        sqlx::query_scalar("SELECT edge_kind::text FROM temper_next.kb_edges WHERE id=$1")
            .bind(edge.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kind, "contains", "edge_kind retyped");
}

// ── Task 1.7: relationship_reweight ─────────────────────────────────────────────

#[tokio::test]
async fn relationship_reweight_changes_weight() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "rw").await;
    let a = make_resource(&pool, ctx, owner, emitter, "temper://rw/a", "alpha").await;
    let b = make_resource(&pool, ctx, owner, emitter, "temper://rw/b", "beta").await;
    let edge = assert_edge(&pool, a, b, EdgeKind::LeadsTo, Some("x"), 1.0, ctx, emitter).await;

    fire_one(
        &pool,
        SeedAction::RelationshipReweight {
            edge,
            weight: 3.5,
            emitter,
        },
    )
    .await;

    let weight: f64 = sqlx::query_scalar("SELECT weight FROM temper_next.kb_edges WHERE id=$1")
        .bind(edge.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(weight, 3.5, "edge weight updated");
}

// ── consolidated replay parity for the 4c mutations ─────────────────────────────

/// Fire every new mutation, then reset + replay the ledger through the SAME `_project_*` halves and
/// prove the projections come back byte-identical (replay-is-the-same-code-path, spec §0/§3). Also
/// covers the relationship_folded replay arm wired alongside the 4c ones.
#[tokio::test]
async fn mutations_replay_byte_identically() {
    use temper_substrate::replay;
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx_a = make_context(&pool, owner, "rp-a").await;
    let ctx_b = make_context(&pool, owner, "rp-b").await;
    let a = make_resource(&pool, ctx_a, owner, emitter, "temper://rp/a", "alpha body").await;
    let b = make_resource(&pool, ctx_a, owner, emitter, "temper://rp/b", "beta body").await;
    let edge = assert_edge(
        &pool,
        a,
        b,
        EdgeKind::LeadsTo,
        Some("x"),
        1.0,
        ctx_a,
        emitter,
    )
    .await;

    fire_one(
        &pool,
        SeedAction::ResourceUpdate {
            resource: a,
            title: Some("Renamed"),
            origin_uri: None,
            emitter,
        },
    )
    .await;
    fire_one(
        &pool,
        SeedAction::ResourceRehome {
            resource: a,
            home: AnchorRef::context(ctx_b),
            emitter,
        },
    )
    .await;
    fire_one(
        &pool,
        SeedAction::RelationshipRetype {
            edge,
            kind: EdgeKind::Contains,
            polarity: EdgePolarity::Forward,
            emitter,
        },
    )
    .await;
    fire_one(
        &pool,
        SeedAction::RelationshipReweight {
            edge,
            weight: 2.5,
            emitter,
        },
    )
    .await;
    fire_one(
        &pool,
        SeedAction::RelationshipFold {
            edge,
            reason: Some("retired"),
            emitter,
        },
    )
    .await;
    fire_one(
        &pool,
        SeedAction::ResourceDelete {
            resource: b,
            emitter,
        },
    )
    .await;

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_artifact();
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();

    for ((ta, va), (tb, vb)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(va, vb, "projection table {ta} diverged under replay");
    }
}

// ── property_set single-valued supersede ────────────────────────────────────────

/// Setting a single-valued key repeatedly (incl. revert-to-a-prior-value) leaves exactly one ACTIVE
/// row at the latest value, with the prior values folded as history — and never trips the active
/// uniqueness index.
#[tokio::test]
async fn property_set_supersedes_prior_value() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "ps").await;
    let r = make_resource(&pool, ctx, owner, emitter, "temper://ps/r", "body").await;

    for v in ["backlog", "done", "backlog"] {
        fire_one(
            &pool,
            SeedAction::PropertySet {
                resource: r,
                key: "temper-stage",
                value: &serde_json::json!(v),
                weight: 1.0,
                emitter,
            },
        )
        .await;
    }

    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM temper_next.kb_properties \
         WHERE owner_id=$1 AND property_key='temper-stage' AND NOT is_folded",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        active, 1,
        "exactly one active temper-stage row after supersede + revert"
    );

    let val: serde_json::Value = sqlx::query_scalar(
        "SELECT property_value FROM temper_next.kb_properties \
         WHERE owner_id=$1 AND property_key='temper-stage' AND NOT is_folded",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        val,
        serde_json::json!("backlog"),
        "current value is the last set"
    );

    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM temper_next.kb_properties \
         WHERE owner_id=$1 AND property_key='temper-stage'",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, 3, "prior values preserved as folded history");
}

// ── Slice 2: writes module composition ──────────────────────────────────────────

/// create_resource + update_resource through the typed write ops, observed via readback — proves the
/// composition (block + single-valued properties + title + rehome) lands at the §9 read floor.
#[tokio::test]
async fn writes_create_then_update_reflected_in_readback() {
    use temper_substrate::{readback, writes};
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let ctx = make_context(&pool, owner, "w").await;
    let ctx2 = make_context(&pool, owner, "w2").await;

    let create_props = vec![("temper-stage".to_string(), serde_json::json!("backlog"))];
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "Orig",
            origin_uri: "temper://w/r",
            body: "original body text here",
            doc_type: "task",
            home: ctx,
            owner,
            originator: owner,
            emitter,
            properties: &create_props,
            chunks: None,
        },
    )
    .await
    .unwrap();

    writes::update_resource(
        &pool,
        writes::UpdateParams {
            resource: r,
            body: Some("revised body text now"),
            title: Some("Renamed"),
            origin_uri: None,
            properties: &[("temper-stage".to_string(), serde_json::json!("done"))],
            chunks: None,
            rehome_to: Some(ctx2),
            emitter,
        },
    )
    .await
    .unwrap();

    let row = readback::resource_row(&pool, owner.uuid(), r.uuid())
        .await
        .unwrap();
    assert_eq!(row.title, "Renamed", "title updated");
    assert_eq!(
        row.stage.as_deref(),
        Some("done"),
        "stage superseded, single current value"
    );

    let body: String = sqlx::query_scalar("SELECT temper_next.resource_body_text($1)")
        .bind(r.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(body.contains("revised"), "body revised: {body:?}");

    let anchor: Uuid = sqlx::query_scalar(
        "SELECT anchor_id FROM temper_next.kb_resource_homes WHERE resource_id=$1",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(anchor, ctx2.uuid(), "rehomed to ctx2");
}

/// The natural-key resolvers find an entity + context by their synthesis-written keys.
#[tokio::test]
async fn writes_resolvers_find_context_and_emitter() {
    use temper_substrate::writes;
    let pool = setup().await;
    let (owner, _sys) = system_actor(&pool).await;
    // a context named "My Ctx" → slug "my-ctx"; the owner's per-surface emitter entity is named
    // `<handle>@<surface>` (the de-hardcoded resolver) — the owner is the boot-seeded `system` actor,
    // so the cli emitter is `system@cli`.
    common::insert_context(&pool, "kb_profiles", owner.uuid(), "my-ctx", "My Ctx")
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'system@cli')")
        .bind(owner.uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let ctx = writes::resolve_context(&pool, owner, "My Ctx")
        .await
        .unwrap();
    let exists: bool = sqlx::query_scalar(
        "SELECT exists(SELECT 1 FROM temper_next.kb_contexts WHERE id=$1 AND slug='my-ctx')",
    )
    .bind(ctx.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        exists,
        "resolve_context slugified the name and found the row"
    );

    let emitter = writes::resolve_emitter(&pool, owner, "cli").await.unwrap();
    let name: String = sqlx::query_scalar("SELECT name FROM temper_next.kb_entities WHERE id=$1")
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "system@cli");
}
