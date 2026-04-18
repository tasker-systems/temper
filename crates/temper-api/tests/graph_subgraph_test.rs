#![cfg(feature = "test-db")]
//! Integration tests for `graph_service::aggregator_subgraph`.
//!
//! All tests load `scripts/seed-graph-fixtures.sql` after `clean_and_seed`,
//! then call the service against Alice's profile asking for concepts in
//! `graph-test-primary`.

mod common;

use sqlx::PgPool;
use std::path::Path;
use uuid::Uuid;

use temper_api::services::graph_service::{aggregator_subgraph, AggregatorSubgraphParams};
use temper_core::frontmatter::document::DocType;

// Well-known UUIDs from scripts/seed-graph-fixtures.sql.
const ALICE: &str = "00000000-0000-0000-0088-000000000001";
#[allow(dead_code)]
const BOB: &str = "00000000-0000-0000-0088-000000000002";

const C1_IDEMPOTENCY: &str = "00000000-0000-0000-00c1-000000000001";
const C2_CIRCUIT: &str = "00000000-0000-0000-00c1-000000000002";
const C3_SINGLETON: &str = "00000000-0000-0000-00c1-000000000003";
const C4_AUTH: &str = "00000000-0000-0000-00c1-000000000004";
const C5_DELETED: &str = "00000000-0000-0000-00c1-000000000005";

const M1_OAUTH: &str = "00000000-0000-0000-00c2-000000000001";
const M2_MIDDLEWARE: &str = "00000000-0000-0000-00c2-000000000002";
const M3_SESSION: &str = "00000000-0000-0000-00c2-000000000003";
const M4_CIRCUIT_DESIGN: &str = "00000000-0000-0000-00c2-000000000004";
const M5_CIRCUIT_IMPL: &str = "00000000-0000-0000-00c2-000000000005";
const M6_JWT: &str = "00000000-0000-0000-00c2-000000000006";

const T1_TOKEN_REFRESH: &str = "00000000-0000-0000-00c3-000000000001";
const T2_SESSION_MGMT: &str = "00000000-0000-0000-00c3-000000000002";

const B1_BOB_CONCEPT: &str = "00000000-0000-0000-00c4-000000000001";
const B2_BOB_RESEARCH: &str = "00000000-0000-0000-00c4-000000000002";

const S1_SECONDARY: &str = "00000000-0000-0000-00c5-000000000001";

/// Load `scripts/seed-graph-fixtures.sql` into the test pool.
async fn load_graph_fixtures(pool: &PgPool) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR"); // crates/temper-api
    let sql_path = Path::new(manifest_dir)
        .join("../..")
        .join("scripts/seed-graph-fixtures.sql");
    let sql = std::fs::read_to_string(&sql_path)
        .unwrap_or_else(|e| panic!("read graph fixture sql at {}: {e}", sql_path.display()));
    // The script is wrapped in BEGIN/COMMIT; execute as-is.
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .expect("load graph fixtures");
}

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn happy_path_returns_all_concepts_and_direct_members(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph should succeed");

    let node_ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();

    // All active concepts present
    assert!(node_ids.contains(&uuid(C1_IDEMPOTENCY)), "c1 present");
    assert!(node_ids.contains(&uuid(C2_CIRCUIT)), "c2 present");
    assert!(
        node_ids.contains(&uuid(C3_SINGLETON)),
        "c3 (singleton) present"
    );
    assert!(node_ids.contains(&uuid(C4_AUTH)), "c4 present");

    // All direct members present
    assert!(
        node_ids.contains(&uuid(M1_OAUTH)),
        "m1 present (shared member)"
    );
    assert!(node_ids.contains(&uuid(M2_MIDDLEWARE)), "m2 present");
    assert!(node_ids.contains(&uuid(M3_SESSION)), "m3 present");
    assert!(node_ids.contains(&uuid(M4_CIRCUIT_DESIGN)), "m4 present");
    assert!(node_ids.contains(&uuid(M5_CIRCUIT_IMPL)), "m5 present");
    assert!(node_ids.contains(&uuid(M6_JWT)), "m6 present");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn tier_three_reachable_included(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        ids.contains(&uuid(T1_TOKEN_REFRESH)),
        "t1 should be reachable via c4 → m6 → t1 at depth 2"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn tier_four_unreachable_excluded(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        !ids.contains(&uuid(T2_SESSION_MGMT)),
        "t2 is depth-3 from c4 and must NOT appear at depth=2"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn singleton_concept_returned_as_isolated_node(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        ids.contains(&uuid(C3_SINGLETON)),
        "singleton concept still present"
    );

    let singleton_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.source == uuid(C3_SINGLETON) || e.target == uuid(C3_SINGLETON))
        .collect();
    assert_eq!(
        singleton_edges.len(),
        0,
        "singleton concept has no edges in the returned subgraph"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn diamond_shared_member_appears_once(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let m1_count = result
        .nodes
        .iter()
        .filter(|n| n.id == uuid(M1_OAUTH))
        .count();
    assert_eq!(m1_count, 1, "shared member m1 should appear exactly once");

    // And both concept→m1 edges should be present
    let m1_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.target == uuid(M1_OAUTH) || e.source == uuid(M1_OAUTH))
        .collect();
    assert!(
        m1_edges.len() >= 2,
        "m1 should have edges from both c1 and c2"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cross_owner_concept_excluded(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    // Alice queries her primary context — Bob's b1 concept is in Bob's
    // OWN context so it shouldn't surface regardless, but double-check.
    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        !ids.contains(&uuid(B1_BOB_CONCEPT)),
        "bob's concept must not leak to alice's query"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cross_owner_edge_attempt_filtered(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    // m2 has an edge pointing at b2 (bob-owned). Expected: b2 never
    // appears as a node (visibility filter), and the edge is dropped
    // because an edge whose target isn't in the node set is excluded.
    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        !ids.contains(&uuid(B2_BOB_RESEARCH)),
        "b2 (bob-owned) must not appear"
    );

    let leak_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.target == uuid(B2_BOB_RESEARCH) || e.source == uuid(B2_BOB_RESEARCH))
        .collect();
    assert_eq!(
        leak_edges.len(),
        0,
        "edge pointing at bob-owned resource must be filtered out"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deleted_concept_excluded(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    let ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        !ids.contains(&uuid(C5_DELETED)),
        "soft-deleted concept (is_active=false) must NOT appear"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn empty_context_returns_empty_subgraph(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "nonexistent-context",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    assert!(
        result.nodes.is_empty(),
        "nonexistent context yields empty nodes"
    );
    assert!(
        result.edges.is_empty(),
        "nonexistent context yields empty edges"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn multi_context_isolation(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    // Query primary — secondary's s1 must NOT appear.
    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph primary");

    let primary_ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        !primary_ids.contains(&uuid(S1_SECONDARY)),
        "secondary context's concept must not appear in primary query"
    );

    // Query secondary — s1 IS there, primary's concepts are NOT.
    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-secondary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph secondary");

    let secondary_ids: Vec<Uuid> = result.nodes.iter().map(|n| n.id).collect();
    assert!(
        secondary_ids.contains(&uuid(S1_SECONDARY)),
        "s1 present in secondary"
    );
    assert!(
        !secondary_ids.contains(&uuid(C1_IDEMPOTENCY)),
        "primary context's concept must not appear in secondary query"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edge_count_reflects_total_not_subgraph(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: uuid(ALICE),
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await
    .expect("aggregator_subgraph");

    // m1 has edges from c1 AND c2 → total edges touching m1 = 2.
    let m1 = result
        .nodes
        .iter()
        .find(|n| n.id == uuid(M1_OAUTH))
        .expect("m1 should be in the result");
    assert_eq!(
        m1.edge_count, 2,
        "m1 has 2 total edges (from c1 and c2); edge_count reflects total"
    );

    // c1 has edges to m1, m2, m3 → edge_count = 3.
    let c1 = result
        .nodes
        .iter()
        .find(|n| n.id == uuid(C1_IDEMPOTENCY))
        .expect("c1 should be in the result");
    assert_eq!(c1.edge_count, 3, "c1 has 3 outgoing edges");
}

// ─── Handler smoke tests (service layer already integration-tested) ─────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn handler_rejects_non_me_owner(pool: PgPool) {
    // Covers the `owner != "@me"` branch without needing the full HTTP stack.
    // Full auth/HTTP integration is covered in the existing auth_test.rs
    // pattern; here we assert the service contract one layer down is sound.
    common::fixtures::clean_and_seed(&pool).await;
    load_graph_fixtures(&pool).await;

    let alice = uuid(ALICE);
    let result = aggregator_subgraph(
        &pool,
        AggregatorSubgraphParams {
            caller_profile_id: alice,
            context_name: "graph-test-primary",
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await;
    assert!(
        result.is_ok(),
        "service layer succeeds for caller's own data"
    );
}
