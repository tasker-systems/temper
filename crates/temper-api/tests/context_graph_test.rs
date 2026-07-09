//! Beat E — integration tests for the context-graph SQL reads.
#![cfg(all(test, feature = "test-db"))]

use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::{seed_context_with_goal_and_tasks, seed_profile};

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn containers_count_members_regardless_of_edge_label_or_direction(pool: PgPool) {
    // The parent_of -> advances backfill (20260709000005) REVERSES direction and changes
    // edge_kind. Member counts must be invariant across both representations, or every
    // territory silently empties the day that migration lands. (spec §2, finding 1)
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let rows: Vec<(Uuid, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, label, member_count FROM graph_context_containers($1, $2, $3, $4)",
    )
    .bind(profile)
    .bind(ctx)
    .bind(&["goal"][..])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("containers");

    assert_eq!(rows.len(), 1, "one goal container");
    assert_eq!(rows[0].0, goal);
    assert_eq!(rows[0].2, 3, "three tasks reachable at depth 2");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn containers_deny_direction_invisible_resources_are_absent(pool: PgPool) {
    // Deny direction: a resource visible to A but not B must not appear in B's counts,
    // and its EXISTENCE must not leak through the member_count either.
    let (owner, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let stranger = seed_profile(&pool, "stranger").await;

    let rows: Vec<(Uuid, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, label, member_count FROM graph_context_containers($1, $2, $3, $4)",
    )
    .bind(stranger)
    .bind(ctx)
    .bind(&["goal"][..])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("containers");

    assert!(
        rows.is_empty(),
        "stranger sees no containers, not empty ones"
    );
    let _ = (owner, goal);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn residual_counts_group_by_doc_type_and_exclude_contained(pool: PgPool) {
    let (profile, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT group_value, member_count FROM graph_context_residual_counts($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(ctx)
    .bind("doc_type")
    .bind(&["goal"][..])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("residual counts");

    // The 3 tasks reach the goal, so nothing is residual.
    assert!(
        rows.is_empty(),
        "contained tasks are not residual, got {rows:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn container_counts_survive_the_parent_of_to_advances_conversion(pool: PgPool) {
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    async fn member_count(pool: &PgPool, profile: Uuid, ctx: Uuid) -> i32 {
        sqlx::query_scalar::<_, i32>(
            "SELECT member_count FROM graph_context_containers($1, $2, $3, $4)",
        )
        .bind(profile)
        .bind(ctx)
        .bind(&["goal"][..])
        .bind(2_i32)
        .fetch_one(pool)
        .await
        .expect("count")
    }

    let before = member_count(&pool, profile, ctx).await;

    // Run the sibling session's conversion: parent_of goal->task becomes advances task->goal
    // (reversed direction, different edge_kind). The undirected, label-blind walk is invariant.
    let converted: i32 = sqlx::query_scalar("SELECT backfill_goal_parent_of_to_advances()")
        .fetch_one(&pool)
        .await
        .expect("backfill");
    assert_eq!(converted, 3, "backfill converts all three parent_of edges");

    assert_eq!(
        member_count(&pool, profile, ctx).await,
        before,
        "undirected, label-blind walk is invariant across the parent_of -> advances conversion"
    );
    let _ = goal;
}
