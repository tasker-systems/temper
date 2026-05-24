#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;
use temper_core::types::graph::{EdgeKind, GraphEdgeRow, GraphNeighborRow, GraphTraversalRow};

/// Inserting an edge and querying neighbors returns the expected peer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_insert_edge_and_query_neighbors(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "graph@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;

    // Query outgoing neighbors of r1
    let rows: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT resource_id, label, direction FROM graph_neighbors($1, $2, 'outgoing', '{}')",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("graph_neighbors query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r2);
    assert_eq!(rows[0].1, "extends");
    assert_eq!(rows[0].2, "outgoing");

    // Query incoming neighbors of r2
    let rows: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT resource_id, label, direction FROM graph_neighbors($1, $2, 'incoming', '{}')",
    )
    .bind(profile)
    .bind(r2)
    .fetch_all(&pool)
    .await
    .expect("graph_neighbors incoming");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r1);
    assert_eq!(rows[0].1, "extends");
    assert_eq!(rows[0].2, "incoming");
}

/// Bidirectional neighbor query returns both directions.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_neighbors_both_directions(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "both@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    // r1 → r2 (extends), r3 → r1 (depends_on)
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r3, r1, "depends_on", profile).await;

    // Both directions from r1: should see r2 (outgoing) and r3 (incoming)
    let rows: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT resource_id, direction FROM graph_neighbors($1, $2, 'both', '{}')")
            .bind(profile)
            .bind(r1)
            .fetch_all(&pool)
            .await
            .expect("graph_neighbors both");

    assert_eq!(rows.len(), 2);
    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2));
    assert!(ids.contains(&r3));
}

/// Edge type filter restricts neighbor results.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_neighbors_edge_type_filter(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "filter@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r1, r3, "references", profile).await;

    // Filter to only `leads_to` edges (the kind 'extends' projects to).
    // The p_edge_types filter operates on edge_kind::text post-cutover.
    let rows: Vec<(uuid::Uuid,)> =
        sqlx::query_as("SELECT resource_id FROM graph_neighbors($1, $2, 'both', $3)")
            .bind(profile)
            .bind(r1)
            .bind(vec!["leads_to"])
            .fetch_all(&pool)
            .await
            .expect("graph_neighbors filtered");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r2);
}

/// graph_resource_edges returns edges with peer metadata.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_graph_resource_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "edges@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "depends_on", profile).await;

    let rows: Vec<(uuid::Uuid, String, String, String)> = sqlx::query_as(
        "SELECT peer_resource_id, peer_title, label, direction FROM graph_resource_edges($1, $2)",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("graph_resource_edges");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, r2);
    assert_eq!(rows[0].1, "Doc B");
    assert_eq!(rows[0].2, "depends_on");
    assert_eq!(rows[0].3, "outgoing");
}

// ─── Task 7: Multi-Hop Traversal and Cycle Detection ────────────────────────

/// Multi-hop traversal respects the depth limit.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_multi_hop(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "multihop@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;
    let r4 = common::fixtures::create_test_resource(&pool, profile, "R4", "r4").await;

    // Chain: r1 → r2 → r3 → r4
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r3, r4, "extends", profile).await;

    // Depth 2: should find r2 (depth 1) and r3 (depth 2), but NOT r4
    let rows: Vec<(uuid::Uuid, i32)> =
        sqlx::query_as("SELECT resource_id, depth FROM graph_traverse($1, $2, $3, $4)")
            .bind(profile)
            .bind(vec![r1])
            .bind(2_i32)
            .bind(Vec::<String>::new())
            .fetch_all(&pool)
            .await
            .expect("traverse depth=2");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2), "should find r2 at depth 1");
    assert!(ids.contains(&r3), "should find r3 at depth 2");
    assert!(!ids.contains(&r4), "r4 should be beyond depth 2");
    assert!(!ids.contains(&r1), "seed r1 should not appear in results");

    // Depth 3: should now find r4
    let rows: Vec<(uuid::Uuid, i32)> =
        sqlx::query_as("SELECT resource_id, depth FROM graph_traverse($1, $2, $3, $4)")
            .bind(profile)
            .bind(vec![r1])
            .bind(3_i32)
            .bind(Vec::<String>::new())
            .fetch_all(&pool)
            .await
            .expect("traverse depth=3");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r4), "r4 should appear at depth 3");
}

/// Cycle detection prevents infinite loops during traversal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_cycle_detection(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "cycle@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;

    // Cycle: r1 → r2 → r3 → r1
    common::fixtures::create_test_edge(&pool, r1, r2, "relates_to", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "relates_to", profile).await;
    common::fixtures::create_test_edge(&pool, r3, r1, "relates_to", profile).await;

    // High depth limit — should terminate without infinite loop
    let rows: Vec<(uuid::Uuid, i32)> =
        sqlx::query_as("SELECT resource_id, depth FROM graph_traverse($1, $2, $3, $4)")
            .bind(profile)
            .bind(vec![r1])
            .bind(10_i32)
            .bind(Vec::<String>::new())
            .fetch_all(&pool)
            .await
            .expect("traverse with cycle");

    assert_eq!(
        rows.len(),
        2,
        "should find exactly r2 and r3, no duplicates from cycling"
    );
    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2));
    assert!(ids.contains(&r3));
}

/// Typed filter restricts which edges are followed during traversal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_typed_filter(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "typed@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;
    let r4 = common::fixtures::create_test_resource(&pool, profile, "R4", "r4").await;

    // r1→r2 (extends), r2→r3 (extends), r1→r4 (references)
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r1, r4, "references", profile).await;

    // Filter to `leads_to` only (the kind 'extends' projects to).
    // p_edge_types filters edge_kind::text post-cutover; 'references' maps
    // to `near`, so it stays filtered out.
    let rows: Vec<(uuid::Uuid, i32)> =
        sqlx::query_as("SELECT resource_id, depth FROM graph_traverse($1, $2, $3, $4)")
            .bind(profile)
            .bind(vec![r1])
            .bind(3_i32)
            .bind(vec!["leads_to"])
            .fetch_all(&pool)
            .await
            .expect("traverse typed filter");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(ids.contains(&r2), "r2 reachable via extends");
    assert!(ids.contains(&r3), "r3 reachable via extends chain");
    assert!(
        !ids.contains(&r4),
        "r4 only reachable via references, should be filtered out"
    );
}

/// Path weight decays multiplicatively across hops.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_path_weight_decay(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "weight@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;

    // Insert edges with custom weights via the projection-aware fixture.
    common::fixtures::create_test_edge_weighted(&pool, r1, r2, "extends", 0.8, profile).await;
    common::fixtures::create_test_edge_weighted(&pool, r2, r3, "extends", 0.6, profile).await;

    let rows: Vec<(uuid::Uuid, i32, f64)> = sqlx::query_as(
        "SELECT resource_id, depth, path_weight FROM graph_traverse($1, $2, $3, $4)",
    )
    .bind(profile)
    .bind(vec![r1])
    .bind(3_i32)
    .bind(Vec::<String>::new())
    .fetch_all(&pool)
    .await
    .expect("traverse path weight");

    let r2_row = rows
        .iter()
        .find(|r| r.0 == r2)
        .expect("r2 should be in results");
    let r3_row = rows
        .iter()
        .find(|r| r.0 == r3)
        .expect("r3 should be in results");

    assert!(
        (r2_row.2 - 0.8).abs() < 0.001,
        "r2 path_weight should be ~0.8, got {}",
        r2_row.2
    );
    assert!(
        (r3_row.2 - 0.48).abs() < 0.001,
        "r3 path_weight should be ~0.48 (0.8 * 0.6), got {}",
        r3_row.2
    );
}

// ─── Task 8: Visibility Scoping ─────────────────────────────────────────────

/// Traversal does not cross into resources owned by another profile.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_traverse_visibility_blocks_other_profiles(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let alice = common::fixtures::create_test_profile(&pool, "alice@test.com").await;
    let bob = common::fixtures::create_test_profile(&pool, "bob@test.com").await;

    let a1 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc 1", "alice-1").await;
    let a2 = common::fixtures::create_test_resource(&pool, alice, "Alice Doc 2", "alice-2").await;
    let b1 = common::fixtures::create_test_resource(&pool, bob, "Bob Doc 1", "bob-1").await;

    // a1→b1 (extends), b1→a2 (extends), a1→a2 (relates_to)
    common::fixtures::create_test_edge(&pool, a1, b1, "extends", alice).await;
    common::fixtures::create_test_edge(&pool, b1, a2, "extends", bob).await;
    common::fixtures::create_test_edge(&pool, a1, a2, "relates_to", alice).await;

    // Alice traverses filtering to edge_kind 'leads_to' (the kind 'extends'
    // projects to): should NOT see b1, should NOT reach a2 through b1.
    let rows: Vec<(uuid::Uuid, i32)> =
        sqlx::query_as("SELECT resource_id, depth FROM graph_traverse($1, $2, $3, $4)")
            .bind(alice)
            .bind(vec![a1])
            .bind(5_i32)
            .bind(vec!["leads_to"])
            .fetch_all(&pool)
            .await
            .expect("traverse leads_to as alice");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(!ids.contains(&b1), "alice should NOT see bob's resource b1");
    assert!(
        !ids.contains(&a2),
        "a2 should NOT be reachable through b1 via leads_to"
    );

    // Alice traverses with no type filter: SHOULD see a2 via direct relates_to edge
    let rows: Vec<(uuid::Uuid, i32)> =
        sqlx::query_as("SELECT resource_id, depth FROM graph_traverse($1, $2, $3, $4)")
            .bind(alice)
            .bind(vec![a1])
            .bind(5_i32)
            .bind(Vec::<String>::new())
            .fetch_all(&pool)
            .await
            .expect("traverse unfiltered as alice");

    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.0).collect();
    assert!(
        ids.contains(&a2),
        "alice SHOULD see a2 via direct relates_to edge"
    );
    assert!(!ids.contains(&b1), "alice still should NOT see bob's b1");
}

/// Neighbor queries only return resources visible to the requesting profile.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_neighbors_visibility(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let alice = common::fixtures::create_test_profile(&pool, "alice-n@test.com").await;
    let bob = common::fixtures::create_test_profile(&pool, "bob-n@test.com").await;

    let a1 = common::fixtures::create_test_resource(&pool, alice, "Alice A1", "alice-a1").await;
    let a2 = common::fixtures::create_test_resource(&pool, alice, "Alice A2", "alice-a2").await;
    let b1 = common::fixtures::create_test_resource(&pool, bob, "Bob B1", "bob-b1").await;

    // a1→a2 (extends), a1→b1 (references)
    common::fixtures::create_test_edge(&pool, a1, a2, "extends", alice).await;
    common::fixtures::create_test_edge(&pool, a1, b1, "references", alice).await;

    // Alice's neighbors of a1: should see only a2 (not b1)
    let rows: Vec<(uuid::Uuid,)> =
        sqlx::query_as("SELECT resource_id FROM graph_neighbors($1, $2, 'outgoing', '{}')")
            .bind(alice)
            .bind(a1)
            .fetch_all(&pool)
            .await
            .expect("neighbors as alice");

    assert_eq!(rows.len(), 1, "alice should see only 1 neighbor");
    assert_eq!(rows[0].0, a2, "alice should see a2 but not bob's b1");
}

/// graph_resource_edges respects visibility scoping.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resource_edges_visibility(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let alice = common::fixtures::create_test_profile(&pool, "alice-e@test.com").await;
    let bob = common::fixtures::create_test_profile(&pool, "bob-e@test.com").await;

    let a1 = common::fixtures::create_test_resource(&pool, alice, "Alice A1", "alice-ea1").await;
    let a2 = common::fixtures::create_test_resource(&pool, alice, "Alice A2", "alice-ea2").await;
    let b1 = common::fixtures::create_test_resource(&pool, bob, "Bob B1", "bob-eb1").await;

    // a1→a2 (extends), a1→b1 (references)
    common::fixtures::create_test_edge(&pool, a1, a2, "extends", alice).await;
    common::fixtures::create_test_edge(&pool, a1, b1, "references", alice).await;

    // graph_resource_edges as alice: should only return the a2 edge
    let rows: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT peer_resource_id, label FROM graph_resource_edges($1, $2)")
            .bind(alice)
            .bind(a1)
            .fetch_all(&pool)
            .await
            .expect("resource_edges as alice");

    assert_eq!(rows.len(), 1, "alice should see only 1 edge");
    assert_eq!(rows[0].0, a2, "the visible edge should point to a2");
}

// ─── Task 9: Constraint Enforcement ─────────────────────────────────────────

/// Self-referential edges are rejected by the check constraint.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_self_edge_rejected(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "self@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;

    // Direct INSERT to assert the table CHECK still fires. asserted_by_event_id
    // / last_event_id reference any visible event row; reuse a synthesized
    // assertion event via the fixture's path is heavier than needed, so we
    // synthesize one inline.
    let event_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_events (id, event_type_id, profile_id, device_id, topic_id, scope_id,
                                  payload, metadata, "references", correlation_id, occurred_at, created)
           SELECT $1, (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'),
                  $2, 'fixture', '019e3d6f-2300-7000-8000-000000000050',
                  '019e3d6f-2300-7000-8000-000000000010', '{}'::jsonb,
                  jsonb_build_object('source','fixture'), '[]'::jsonb, $1, now(), now()"#,
    )
    .bind(event_id)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("synth event for self-edge test");

    let result = sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id,
             edge_kind, polarity, label, weight,
             asserted_by_event_id, last_event_id, is_folded)
           VALUES (gen_random_uuid(), $1, $1, 'leads_to'::edge_kind,
                   'inverse'::edge_polarity, 'extends', 1.0, $2, $2, false)"#,
    )
    .bind(r1)
    .bind(event_id)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "self-referential edge should be rejected");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("chk_no_self_edge"),
        "error should reference chk_no_self_edge constraint, got: {err_msg}"
    );
}

/// Duplicate edges (same source, target, type) are rejected by unique constraint.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_duplicate_edge_rejected(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "dup@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;

    // First edge succeeds
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;

    // Synthesize an assertion event so the FK on asserted_by_event_id resolves
    // when we attempt the direct INSERT below.
    let event_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_events (id, event_type_id, profile_id, device_id, topic_id, scope_id,
                                  payload, metadata, "references", correlation_id, occurred_at, created)
           SELECT $1, (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'),
                  $2, 'fixture', '019e3d6f-2300-7000-8000-000000000050',
                  '019e3d6f-2300-7000-8000-000000000010', '{}'::jsonb,
                  jsonb_build_object('source','fixture'), '[]'::jsonb, $1, now(), now()"#,
    )
    .bind(event_id)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("synth event for dup-edge test");

    // Duplicate (same source, target, edge_kind, label, polarity) should fail.
    let result = sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id,
             edge_kind, polarity, label, weight,
             asserted_by_event_id, last_event_id, is_folded)
           VALUES (gen_random_uuid(), $1, $2, 'leads_to'::edge_kind,
                   'inverse'::edge_polarity, 'extends', 1.0, $3, $3, false)"#,
    )
    .bind(r1)
    .bind(r2)
    .bind(event_id)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "duplicate edge should be rejected");

    // Different label on same pair should succeed (uq_resource_edge includes label).
    let result = sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id,
             edge_kind, polarity, label, weight,
             asserted_by_event_id, last_event_id, is_folded)
           VALUES (gen_random_uuid(), $1, $2, 'near'::edge_kind,
                   'forward'::edge_polarity, 'references', 1.0, $3, $3, false)"#,
    )
    .bind(r1)
    .bind(r2)
    .bind(event_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "different label on same pair should succeed"
    );
}

/// Deleting a resource cascades to remove all edges touching it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_edge_cascade_on_resource_delete(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "cascade@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "R1", "r1").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "R2", "r2").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "R3", "r3").await;

    // r1→r2 (extends), r2→r3 (depends_on)
    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, r2, r3, "depends_on", profile).await;

    // Delete r2
    sqlx::query("DELETE FROM kb_resources WHERE id = $1")
        .bind(r2)
        .execute(&pool)
        .await
        .expect("delete r2");

    // Count remaining edges — both should be gone
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM kb_resource_edges WHERE source_resource_id = ANY($1) OR target_resource_id = ANY($1)",
    )
    .bind(vec![r1, r2, r3])
    .fetch_one(&pool)
    .await
    .expect("count edges");

    assert_eq!(count.0, 0, "all edges touching r2 should cascade on delete");
}

/// list_resource_edges returns correct edges through the service layer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resource_edges_service(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "list-edges@test.com").await;
    let r_b = common::fixtures::create_test_resource(&pool, profile, "Base Doc", "base-doc").await;
    let r_a =
        common::fixtures::create_test_resource(&pool, profile, "Dependent Doc", "dependent-doc")
            .await;

    // A depends_on B
    common::fixtures::create_test_edge(&pool, r_a, r_b, "depends_on", profile).await;

    // Call the service function for resource A
    let edges = temper_api::services::edge_service::list_resource_edges(&pool, profile, r_a)
        .await
        .expect("list_resource_edges for A");

    assert_eq!(edges.len(), 1, "A should have 1 edge");
    assert_eq!(edges[0].label, "depends_on");
    assert_eq!(edges[0].edge_kind, EdgeKind::LeadsTo);
    assert_eq!(edges[0].direction, "outgoing");
    assert_eq!(edges[0].peer_slug, "base-doc");
    assert_eq!(edges[0].peer_resource_id, r_b);

    // Call the service function for resource B (incoming edge)
    let edges_b = temper_api::services::edge_service::list_resource_edges(&pool, profile, r_b)
        .await
        .expect("list_resource_edges for B");

    assert_eq!(edges_b.len(), 1, "B should have 1 incoming edge");
    assert_eq!(edges_b[0].label, "depends_on");
    assert_eq!(edges_b[0].edge_kind, EdgeKind::LeadsTo);
    assert_eq!(edges_b[0].direction, "incoming");
    assert_eq!(edges_b[0].peer_slug, "dependent-doc");
    assert_eq!(edges_b[0].peer_resource_id, r_a);
}

/// FromRow alignment: verify Rust types deserialize correctly from SQL function results.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_from_row_alignment(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "fromrow@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    common::fixtures::create_test_edge(&pool, r1, r2, "extends", profile).await;

    // Verify GraphNeighborRow FromRow alignment
    let neighbors: Vec<GraphNeighborRow> = sqlx::query_as(
        "SELECT resource_id, edge_kind, polarity, label, direction, weight FROM graph_neighbors($1, $2, 'both', '{}')",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("GraphNeighborRow deserialization");

    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].resource_id, r2);
    assert_eq!(neighbors[0].edge_kind, EdgeKind::LeadsTo);
    assert_eq!(neighbors[0].label, "extends");
    assert_eq!(neighbors[0].direction, "outgoing");

    // Verify GraphEdgeRow FromRow alignment
    let edges: Vec<GraphEdgeRow> = sqlx::query_as(
        "SELECT edge_id, peer_resource_id, peer_title, peer_slug, edge_kind, polarity, label, direction, weight, created FROM graph_resource_edges($1, $2)",
    )
    .bind(profile)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .expect("GraphEdgeRow deserialization");

    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].peer_resource_id, r2);
    assert_eq!(edges[0].peer_title, "Doc B");
    assert_eq!(edges[0].edge_kind, EdgeKind::LeadsTo);
    assert_eq!(edges[0].label, "extends");

    // Verify GraphTraversalRow FromRow alignment
    let traversal: Vec<GraphTraversalRow> = sqlx::query_as(
        "SELECT resource_id, depth, path, edge_kind, polarity, label, from_resource_id, path_weight FROM graph_traverse($1, $2, 3, '{}')",
    )
    .bind(profile)
    .bind(vec![r1])
    .fetch_all(&pool)
    .await
    .expect("GraphTraversalRow deserialization");

    assert_eq!(traversal.len(), 1);
    assert_eq!(traversal[0].resource_id, r2);
    assert_eq!(traversal[0].depth, 1);
    assert_eq!(traversal[0].edge_kind, Some(EdgeKind::LeadsTo));
    assert_eq!(traversal[0].label.as_deref(), Some("extends"));
}
