#![cfg(feature = "test-db")]

//! End-to-end coverage for the first-class `goal` field (task 019f3d55).
//!
//! Drives the real `temper-client` → `temper-api` → Postgres path (the same wire
//! the CLI uses): a resource created with `IngestPayload.goal` projects a live
//! `advances`→goal edge server-side, and `list --goal <ref>` (via
//! `ResourceListParams.goal`) returns only the linked resources. Update set/clear
//! ride the `ResourceUpdateRequest` `goal`/`clear_goal` tri-state through
//! `PATCH /api/resources/{id}`, and the list filter reflects each change.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_workflow::types::resource::{ResourceListParams, ResourceUpdateRequest};
use uuid::Uuid;

/// Seed a resource via the API client, optionally linked to a goal id. Returns the
/// created resource id.
async fn seed(
    client: &temper_client::TemperClient,
    context: &str,
    doc_type: &str,
    slug: &str,
    goal: Option<Uuid>,
) -> Uuid {
    let mut managed = serde_json::Map::new();
    managed.insert("temper-mode".to_string(), serde_json::json!("build"));
    managed.insert("temper-effort".to_string(), serde_json::json!("small"));

    let payload = IngestPayload {
        title: format!("Goal e2e {slug}"),
        origin_uri: format!("kb://{context}/{doc_type}/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: doc_type.to_string(),
        goal,
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: Some(serde_json::Value::Object(managed)),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    let row = client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource via client");
    Uuid::from(row.id)
}

/// The slugs of the tasks returned by `list --goal <goal>`, derived from
/// `origin_uri` (`temper-slug` is a §7-Die key, not persisted on the row).
async fn tasks_for_goal(
    client: &temper_client::TemperClient,
    context_id: Uuid,
    goal: Uuid,
) -> Vec<String> {
    let resp = client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(context_id.to_string()),
            doc_type_name: Some("task".to_string()),
            goal: Some(goal),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("list --goal failed");
    let mut slugs: Vec<String> = resp
        .rows
        .iter()
        .filter_map(|r| r.origin_uri.rsplit('/').next().map(str::to_string))
        .collect();
    slugs.sort();
    slugs
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn goal_create_list_update_clear_roundtrip(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("goalfilter", None)
        .await
        .expect("create goalfilter context");
    let context_id = Uuid::from(context.id);

    let goal = seed(&app.client, "goalfilter", "goal", "the-goal", None).await;

    // A task created WITH the goal is linked; an unlinked one is not.
    seed(&app.client, "goalfilter", "task", "linked", Some(goal)).await;
    let unlinked = seed(&app.client, "goalfilter", "task", "unlinked", None).await;

    assert_eq!(
        tasks_for_goal(&app.client, context_id, goal).await,
        vec!["linked"],
        "create --goal must project an edge the list filter finds"
    );

    // Set the unlinked task's goal via the PATCH wire (`goal`) → now both linked.
    app.client
        .resources()
        .update(
            unlinked,
            &ResourceUpdateRequest {
                goal: Some(goal),
                ..Default::default()
            },
        )
        .await
        .expect("update set goal");
    assert_eq!(
        tasks_for_goal(&app.client, context_id, goal).await,
        vec!["linked", "unlinked"],
        "update --goal must add the resource to the goal's filtered set"
    );

    // Clear it again via the PATCH wire (`clear_goal`) → back to just linked.
    app.client
        .resources()
        .update(
            unlinked,
            &ResourceUpdateRequest {
                clear_goal: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("update clear goal");
    assert_eq!(
        tasks_for_goal(&app.client, context_id, goal).await,
        vec!["linked"],
        "update --clear-goal must fold the edge and drop the resource from the set"
    );
}
