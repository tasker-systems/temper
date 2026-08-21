//! Chunk B — the traversal read (spec §5.2).
//!
//! > **A composition grounds you. It does not navigate you.**
//!
//! Grounding sets a space; this walks inside it without re-running the question. What these tests
//! hold down is that the walk is *undirected*, *visibility-scoped*, and *induced* — and that it is
//! the SAME body chunk A uses, not a second walk that can drift from it.
#![cfg(all(test, feature = "test-db"))]

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::services::{context_graph_service, graph_service};

mod common;
use common::{
    seed_contains_edge, seed_context_with_goal_and_tasks, seed_context_with_two_hop_session,
    seed_genesis_event, seed_profile, seed_resource,
};

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_walk_is_undirected(pool: PgPool) {
    // THE POINT OF RETIRING `graph_traverse`. That function walked FORWARD ONLY — its base arm
    // matched `e.source_id = ANY(p_seed_ids)` and its recursive arm joined `e.source_id =
    // w.target_id` — so hopping from a leaf reached nothing, because the edge points AT the leaf.
    // A reader who clicks a task should reach its goal; direction is the assertion's grammar, not
    // the reader's.
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let tasks: Vec<Uuid> =
        sqlx::query_scalar("SELECT target_id FROM kb_edges WHERE source_id = $1 AND NOT is_folded")
            .bind(goal)
            .fetch_all(&pool)
            .await
            .expect("tasks");
    let leaf = tasks[0];

    // Hop from the LEAF — the end an arrow points at.
    let sub = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[leaf], 1)
        .await
        .expect("traverse");

    assert!(
        sub.nodes.iter().any(|n| n.id == goal),
        "hopping from a task must reach its goal — the walk follows edges in both directions"
    );
    assert!(
        sub.edges
            .iter()
            .any(|e| e.target == leaf && e.source == goal),
        "and the edge is returned in its asserted orientation"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn depth_is_the_reach_and_is_clamped(pool: PgPool) {
    // goal -> task -> session. At one hop from the goal the session is out of reach; at two it is
    // in. Depth is the parameter that makes a hop mean something.
    let (profile, _ctx, session) = seed_context_with_two_hop_session(&pool).await;
    let goal: Uuid = sqlx::query_scalar(
        "SELECT source_id FROM kb_edges e WHERE NOT EXISTS \
         (SELECT 1 FROM kb_edges p WHERE p.target_id = e.source_id) LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("goal");

    let one = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[goal], 1)
        .await
        .expect("depth 1");
    assert!(
        !one.nodes.iter().any(|n| n.id == session),
        "the session is two hops away and must not appear at depth 1"
    );

    let two = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[goal], 2)
        .await
        .expect("depth 2");
    assert!(
        two.nodes.iter().any(|n| n.id == session),
        "and must appear at depth 2"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn depth_clamps_to_three(pool: PgPool) {
    // Needs a chain LONGER than the clamp, or the assertion is vacuous: on a two-hop fixture,
    // depth 3 and depth 9999 return the same thing whether clamping happens or not. This chain is
    // six long, so the clamp is the only reason the walk stops.
    let (profile, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 0).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    let mut chain: Vec<Uuid> = Vec::new();
    for i in 0..7 {
        chain.push(seed_resource(&pool, ctx, profile, event, &format!("Link {i}"), "note").await);
    }
    for pair in chain.windows(2) {
        seed_contains_edge(&pool, pair[0], pair[1], ctx, event).await;
    }
    let head = chain[0];

    let at_three = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[head], 3)
        .await
        .expect("depth 3");
    let absurd = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[head], 9_999)
        .await
        .expect("clamped, not refused");

    assert_eq!(
        at_three.nodes.len(),
        4,
        "head plus three hops — the walk stops at the clamp, not at the end of the chain"
    );
    assert!(
        !at_three.nodes.iter().any(|n| n.id == chain[6]),
        "the far end of a seven-link chain is out of reach"
    );
    assert_eq!(
        absurd.nodes.len(),
        at_three.nodes.len(),
        "an absurd depth returns exactly what depth 3 returns"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_seed_that_reaches_nothing_still_renders(pool: PgPool) {
    // A hop that silently drops the thing you hopped FROM leaves the reader nowhere — they would
    // be looking at a screen with no relationship to the mark they clicked.
    let (profile, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 2).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    let island = seed_resource(&pool, ctx, profile, event, "Island", "note").await;

    let sub = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[island], 3)
        .await
        .expect("traverse");

    assert_eq!(sub.nodes.len(), 1, "just the seed");
    assert_eq!(sub.nodes[0].id, island);
    assert!(sub.edges.is_empty());
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_returned_edge_dangles(pool: PgPool) {
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 6).await;

    let sub = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[goal], 3)
        .await
        .expect("traverse");

    let present: std::collections::HashSet<Uuid> = sub.nodes.iter().map(|n| n.id).collect();
    for e in &sub.edges {
        assert!(present.contains(&e.source) && present.contains(&e.target));
    }
    assert_eq!(sub.edges.len(), 6, "every edge among the reached set");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn folded_edges_are_not_traversable(pool: PgPool) {
    // A retracted assertion is not a road. Traversing one would let a reader walk into material
    // via a relationship somebody took back.
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 1).await;
    let event = seed_genesis_event(&pool, profile, ctx).await;
    let ghost = seed_resource(&pool, ctx, profile, event, "Ghost", "note").await;
    seed_contains_edge(&pool, goal, ghost, ctx, event).await;
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE target_id = $1")
        .bind(ghost)
        .execute(&pool)
        .await
        .expect("fold");

    let sub = graph_service::traversal_slice(&pool, ProfileId::from(profile), &[goal], 3)
        .await
        .expect("traverse");

    assert!(
        !sub.nodes.iter().any(|n| n.id == ghost),
        "a folded edge is not a road out of the goal"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deny_direction_a_stranger_traverses_nothing(pool: PgPool) {
    // Deny-as-absence, and specifically: passing a seed you cannot see must not confirm it exists.
    let (_owner, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 4).await;
    let stranger = seed_profile(&pool, "stranger").await;

    let sub = graph_service::traversal_slice(&pool, ProfileId::from(stranger), &[goal], 3)
        .await
        .expect("traverse");

    assert!(
        !sub.nodes.iter().any(|n| n.id == goal),
        "the seed itself is absent, not present-but-empty"
    );
    assert!(sub.edges.is_empty());
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn empty_seeds_are_refused(pool: PgPool) {
    let (profile, _ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 2).await;
    assert!(
        graph_service::traversal_slice(&pool, ProfileId::from(profile), &[], 1)
            .await
            .is_err(),
        "a traversal with nothing to traverse from is a caller error"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_context_door_and_the_traversal_read_cannot_drift(pool: PgPool) {
    // The two had identical bodies. `context_composition` is now a thin alias, and this pins it
    // there so the duplication cannot quietly return before chunk E deletes the context door.
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 5).await;

    for depth in 1..=3_i32 {
        let via_context = context_graph_service::context_composition(
            &pool,
            ProfileId::from(profile),
            &[goal],
            depth,
        )
        .await
        .expect("context composition");
        let via_traversal =
            graph_service::traversal_slice(&pool, ProfileId::from(profile), &[goal], depth)
                .await
                .expect("traversal");

        assert_eq!(
            via_context.nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            via_traversal.nodes.iter().map(|n| n.id).collect::<Vec<_>>(),
            "same nodes at depth {depth}"
        );
        assert_eq!(
            via_context.edges.iter().map(|e| e.id).collect::<Vec<_>>(),
            via_traversal.edges.iter().map(|e| e.id).collect::<Vec<_>>(),
            "same edges at depth {depth}"
        );
    }
}
