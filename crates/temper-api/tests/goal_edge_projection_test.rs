#![cfg(feature = "test-db")]
//! Task 019f3d55 — first-class `goal` field with live edge projection.
//!
//! Exercises `DbBackend` directly (like `set_facet_test` / `act_authorship_test`):
//! creating a resource with a `goal` projects a live `advances`→goal edge
//! (`EdgeKind::LeadsTo`, `Polarity::Forward`, label `advances`); updating with
//! `GoalPatch::Set` folds the old edge and asserts a new one (replace-in-place);
//! `GoalPatch::Clear` folds the edge; and `list --goal` filters by the edge. The
//! fold path's `doc_type='goal'` guard is covered by asserting a sibling
//! session→task `advances` edge is NOT disturbed by a goal reassignment.

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, GoalPatch, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceListParams;
use uuid::Uuid;

mod common;

async fn backend_with_context(pool: &PgPool, email: &str) -> (DbBackend, ContextId, ProfileId) {
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, email).await;
    (
        DbBackend::new(pool.clone(), ProfileId::from(profile)),
        ContextId::from(context),
        ProfileId::from(profile),
    )
}

fn create_cmd(
    context: ContextId,
    doctype: &str,
    slug: &str,
    goal: Option<ResourceId>,
) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: doctype.to_string(),
        home: HomeAnchor::Context(context),
        title: format!("Goal test {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal,
        origin_uri: Some(format!("test://goal-{slug}")),
        chunks_packed: None,
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// Count the live (non-folded) `advances`→goal edges from `source` to `target`.
async fn advances_edge_count(pool: &PgPool, source: ResourceId, target: ResourceId) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_edges \
         WHERE source_table = 'kb_resources' AND source_id = $1 \
           AND target_table = 'kb_resources' AND target_id = $2 \
           AND edge_kind = 'leads_to' AND label = 'advances' AND NOT is_folded",
    )
    .bind(Uuid::from(source))
    .bind(Uuid::from(target))
    .fetch_one(pool)
    .await
    .expect("count advances edges")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_with_goal_projects_live_edge(pool: PgPool) {
    let (backend, context, _) = backend_with_context(&pool, "goal-create@example.com").await;

    let goal = backend
        .create_resource(create_cmd(context, "goal", "the-goal", None))
        .await
        .expect("create goal")
        .value
        .id;
    let task = backend
        .create_resource(create_cmd(context, "task", "the-task", Some(goal)))
        .await
        .expect("create task with goal")
        .value
        .id;

    assert_eq!(
        advances_edge_count(&pool, task, goal).await,
        1,
        "create with a goal must project exactly one live advances→goal edge"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_set_replaces_and_clear_retracts_goal(pool: PgPool) {
    let (backend, context, _) = backend_with_context(&pool, "goal-update@example.com").await;

    let goal_a = backend
        .create_resource(create_cmd(context, "goal", "goal-a", None))
        .await
        .expect("create goal a")
        .value
        .id;
    let goal_b = backend
        .create_resource(create_cmd(context, "goal", "goal-b", None))
        .await
        .expect("create goal b")
        .value
        .id;
    // Task starts linked to goal_a.
    let task = backend
        .create_resource(create_cmd(context, "task", "task", Some(goal_a)))
        .await
        .expect("create task")
        .value
        .id;
    assert_eq!(advances_edge_count(&pool, task, goal_a).await, 1);

    // Set → goal_b: the goal_a edge is folded, a goal_b edge asserted (replace-in-place).
    backend
        .update_resource(update_goal(task, Some(GoalPatch::Set(goal_b))))
        .await
        .expect("update set goal_b");
    assert_eq!(
        advances_edge_count(&pool, task, goal_a).await,
        0,
        "replacing the goal must fold the old edge"
    );
    assert_eq!(
        advances_edge_count(&pool, task, goal_b).await,
        1,
        "replacing the goal must assert the new edge"
    );

    // Clear → the goal_b edge is folded, none remain.
    backend
        .update_resource(update_goal(task, Some(GoalPatch::Clear)))
        .await
        .expect("update clear goal");
    assert_eq!(
        advances_edge_count(&pool, task, goal_b).await,
        0,
        "clearing the goal must fold the edge"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn goal_reassignment_leaves_session_advances_edge_untouched(pool: PgPool) {
    // The fold-lookup gates on target doc_type='goal', so a sibling session→task `advances`
    // edge (same kind+label, different target doc-type) must survive a goal reassignment.
    let (backend, context, _) = backend_with_context(&pool, "goal-guard@example.com").await;

    let goal = backend
        .create_resource(create_cmd(context, "goal", "g", None))
        .await
        .expect("goal")
        .value
        .id;
    let task = backend
        .create_resource(create_cmd(context, "task", "t", None))
        .await
        .expect("task")
        .value
        .id;
    // A session that advances the task AND is (unusually) linked to the goal too.
    let session = backend
        .create_resource(create_cmd(context, "session", "s", Some(goal)))
        .await
        .expect("session with goal")
        .value
        .id;
    // Assert the session→task `advances` edge via the public relationship path.
    backend
        .assert_relationship(temper_workflow::operations::AssertRelationship {
            source: session,
            target: task,
            edge_kind: temper_core::types::graph::EdgeKind::LeadsTo,
            polarity: temper_core::types::graph::Polarity::Forward,
            label: "advances".to_string(),
            weight: 1.0,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("assert session→task advances");
    assert_eq!(advances_edge_count(&pool, session, task).await, 1);
    assert_eq!(advances_edge_count(&pool, session, goal).await, 1);

    // Clear the session's goal — only the →goal edge folds; the →task edge stays.
    backend
        .update_resource(update_goal(session, Some(GoalPatch::Clear)))
        .await
        .expect("clear session goal");
    assert_eq!(
        advances_edge_count(&pool, session, goal).await,
        0,
        "the →goal edge must fold"
    );
    assert_eq!(
        advances_edge_count(&pool, session, task).await,
        1,
        "the →task advances edge must NOT be disturbed by a goal clear"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_filters_tasks_by_goal_edge(pool: PgPool) {
    let (backend, context, profile_id) = backend_with_context(&pool, "goal-list@example.com").await;

    let goal = backend
        .create_resource(create_cmd(context, "goal", "goal", None))
        .await
        .expect("goal")
        .value
        .id;
    let linked = backend
        .create_resource(create_cmd(context, "task", "linked", Some(goal)))
        .await
        .expect("linked task")
        .value
        .id;
    // An unlinked task must NOT appear in the goal-filtered list.
    backend
        .create_resource(create_cmd(context, "task", "unlinked", None))
        .await
        .expect("unlinked task");

    let resp = temper_services::backend::substrate_read::list_select(
        &pool,
        profile_id,
        ResourceListParams {
            doc_type_name: Some("task".to_string()),
            goal: Some(Uuid::from(goal)),
            ..Default::default()
        },
    )
    .await
    .expect("list by goal");

    let ids: Vec<Uuid> = resp.rows.iter().map(|r| Uuid::from(r.id)).collect();
    assert_eq!(
        ids,
        vec![Uuid::from(linked)],
        "list --goal must return exactly the task linked to that goal via the advances edge"
    );
}

fn update_goal(resource: ResourceId, goal: Option<GoalPatch>) -> UpdateResource {
    UpdateResource {
        resource,
        title: None,
        slug: None,
        body: None,
        managed_meta: None,
        open_meta: None,
        goal,
        move_to: None,
        context_ref: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}
