#![cfg(feature = "test-db")]

//! End-to-end coverage for the `--status` filter on goal listing.
//!
//! `temper resource list --type goal --context <ctx> --status <status>` must return ONLY
//! the goals at that status. Until 2026-07-28 it returned **every** goal and accepted
//! values outside the enum without error: `ResourceListParams` carried no `status` field,
//! so the flag was plumbed through the CLI and dropped before SQL.
//!
//! That is worse than a missing filter. A missing filter is visible; this one made
//! *presence* the lie — a row returned under `--status active` was no evidence at all of
//! being active, and a status-truth pass over the goal list on 2026-07-27 reported
//! "43 goals active" on its strength when three were already `completed`.
//!
//! The filter now rides `ResourceListParams.status` into `filtered_visible_page`, which
//! adds a `wp.status = $n` predicate against the `kb_resource_workflow_props` pivot view
//! (migration 20260728000010 added the `temper-status` key to it) — structurally
//! identical to how `stage` is filtered, so the two cannot drift.
//!
//! **Both halves matter and they fail differently.** `filters_by_status` catches a filter
//! that returns the wrong subset; `rejects_status_outside_the_enum` catches the actual
//! historical defect, which was *accepting everything*. A suite with only the first would
//! have gone green against the broken build for any single-status fixture.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_workflow::types::resource::ResourceListParams;

/// Seed a goal at a given status via the API client (cloud-only; no vault files written).
/// Mirrors `seed_task` in `resource_list_stage_filter_test.rs`, narrowed to goals.
async fn seed_goal(
    client: &temper_client::TemperClient,
    context: &str,
    slug: &str,
    title: &str,
    status: &str,
) {
    let mut managed = serde_json::Map::new();
    managed.insert("temper-status".to_string(), serde_json::json!(status));

    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("kb://{context}/goal/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "goal".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: Some(serde_json::Value::Object(managed)),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    client
        .ingest()
        .create(&payload)
        .await
        .expect("seed goal via client");
}

/// Seed goals at three statuses, then prove `--status` selects only the matching goals:
/// `active` → only the two active, `completed` → only the two completed (disjoint), and
/// no `--status` → all five.
///
/// The five-seeded / two-returned shape is the point. Against the pre-fix build every one
/// of these assertions returned all five, so the counts here are what separate a real
/// filter from a dropped one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_filters_by_status(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("statusfilter", None)
        .await
        .expect("create statusfilter context");

    // Two active, two completed, one paused.
    seed_goal(&app.client, "statusfilter", "act-1", "Active One", "active").await;
    seed_goal(&app.client, "statusfilter", "act-2", "Active Two", "active").await;
    seed_goal(
        &app.client,
        "statusfilter",
        "comp-1",
        "Completed One",
        "completed",
    )
    .await;
    seed_goal(
        &app.client,
        "statusfilter",
        "comp-2",
        "Completed Two",
        "completed",
    )
    .await;
    seed_goal(
        &app.client,
        "statusfilter",
        "paus-1",
        "Paused One",
        "paused",
    )
    .await;

    let list_status = |status: Option<&'static str>| {
        let ctx_id = uuid::Uuid::from(context.id).to_string();
        let client = &app.client;
        async move {
            client
                .resources()
                .list(&ResourceListParams {
                    context_ref: Some(ctx_id),
                    doc_type_name: Some("goal".to_string()),
                    status: status.map(str::to_string),
                    limit: Some(50),
                    ..Default::default()
                })
                .await
        }
    };

    // `ResourceRow.slug` is `None` in the substrate (`temper-slug` is a §7-Die key, not
    // persisted); the seed encodes the slug as the last `origin_uri` segment.
    fn slugs(rows: &[temper_workflow::types::resource::ResourceRow]) -> Vec<&str> {
        let mut s: Vec<&str> = rows
            .iter()
            .filter_map(|r| r.origin_uri.rsplit('/').next())
            .collect();
        s.sort_unstable();
        s
    }

    let active = list_status(Some("active"))
        .await
        .expect("list --status active failed");
    assert_eq!(
        slugs(&active.rows),
        vec!["act-1", "act-2"],
        "--status active must return ONLY the active goals; got {:?}",
        slugs(&active.rows)
    );

    let completed = list_status(Some("completed"))
        .await
        .expect("list --status completed failed");
    assert_eq!(
        slugs(&completed.rows),
        vec!["comp-1", "comp-2"],
        "--status completed must return ONLY the completed goals; got {:?}",
        slugs(&completed.rows)
    );

    let all = list_status(None)
        .await
        .expect("list with no status filter failed");
    assert_eq!(
        all.rows.len(),
        5,
        "no --status filter must return all five seeded goals; got {}",
        all.rows.len()
    );
}

/// A `status` outside the goal enum is rejected, not silently matched against nothing.
///
/// This is the guard for the ACTUAL historical defect. `--status bogus-value` returned all
/// 43 goals on the pre-fix build; a naive fix that only adds the SQL predicate would turn
/// that into a confident empty page, which is still a lie — the caller cannot distinguish
/// "no goals are bogus-value" from "bogus-value is not a status".
///
/// The check lives in `filtered_visible_page`, so it holds for every door (CLI, MCP, raw
/// HTTP), not only the surface that happens to validate client-side.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_rejects_status_outside_the_enum(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("statusreject", None)
        .await
        .expect("create statusreject context");

    seed_goal(&app.client, "statusreject", "act-1", "Active One", "active").await;

    let result = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(uuid::Uuid::from(context.id).to_string()),
            doc_type_name: Some("goal".to_string()),
            status: Some("bogus-value".to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await;

    let err = result.expect_err(
        "a status outside the enum must be REJECTED; an empty page is not enough, \
         because it is indistinguishable from a valid status with no matches",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("bogus-value") || msg.to_lowercase().contains("status"),
        "the rejection must name the offending value or field so the caller can fix the \
         typo; got: {msg}"
    );
}
