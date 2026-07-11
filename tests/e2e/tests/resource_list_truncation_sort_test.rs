#![cfg(feature = "test-db")]

//! End-to-end coverage for the list **truncation signal** and the **sort** /
//! **title-filter** mechanics — the tooling half of the "agents assert wrong
//! backlog/status from a silently truncated list" footgun.
//!
//! Three properties, all through the real `temper-client` → `temper-api` →
//! Postgres path:
//!   1. `total` reflects the FULL filtered match count even when `limit` caps
//!      the returned page — so a caller can always tell there is more (the CLI
//!      surfaces this as a `truncated` flag + stderr hint).
//!   2. `sort` + `order` reorder the set (title ascending is distinct from the
//!      default updated-desc).
//!   3. `q` (the `--title-contains` filter) narrows by title substring.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

/// Seed a task via the API client (cloud-only; no vault files). Mirrors
/// `resource_list_stage_filter_test`'s helper.
async fn seed_task(client: &temper_client::TemperClient, context: &str, slug: &str, title: &str) {
    let mut managed = serde_json::Map::new();
    managed.insert("temper-stage".to_string(), serde_json::json!("backlog"));
    managed.insert("temper-mode".to_string(), serde_json::json!("build"));
    managed.insert("temper-effort".to_string(), serde_json::json!("small"));

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("kb://{context}/task/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
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
        .expect("seed task via client");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_truncation_signal_and_sort_and_title_filter(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("listmechanics", None)
        .await
        .expect("create listmechanics context");
    let ctx_ref = uuid::Uuid::from(context.id).to_string();

    // Five tasks. Seed order (oldest → newest) is deliberately NOT alphabetical
    // so title-asc is provably distinct from the default updated-desc.
    for (slug, title) in [
        ("zebra", "Zebra task"),
        ("alpha", "Alpha task"),
        ("mango", "Mango task"),
        ("delta", "Delta task"),
        ("bravo", "Bravo maintenance task"),
    ] {
        seed_task(&app.client, "listmechanics", slug, title).await;
    }

    // (1) Truncation signal: a limit of 2 caps the page, but `total` still
    // reports all five — the material the CLI turns into `truncated: true`.
    let page = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(ctx_ref.clone()),
            doc_type_name: Some("task".to_string()),
            limit: Some(2),
            ..Default::default()
        })
        .await
        .expect("limited list failed");
    assert_eq!(page.rows.len(), 2, "limit=2 must cap the page to two rows");
    assert_eq!(
        page.total, 5,
        "total must report the FULL filtered count (5), not the page size — this is the \
         truncation signal a caller reasons over; got {}",
        page.total
    );
    assert!(
        (page.rows.len() as i64) < page.total,
        "rows.len() < total is the definition of a truncated page"
    );

    // (2) Sort: title ascending orders Alpha, Bravo, Delta, Mango, Zebra —
    // distinct from the default updated-desc (Bravo, Delta, Mango, Alpha, Zebra).
    let sorted = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(ctx_ref.clone()),
            doc_type_name: Some("task".to_string()),
            sort: Some(ResourceSortField::Title),
            order: Some(SortOrder::Asc),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("sorted list failed");
    let titles: Vec<&str> = sorted.rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Alpha task",
            "Bravo maintenance task",
            "Delta task",
            "Mango task",
            "Zebra task",
        ],
        "sort=title order=asc must return rows alphabetically by title; got {titles:?}"
    );

    // (3) Title filter (`q` ← `--title-contains`): narrows to the substring match.
    let filtered = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(ctx_ref.clone()),
            doc_type_name: Some("task".to_string()),
            q: Some("maintenance".to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("title-filtered list failed");
    let filtered_titles: Vec<&str> = filtered.rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        filtered_titles,
        vec!["Bravo maintenance task"],
        "q=maintenance must return ONLY the title containing it; got {filtered_titles:?}"
    );
    assert_eq!(
        filtered.total, 1,
        "filtered total must count only matching rows"
    );
}
