//! Beat E — integration tests for the context-graph SQL reads.
#![cfg(all(test, feature = "test-db"))]

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::services::context_graph_service;

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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn atlas_nodes_visible_reports_task_stage(pool: PgPool) {
    // Spec D8: the widened graph_atlas_nodes_visible must surface a task's workflow stage.
    // Stage is stored under property_key `temper-stage` (NOT `stage`) as a jsonb scalar,
    // exactly like the legacy subgraph's `stage_raw`. There is no `set_resource_stage`
    // helper, so the seed writes the kb_properties row directly, mirroring how the shared
    // seed_resource helper writes `doc_type`.
    let (profile, _ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 1).await;

    let task_id: Uuid = sqlx::query_scalar(
        "SELECT owner_id FROM kb_properties WHERE property_key='doc_type'
           AND property_value #>> '{}' = 'task' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("a task");

    // Write the stage the way the codebase stores it: a `temper-stage` kb_properties row
    // holding a jsonb string. Reuse an existing event row to satisfy the FK.
    sqlx::query(
        "INSERT INTO kb_properties \
             (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'temper-stage', to_jsonb('in-progress'::text), \
                 (SELECT id FROM kb_events LIMIT 1), (SELECT id FROM kb_events LIMIT 1))",
    )
    .bind(task_id)
    .execute(&pool)
    .await
    .expect("insert temper-stage property");

    let stage: Option<String> =
        sqlx::query_scalar("SELECT stage FROM graph_atlas_nodes_visible($1, $2)")
            .bind(profile)
            .bind(&[task_id][..])
            .fetch_one(&pool)
            .await
            .expect("node row");

    assert_eq!(stage.as_deref(), Some("in-progress"));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_returns_containers_and_empty_residual(pool: PgPool) {
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let p = context_graph_service::context_panorama(
        &pool,
        ProfileId::from(profile),
        ContextId::from(ctx),
        "doc_type",
        &["goal".to_string()],
        2,
    )
    .await
    .expect("panorama");

    assert_eq!(p.containers.len(), 1);
    assert_eq!(p.containers[0].id, goal);
    assert_eq!(p.containers[0].member_count, 3);
    assert_eq!(p.residual.group_key, "doc_type");
    assert!(
        p.residual.buckets.is_empty(),
        "tray empties on well-edged data"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn composition_from_a_container_includes_its_members(pool: PgPool) {
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let sg =
        context_graph_service::context_composition(&pool, ProfileId::from(profile), &[goal], 1)
            .await
            .expect("composition");

    assert_eq!(sg.nodes.len(), 4, "goal + 3 tasks");
    assert_eq!(sg.edges.len(), 3);
}

// ─── Axum-level handler tests (auth + request-shape) ────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_endpoint_404s_for_an_invisible_context(pool: PgPool) {
    // Deny-as-absence: a stranger asking for a profile-owned context they cannot see gets 404,
    // NEVER a 403 that would confirm the context exists. The context is owned by the seed's
    // "owner" profile; the stranger is a fully-provisioned but unrelated profile.
    let app = common::setup_test_app(pool).await;
    let (_owner, ctx, _goal) = seed_context_with_goal_and_tasks(&app.pool, 1).await;

    let email = format!("stranger-{}@test.com", Uuid::now_v7());
    let stranger = common::fixtures::create_test_profile(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{stranger}"), &email);

    let res = app
        .client
        .get(app.url(&format!(
            "/api/graph/contexts/panorama?context_ref={ctx}&group_by=doc_type"
        )))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("panorama request");

    assert_eq!(
        res.status().as_u16(),
        404,
        "invisible context is absent (404), not forbidden (403)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn composition_endpoint_400s_without_exactly_one_target(pool: PgPool) {
    // parse-don't-validate: the composition drill needs EXACTLY one of `container` / `group`.
    // Neither → 400; both → 400. The context is the caller's own (visible), so the 400 comes
    // from the request-shape check, not the visibility gate.
    let app = common::setup_test_app(pool).await;

    let email = format!("compose-{}@test.com", Uuid::now_v7());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    // Neither container nor group.
    let neither = app
        .client
        .get(app.url(&format!(
            "/api/graph/contexts/composition?context_ref={ctx}"
        )))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("composition (neither) request");
    assert_eq!(
        neither.status().as_u16(),
        400,
        "neither container nor group must be rejected"
    );

    // Both container and group.
    let both = app
        .client
        .get(app.url(&format!(
            "/api/graph/contexts/composition?context_ref={ctx}&container={}&group=doc_type:task",
            Uuid::now_v7()
        )))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("composition (both) request");
    assert_eq!(
        both.status().as_u16(),
        400,
        "supplying both container and group must be rejected"
    );
}
