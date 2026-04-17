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
#[allow(dead_code)]
const C5_DELETED: &str = "00000000-0000-0000-00c1-000000000005";

const M1_OAUTH: &str = "00000000-0000-0000-00c2-000000000001";
const M2_MIDDLEWARE: &str = "00000000-0000-0000-00c2-000000000002";
const M3_SESSION: &str = "00000000-0000-0000-00c2-000000000003";
const M4_CIRCUIT_DESIGN: &str = "00000000-0000-0000-00c2-000000000004";
const M5_CIRCUIT_IMPL: &str = "00000000-0000-0000-00c2-000000000005";
const M6_JWT: &str = "00000000-0000-0000-00c2-000000000006";

#[allow(dead_code)]
const T1_TOKEN_REFRESH: &str = "00000000-0000-0000-00c3-000000000001";
#[allow(dead_code)]
const T2_SESSION_MGMT: &str = "00000000-0000-0000-00c3-000000000002";

#[allow(dead_code)]
const B1_BOB_CONCEPT: &str = "00000000-0000-0000-00c4-000000000001";
#[allow(dead_code)]
const B2_BOB_RESEARCH: &str = "00000000-0000-0000-00c4-000000000002";

#[allow(dead_code)]
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
