#![cfg(feature = "artifact-tests")]
//! WS6 chunk 3 — parity-read harness. Each test ports one production read to `temper_next.*` and
//! asserts identical output for the same logical query over the synthesized prod-shape fixture.
//! Isolated ephemeral DB per test via `#[sqlx::test(migrator = ...)]` (NOT the psql-reset write-path
//! group). Runtime schema-qualified reads throughout (same discipline as `synthesis::source`).
mod common;

use std::collections::BTreeMap;

use common::fixture_ids;
use temper_next::readback::{self, ResolvedIds};

/// Smoke test: the chunk-3 harness composes. Seed the prod-shape fixture into `public.*`, synthesize
/// into `temper_next.*`, then build the `old↔new` id bimap by `origin_uri`. The synthesized id set is
/// the 4 ACTIVE fixture resources (R4 `temper://fixture/deleted-doc` is excluded, §0 active-only), and
/// the bimap round-trips for a known fixture resource.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn parity_harness_setup_synthesizes(pool: sqlx::PgPool) {
    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");

    let new_ids: Vec<_> = ids.new_ids().collect();
    assert!(!new_ids.is_empty(), "synthesized id set is non-empty");
    assert_eq!(
        ids.len(),
        4,
        "4 active fixture resources synthesized (R4 deleted-doc excluded, §0 active-only)"
    );

    // The bimap round-trips for a known fixture resource (R2, the task).
    let new = ids
        .to_new(fixture_ids::RESOURCE_TASK)
        .expect("R2 (task) has a synthesized id");
    assert_eq!(
        ids.to_old(new),
        Some(fixture_ids::RESOURCE_TASK),
        "old→new→old round-trips for R2"
    );
    assert_eq!(
        ids.origin_uri_for_new(new),
        Some("temper://fixture/task-doc"),
        "the synthesized id resolves back to R2's origin_uri"
    );
}

/// The projected list-row tuple compared across the two read paths, keyed by `origin_uri`.
type ListProjection = (
    String,         // title
    String,         // doc_type
    Option<String>, // stage
    Option<String>, // mode
    Option<String>, // effort
);

/// §9 — `list` parity. Seed + synthesize the prod-shape fixture, then assert `readback::list` over
/// `temper_next.*` returns the SAME rows with the SAME projected fields as production's `list_visible`
/// over `public.*` for the owner profile P1 (which owns all 4 active fixture resources, so the
/// filterless call returns exactly those 4).
///
/// Compared as a SET keyed by `origin_uri` (a verbatim-carried UNIQUE key), NOT in order: ordered-by-
/// `updated` parity is deliberately NOT asserted. Synthesis sources `kb_resources.updated` from the
/// genesis event's `occurred_at`, which is `now()` = transaction-start time, constant within the single
/// synthesis transaction — so every synthesized row shares one identical `updated` and `ORDER BY updated
/// DESC` is a non-deterministic tie over `temper_next`. The migration-time floor is the row SET + its
/// projected fields, not absolute recency ordering.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn list_parity(pool: sqlx::PgPool) {
    use temper_api::services::resource_service;
    use temper_core::types::resource::ResourceListParams;

    common::seed_and_synthesize(&pool).await;

    // Production read: list_visible(P1). P1 owns all 4 active fixture resources, so resources_visible_to
    // returns exactly those 4. A generous limit (>4) defeats any default page size.
    let params = ResourceListParams {
        limit: Some(100),
        ..Default::default()
    };
    let prod = resource_service::list_visible(&pool, fixture_ids::OWNER_PROFILE, params)
        .await
        .expect("production list_visible");

    let prod_by_uri: BTreeMap<String, ListProjection> = prod
        .rows
        .into_iter()
        .map(|r| {
            (
                r.origin_uri,
                (r.title, r.doc_type_name, r.stage, r.mode, r.effort),
            )
        })
        .collect();

    // Readback over temper_next.*.
    let next_by_uri: BTreeMap<String, ListProjection> = readback::list(&pool)
        .await
        .expect("readback::list")
        .into_iter()
        .map(|r| {
            (
                r.origin_uri,
                (r.title, r.doc_type, r.stage, r.mode, r.effort),
            )
        })
        .collect();

    assert_eq!(
        prod_by_uri.len(),
        4,
        "production returns the 4 active resources"
    );
    assert_eq!(
        next_by_uri.len(),
        4,
        "readback returns the 4 active resources"
    );
    assert_eq!(
        prod_by_uri, next_by_uri,
        "readback::list matches production list_visible row-set + projected fields (keyed by origin_uri)"
    );

    // Spot-check the workflow-field projection: R2 (task) carries stage/mode/effort verbatim...
    assert_eq!(
        next_by_uri.get("temper://fixture/task-doc"),
        Some(&(
            "Task Doc".to_string(),
            "task".to_string(),
            Some("doing".to_string()),
            Some("build".to_string()),
            Some("M".to_string()),
        )),
        "R2 projects its workflow keys verbatim"
    );
    // ...while R1 (goal-doc, a concept with no workflow keys) projects them all as None.
    assert_eq!(
        next_by_uri.get("temper://fixture/goal-doc"),
        Some(&(
            "Goal Doc".to_string(),
            "concept".to_string(),
            None,
            None,
            None,
        )),
        "R1 carries no workflow keys, so stage/mode/effort are None (LEFT-JOIN absent, not dropped)"
    );
}
