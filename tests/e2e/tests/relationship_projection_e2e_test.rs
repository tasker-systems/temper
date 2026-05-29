#![cfg(feature = "test-db")]
//! E2e: dropping kb_resource_edges and rebuilding from the kb_events ledger
//! yields byte-identical traversal output — the spec's headline acceptance
//! criterion for limb 1.

mod common;

use sqlx::PgPool;
use temper_api::MIGRATOR;
use temper_core::operations::ResourceRef;
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ingest::IngestPayload;
use temper_core::types::relationship_requests::{
    AssertRelationshipRequest, FoldRelationshipRequest, RetypeRelationshipRequest,
    ReweightRelationshipRequest,
};
use uuid::Uuid;

/// Seed a resource via the ingest path. Uses pre-computed empty chunks_packed
/// to bypass find_by_body_hash dedup collapsing empty-content resources when the
/// ingest-pipeline feature is enabled (same pattern as projection_pull_test.rs).
async fn seed_resource(
    app: &common::E2eTestApp,
    context: &str,
    doc_type: &str,
    slug: &str,
    title: &str,
) -> temper_core::types::ResourceId {
    let empty_chunks =
        Some(temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks"));
    app.client
        .ingest()
        .create(&IngestPayload {
            title: title.to_string(),
            origin_uri: format!("test://e2e/rel-proj/{slug}"),
            context_name: context.to_string(),
            doc_type_name: doc_type.to_string(),
            content_hash: None,
            slug: slug.to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks,
        })
        .await
        .expect("ingest resource")
        .id
}

/// Stable snapshot row for `graph_traverse` output.
/// Sorted by `(resource_id, depth)` since `graph_traverse` returns at most
/// one row per resource_id (DISTINCT ON), so resource_id is the natural key.
///
/// `path_weight` is encoded via `f64::to_bits` so the snapshot survives
/// floating-point representation differences — matches the pattern in
/// `NeighborRow.weight_bits`.
#[derive(Debug, PartialEq, Eq, Clone)]
struct TraverseRow {
    resource_id: Uuid,
    depth: i32,
    edge_kind: Option<EdgeKind>,
    polarity: Option<Polarity>,
    label: Option<String>,
    from_resource_id: Option<Uuid>,
    path: Vec<Uuid>,
    path_weight_bits: i64,
}

impl PartialOrd for TraverseRow {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TraverseRow {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.resource_id
            .cmp(&other.resource_id)
            .then(self.depth.cmp(&other.depth))
    }
}

/// Stable snapshot row for `graph_neighbors` output.
/// Sorted by `(resource_id, direction, label)`.
#[derive(Debug, PartialEq, Eq, Clone, sqlx::FromRow)]
struct NeighborRow {
    resource_id: Uuid,
    edge_kind: EdgeKind,
    polarity: Polarity,
    label: String,
    direction: String,
    #[allow(dead_code)]
    weight_bits: i64,
}

impl PartialOrd for NeighborRow {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NeighborRow {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.resource_id
            .cmp(&other.resource_id)
            .then(self.direction.cmp(&other.direction))
            .then(self.label.cmp(&other.label))
    }
}

/// Raw `graph_traverse` row tuple as returned by the runtime `query_as`:
/// (resource_id, depth, edge_kind, polarity, label, path, from_resource_id, path_weight).
type TraverseRawRow = (
    Uuid,
    i32,
    Option<EdgeKind>,
    Option<Polarity>,
    Option<String>,
    Vec<Uuid>,
    Option<Uuid>,
    f64,
);

/// Query `graph_traverse` for all resources reachable from `seed_ids`.
async fn snapshot_traverse(pool: &PgPool, profile_id: Uuid, seed_ids: &[Uuid]) -> Vec<TraverseRow> {
    // Runtime query_as: graph_traverse returns a composite row type with
    // edge_kind/edge_polarity Postgres enums that the compile-time macro
    // cannot check against. path_weight is encoded as f64::to_bits to keep
    // snapshot comparisons stable.
    let raw: Vec<TraverseRawRow> = sqlx::query_as(
        r#"
        SELECT
            resource_id,
            depth,
            edge_kind,
            polarity,
            label,
            path,
            from_resource_id,
            path_weight
          FROM graph_traverse($1, $2::uuid[], 10, '{}')
        "#,
    )
    .bind(profile_id)
    .bind(seed_ids)
    .fetch_all(pool)
    .await
    .expect("graph_traverse query");

    let mut rows: Vec<TraverseRow> = raw
        .into_iter()
        .map(|(rid, depth, ek, pol, lbl, path, from, pw)| TraverseRow {
            resource_id: rid,
            depth,
            edge_kind: ek,
            polarity: pol,
            label: lbl,
            from_resource_id: from,
            path,
            path_weight_bits: pw.to_bits() as i64,
        })
        .collect();
    rows.sort();
    rows
}

/// Query `graph_neighbors` for a resource.
async fn snapshot_neighbors(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> Vec<NeighborRow> {
    // We encode weight as bits (i64) so the snapshot is stable across
    // floating-point representation differences; f64::to_bits is deterministic.
    // Runtime query_as: same reason as graph_traverse — Postgres enum casts.
    let raw: Vec<(Uuid, EdgeKind, Polarity, String, String, f64)> = sqlx::query_as(
        r#"
        SELECT
            resource_id,
            edge_kind,
            polarity,
            label,
            direction,
            weight
          FROM graph_neighbors($1, $2, 'both', '{}')
        "#,
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_all(pool)
    .await
    .expect("graph_neighbors query");

    let mut rows: Vec<NeighborRow> = raw
        .into_iter()
        .map(|(rid, ek, pol, lbl, dir, w)| NeighborRow {
            resource_id: rid,
            edge_kind: ek,
            polarity: pol,
            label: lbl,
            direction: dir,
            weight_bits: w.to_bits() as i64,
        })
        .collect();
    rows.sort();
    rows
}

/// The headline acceptance criterion for Limb 1:
/// truncate `kb_resource_edges` and rebuild from `kb_events` via
/// `rebuild_edge_projection` — the resulting traversal output must be
/// byte-identical to the pre-rebuild snapshot.
///
/// Graph under test:
///   alpha --near/references--> beta    (asserted, then reweighted)
///   alpha --contains/parent_of--> gamma (asserted, then retyped to near)
///   alpha --near/references--> delta   (asserted, then folded)
#[sqlx::test(migrator = "MIGRATOR")]
async fn rebuild_edge_projection_yields_identical_traversal(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    // Pre-flight: ensure profile is provisioned.
    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = profile.id;

    app.client
        .contexts()
        .create("rel-proj-ctx")
        .await
        .expect("context create");

    // Seed four resources.
    let alpha_id = seed_resource(&app, "rel-proj-ctx", "research", "alpha", "Alpha").await;
    let beta_id = seed_resource(&app, "rel-proj-ctx", "research", "beta", "Beta").await;
    let gamma_id = seed_resource(&app, "rel-proj-ctx", "research", "gamma", "Gamma").await;
    let delta_id = seed_resource(&app, "rel-proj-ctx", "research", "delta", "Delta").await;

    let alpha_uuid = Uuid::from(alpha_id);
    let beta_uuid = Uuid::from(beta_id);
    let gamma_uuid = Uuid::from(gamma_id);
    let delta_uuid = Uuid::from(delta_id);

    // Assert alpha → beta (near / references / weight 0.8)
    let ack_ab = app
        .client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source: ResourceRef::uuid(alpha_id),
            target_slug: "beta".to_string(),
            edge_kind: EdgeKind::Near,
            polarity: Polarity::Forward,
            label: "references".to_string(),
            weight: 0.8,
        })
        .await
        .expect("assert alpha→beta");
    let corr_ab = ack_ab.correlation_id;

    // Assert alpha → gamma (contains / parent_of / weight 1.0)
    let ack_ag = app
        .client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source: ResourceRef::uuid(alpha_id),
            target_slug: "gamma".to_string(),
            edge_kind: EdgeKind::Contains,
            polarity: Polarity::Forward,
            label: "parent_of".to_string(),
            weight: 1.0,
        })
        .await
        .expect("assert alpha→gamma");
    let corr_ag = ack_ag.correlation_id;

    // Assert alpha → delta (near / references / weight 0.5) — will be folded
    let ack_ad = app
        .client
        .relationships()
        .assert(&AssertRelationshipRequest {
            source: ResourceRef::uuid(alpha_id),
            target_slug: "delta".to_string(),
            edge_kind: EdgeKind::Near,
            polarity: Polarity::Forward,
            label: "references".to_string(),
            weight: 0.5,
        })
        .await
        .expect("assert alpha→delta");
    let corr_ad = ack_ad.correlation_id;

    // Reweight alpha→beta from 0.8 to 0.9
    app.client
        .relationships()
        .reweight(corr_ab, &ReweightRelationshipRequest { weight: 0.9 })
        .await
        .expect("reweight alpha→beta");

    // Retype alpha→gamma from contains/parent_of to near/relates_to
    app.client
        .relationships()
        .retype(
            corr_ag,
            &RetypeRelationshipRequest {
                edge_kind: EdgeKind::Near,
                polarity: Polarity::Forward,
            },
        )
        .await
        .expect("retype alpha→gamma");

    // Fold alpha→delta (retract)
    app.client
        .relationships()
        .fold(
            corr_ad,
            &FoldRelationshipRequest {
                reason: Some("test fold".to_string()),
            },
        )
        .await
        .expect("fold alpha→delta");

    // ── Pre-rebuild snapshot ─────────────────────────────────────────────

    let seeds = vec![alpha_uuid];
    let traverse_before = snapshot_traverse(&app.pool, profile_id, &seeds).await;
    let neighbors_before = snapshot_neighbors(&app.pool, profile_id, alpha_uuid).await;

    // Sanity: alpha is reachable from itself in traversal, but depth-0 rows are
    // excluded by graph_traverse (it only returns depth > 0).
    assert!(
        !traverse_before.is_empty(),
        "should have at least one traversal row (beta and/or gamma)"
    );
    // delta is folded — must not appear in traversal
    assert!(
        !traverse_before.iter().any(|r| r.resource_id == delta_uuid),
        "folded delta must not appear in traversal"
    );
    // beta must be reachable
    assert!(
        traverse_before.iter().any(|r| r.resource_id == beta_uuid),
        "beta must be reachable from alpha"
    );
    // gamma must be reachable (retyped to near)
    assert!(
        traverse_before.iter().any(|r| r.resource_id == gamma_uuid),
        "gamma must be reachable from alpha after retype"
    );

    // ── Rebuild projection ───────────────────────────────────────────────

    let mut conn = app.pool.acquire().await.expect("acquire connection");
    temper_api::services::relationship_service::rebuild_edge_projection(&mut conn)
        .await
        .expect("rebuild_edge_projection");

    // ── Post-rebuild snapshot ────────────────────────────────────────────

    let traverse_after = snapshot_traverse(&app.pool, profile_id, &seeds).await;
    let neighbors_after = snapshot_neighbors(&app.pool, profile_id, alpha_uuid).await;

    // The headline invariant: traversal output must be byte-identical.
    assert_eq!(
        traverse_before, traverse_after,
        "graph_traverse output must be identical after rebuild_edge_projection"
    );
    assert_eq!(
        neighbors_before, neighbors_after,
        "graph_neighbors output must be identical after rebuild_edge_projection"
    );

    // Folded-row persistence: graph_traverse filters folded edges out by
    // default, so the absence check above is necessary but not sufficient —
    // a regression where rebuild dropped the row entirely (instead of
    // preserving it with is_folded=true) would still pass. Query the
    // projection table directly to assert the row survives in folded form.
    let folded: (bool,) = sqlx::query_as(
        "SELECT is_folded FROM kb_resource_edges
          WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(alpha_uuid)
    .bind(delta_uuid)
    .fetch_one(&app.pool)
    .await
    .expect("folded alpha→delta row must survive rebuild in projection table");
    assert!(folded.0, "alpha→delta must be preserved as is_folded=true");
}

/// Migration fidelity: pre-existing edges created via the ingest path
/// (server-side `temper-goal` extraction → `parent_of` edges) survive the
/// rebuild just as explicitly-asserted relationship events do.
///
/// This guards the invariant that `rebuild_edge_projection` replays ALL
/// `relationship_*` events, including those injected by the ingest pipeline.
#[sqlx::test(migrator = "MIGRATOR")]
async fn migration_fidelity_ingest_edges_survive_rebuild(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let profile_id = profile.id;

    app.client
        .contexts()
        .create("fidelity-ctx")
        .await
        .expect("context create");

    // Seed a goal and two tasks linked to it via temper-goal frontmatter.
    // The ingest service extracts temper-goal and emits a relationship_asserted
    // event for the parent_of edge server-side (same path as graph_build_e2e_test).
    let empty_chunks =
        Some(temper_core::types::ingest::pack_chunks(&[]).expect("pack empty chunks"));

    app.client
        .ingest()
        .create(&IngestPayload {
            title: "fidelity-goal".to_string(),
            origin_uri: "test://e2e/fidelity/fidelity-goal".to_string(),
            context_name: "fidelity-ctx".to_string(),
            doc_type_name: "goal".to_string(),
            content_hash: None,
            slug: "fidelity-goal".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks.clone(),
        })
        .await
        .expect("ingest fidelity-goal");

    app.client
        .ingest()
        .create(&IngestPayload {
            title: "fidelity-task-one".to_string(),
            origin_uri: "test://e2e/fidelity/fidelity-task-one".to_string(),
            context_name: "fidelity-ctx".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: None,
            slug: "fidelity-task-one".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-goal": "fidelity-goal"})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks.clone(),
        })
        .await
        .expect("ingest fidelity-task-one");

    app.client
        .ingest()
        .create(&IngestPayload {
            title: "fidelity-task-two".to_string(),
            origin_uri: "test://e2e/fidelity/fidelity-task-two".to_string(),
            context_name: "fidelity-ctx".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: None,
            slug: "fidelity-task-two".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-goal": "fidelity-goal"})),
            open_meta: Some(serde_json::json!({})),
            chunks_packed: empty_chunks,
        })
        .await
        .expect("ingest fidelity-task-two");

    // Resolve goal UUID for seeding traversal.
    let goal_uuid: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_resources WHERE slug = 'fidelity-goal' AND owner_profile_id = $1",
    )
    .bind(profile_id)
    .fetch_one(&app.pool)
    .await
    .expect("resolve fidelity-goal uuid");

    // Verify goal→task edges were created by ingest extraction.
    let edge_count_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'fidelity-goal'
           AND tgt.slug IN ('fidelity-task-one', 'fidelity-task-two')
           AND e.label = 'parent_of'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("count edges before rebuild");
    assert_eq!(
        edge_count_before, 2,
        "ingest should have created 2 parent_of edges from goal to tasks"
    );

    // Snapshot traversal from the goal.
    let seeds = vec![goal_uuid];
    let traverse_before = snapshot_traverse(&app.pool, profile_id, &seeds).await;
    let neighbors_before = snapshot_neighbors(&app.pool, profile_id, goal_uuid).await;

    // Rebuild the projection from events.
    let mut conn = app.pool.acquire().await.expect("acquire connection");
    temper_api::services::relationship_service::rebuild_edge_projection(&mut conn)
        .await
        .expect("rebuild_edge_projection");

    // Verify the edge count is preserved after rebuild.
    let edge_count_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'fidelity-goal'
           AND tgt.slug IN ('fidelity-task-one', 'fidelity-task-two')
           AND e.label = 'parent_of'",
    )
    .fetch_one(&app.pool)
    .await
    .expect("count edges after rebuild");
    assert_eq!(
        edge_count_after, 2,
        "ingest-seeded parent_of edges must survive rebuild_edge_projection"
    );

    // The snapshot invariant: traversal output is identical after rebuild.
    let traverse_after = snapshot_traverse(&app.pool, profile_id, &seeds).await;
    let neighbors_after = snapshot_neighbors(&app.pool, profile_id, goal_uuid).await;

    assert_eq!(
        traverse_before, traverse_after,
        "graph_traverse from goal must be identical after rebuild"
    );
    assert_eq!(
        neighbors_before, neighbors_after,
        "graph_neighbors for goal must be identical after rebuild"
    );
}
