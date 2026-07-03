#![cfg(feature = "test-db")]
//! Regression: `DbBackend::assert_relationship` on a **cogmap-homed source**.
//!
//! A steward's authored-4 nodes are homed to the cognitive map (`kb_resource_homes.anchor_table =
//! 'kb_cogmaps'`), not a context. The edge-home lookup used to hard-filter `anchor_table='kb_contexts'`,
//! so `fetch_one` returned zero rows for every cogmap-homed source and the assert failed with "no rows
//! returned by a query that expected to return at least one row" — the map ended up with orphan nodes
//! and ZERO edges (observed on the first prod steward tick, invocation `019f25a8-…`, `edges_asserted: 0`).
//!
//! The fix reads the source's home anchor without assuming a context and homes the edge to the map when
//! the source is cogmap-homed (the `assert_kernel_edge` path). This test drives the backend command
//! directly against the pre-seeded L0 cogmap: it asserts the edge (a) succeeds and (b) homes to the map.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{AssertRelationship, Backend, Surface};

mod common;

/// The L0 system-default cogmap, born by migration `20260625000001` — present in every seeded DB.
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Create a resource homed to `cogmap`, owned by `owner` (so `owner` can modify it — the steward's
/// authored-4 shape). Returns the new resource id. NO ONNX: the body is chunked+embedded by the server
/// fallback, but the plain `test-db` tier tolerates that (no `test-embed` assertion here).
async fn cogmap_homed_node(
    pool: &PgPool,
    owner: ProfileId,
    emitter: temper_core::types::ids::EntityId,
    origin_uri: &str,
    title: &str,
) -> Uuid {
    temper_substrate::writes::create_kernel_resource(
        pool,
        temper_substrate::writes::KernelCreateParams {
            cogmap: temper_substrate::ids::CogmapId::from(L0_COGMAP),
            resource_id: Uuid::now_v7(),
            title,
            origin_uri,
            doc_type: "concept",
            body: "A distilled node the steward authored into the map.",
            chunks: None,
            owner,
            emitter,
        },
    )
    .await
    .expect("create cogmap-homed node")
    .uuid()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_homes_edge_to_cogmap_for_cogmap_homed_source(pool: PgPool) {
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&pool, "steward@example.com").await;
    let owner = ProfileId::from(profile);
    // The MCP surface emitter (`<handle>@mcp`) — the steward writes over MCP.
    let emitter = temper_substrate::writes::resolve_emitter(&pool, owner, "mcp")
        .await
        .expect("mcp emitter for the test profile");

    let src = cogmap_homed_node(&pool, owner, emitter, "temper://node/a", "Node A").await;
    let tgt = cogmap_homed_node(&pool, owner, emitter, "temper://node/b", "Node B").await;

    let be = DbBackend::new(pool.clone(), owner);
    let out = be
        .assert_relationship(AssertRelationship {
            source: ResourceId::from(src),
            target: ResourceId::from(tgt),
            edge_kind: EdgeKind::Near,
            polarity: Polarity::Forward,
            label: "relates_to".to_string(),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::Mcp,
        })
        .await
        .expect("a cogmap-homed source must assert its edge (regression: used to be RowNotFound)");
    let edge_id = Uuid::from(out.value);

    // The edge homes to the MAP, not a context — the fix routed it through the kernel-edge path.
    let (home_table, home_id): (String, Uuid) =
        sqlx::query_as("SELECT home_anchor_table, home_anchor_id FROM kb_edges WHERE id = $1")
            .bind(edge_id)
            .fetch_one(&pool)
            .await
            .expect("the asserted edge must exist");
    assert_eq!(home_table, "kb_cogmaps", "edge must home to the cogmap");
    assert_eq!(home_id, L0_COGMAP, "edge must home to the SOURCE's cogmap");
}
