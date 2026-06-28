#![cfg(feature = "test-db")]

//! End-to-end coverage for the `--stage` filter on resource listing.
//!
//! `temper resource list --type task --context <ctx> --stage <stage>` must
//! return ONLY the tasks at that stage. The filter rides through
//! `ResourceListParams.stage` into the server-side `build_filters`, which
//! adds a `vb.stage = $n` predicate against the `vault_resources_browse`
//! view (`m.managed_meta->>'temper-stage' AS stage`).
//!
//! Tasks are seeded via the API client (`app.client.ingest()`), each carrying
//! a `temper-stage` managed_meta key, then listed through the real
//! `temper-client` → `temper-api` → Postgres path.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_workflow::types::resource::ResourceListParams;

/// Seed a task at a given stage via the API client (cloud-only; no vault
/// files written). Mirrors the `seed_task` helper in
/// `cloud_task_lookup_e2e_test.rs`, narrowed to the fields this test needs.
async fn seed_task(
    client: &temper_client::TemperClient,
    context: &str,
    slug: &str,
    title: &str,
    stage: &str,
) {
    let mut managed = serde_json::Map::new();
    managed.insert("temper-title".to_string(), serde_json::json!(title));
    managed.insert("temper-stage".to_string(), serde_json::json!(stage));
    managed.insert("temper-mode".to_string(), serde_json::json!("build"));
    managed.insert("temper-effort".to_string(), serde_json::json!("small"));

    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: format!("kb://{context}/task/{slug}"),
        context_ref: format!("@me/{context}"),
        doc_type_name: "task".to_string(),
        content_hash: None,
        slug: slug.to_string(),
        content: String::new(),
        metadata: None,
        managed_meta: Some(serde_json::Value::Object(managed)),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
    };
    client
        .ingest()
        .create(&payload)
        .await
        .expect("seed task via client");
}

/// Seed tasks at three stages, then prove the `--stage` filter on
/// `ResourceListParams` selects only the matching tasks server-side:
/// `in-progress` → only the two in-progress, `done` → only the two done
/// (disjoint), and no `--stage` → all five.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_filters_by_stage(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("stagefilter")
        .await
        .expect("create stagefilter context");

    // Two in-progress, two done, one backlog.
    seed_task(
        &app.client,
        "stagefilter",
        "wip-1",
        "WIP One",
        "in-progress",
    )
    .await;
    seed_task(
        &app.client,
        "stagefilter",
        "wip-2",
        "WIP Two",
        "in-progress",
    )
    .await;
    seed_task(&app.client, "stagefilter", "done-1", "Done One", "done").await;
    seed_task(&app.client, "stagefilter", "done-2", "Done Two", "done").await;
    seed_task(
        &app.client,
        "stagefilter",
        "back-1",
        "Backlog One",
        "backlog",
    )
    .await;

    // --stage in-progress → only the two in-progress tasks.
    let in_progress = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(uuid::Uuid::from(context.id).to_string()),
            doc_type_name: Some("task".to_string()),
            stage: Some("in-progress".to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("list --stage in-progress failed");

    // `ResourceRow.slug` is `None` in the substrate (`temper-slug` is a §7-Die key,
    // not persisted); the seed encodes the slug as the last `origin_uri` segment
    // (`kb://<ctx>/task/<slug>`), so derive it from there.
    let mut in_progress_slugs: Vec<&str> = in_progress
        .rows
        .iter()
        .filter_map(|r| r.origin_uri.rsplit('/').next())
        .collect();
    in_progress_slugs.sort_unstable();
    assert_eq!(
        in_progress_slugs,
        vec!["wip-1", "wip-2"],
        "--stage in-progress must return ONLY the in-progress tasks; got {in_progress_slugs:?}"
    );

    // --stage done → only the two done tasks (disjoint from in-progress).
    let done = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(uuid::Uuid::from(context.id).to_string()),
            doc_type_name: Some("task".to_string()),
            stage: Some("done".to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("list --stage done failed");

    let mut done_slugs: Vec<&str> = done
        .rows
        .iter()
        .filter_map(|r| r.origin_uri.rsplit('/').next())
        .collect();
    done_slugs.sort_unstable();
    assert_eq!(
        done_slugs,
        vec!["done-1", "done-2"],
        "--stage done must return ONLY the done tasks; got {done_slugs:?}"
    );

    // No --stage → all five tasks.
    let all = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(uuid::Uuid::from(context.id).to_string()),
            doc_type_name: Some("task".to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("list with no stage filter failed");

    assert_eq!(
        all.rows.len(),
        5,
        "no --stage filter must return all five seeded tasks; got {}",
        all.rows.len()
    );
}
