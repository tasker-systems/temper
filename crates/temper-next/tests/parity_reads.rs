#![cfg(feature = "artifact-tests")]
//! WS6 chunk 3 — parity-read harness. Each test ports one production read to `temper_next.*` and
//! asserts identical output for the same logical query over the synthesized prod-shape fixture.
//! Isolated ephemeral DB per test via `#[sqlx::test(migrator = ...)]` (NOT the psql-reset write-path
//! group). Runtime schema-qualified reads throughout (same discipline as `synthesis::source`).
mod common;

use std::collections::BTreeMap;

use common::fixture_ids;
use serde_json::{Map, Value};
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

/// The §7-died (and relocated) managed keys: never reappear in a readback's reconstructed managed map.
/// `temper-title`/`temper-slug`/`temper-id`/`temper-context` DIE (the column/render/row-id/home row
/// carry that state authoritatively); `temper-goal` becomes an EDGE; `temper-type` reconciles to the
/// `doc_type` column. Production's manifest still carries all six, so the expected managed set strips
/// them before comparing against readback.
const DROPPED_MANAGED_KEYS: &[&str] = &[
    "temper-title",
    "temper-slug",
    "temper-id",
    "temper-context",
    "temper-goal",
    "temper-type",
];

/// Coerce a serialized `ManagedMeta` (or `open_meta`) `Value` into a JSON object map. Both production
/// shapes are always JSON objects; a non-object is a contract break we want to surface loudly.
fn as_object(v: Value, what: &str) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("expected {what} to be a JSON object, got {other:?}"),
    }
}

/// §9 — `show` + `get_meta` parity. §7 dissolves the production manifest into `kb_resources` columns,
/// the home row, the `doc_type` property, edges, and `kb_properties`; readback reconstructs the
/// managed/open split from `kb_properties` using the inverse fate set (`MANAGED_PROPERTY_KEYS`). The
/// §7-died keys (title/slug/id/context) are gone by construction — the column/home/id carry that state
/// authoritatively — and the relocated keys (`temper-goal` → edge, `temper-type` → `doc_type` column)
/// are absent too. So parity holds modulo `DROPPED_MANAGED_KEYS`: production managed, minus those six,
/// equals readback managed; production `open_meta` equals readback open verbatim.
///
/// The "show" auth gate is `resource_service::get_visible`, already invoked inside `get_meta` — the
/// meta reconstruction IS the show+meta parity (no separate `readback::show` is built).
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn show_and_meta_parity(pool: sqlx::PgPool) {
    use temper_api::services::meta_service;
    use temper_core::types::ids::{ProfileId, ResourceId};

    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");
    assert_eq!(ids.len(), 4, "4 active fixture resources synthesized");

    // Keyed by origin_uri so the per-resource spot-checks can find R1/R2 regardless of iteration order.
    let mut rb_by_uri: BTreeMap<String, readback::ReconstructedMeta> = BTreeMap::new();

    for new_id in ids.new_ids() {
        let origin_uri = ids
            .origin_uri_for_new(new_id)
            .expect("synthesized id has an origin_uri")
            .to_string();
        let old_id = ids
            .to_old(new_id)
            .expect("synthesized id maps back to prod");

        // Production read (auth-gated by get_visible inside get_meta) for the owner profile P1.
        let prod = meta_service::get_meta(
            &pool,
            ProfileId::from(fixture_ids::OWNER_PROFILE),
            ResourceId::from(old_id),
        )
        .await
        .expect("production get_meta");

        let prod_managed_raw = as_object(
            serde_json::to_value(prod.managed_meta.expect("prod managed_meta present"))
                .expect("serialize managed_meta"),
            "managed_meta",
        );
        let prod_open = as_object(prod.open_meta.expect("prod open_meta present"), "open_meta");

        // Production managed minus the §7-died/relocated keys = what readback should reconstruct.
        let mut expected_managed = prod_managed_raw.clone();
        for k in DROPPED_MANAGED_KEYS {
            expected_managed.remove(*k);
        }

        let rb = readback::meta(&pool, new_id).await.expect("readback::meta");

        assert_eq!(
            rb.managed, expected_managed,
            "{origin_uri}: surviving managed keys + values match production minus the §7-dropped set"
        );
        assert_eq!(
            rb.open, prod_open,
            "{origin_uri}: open keys carry verbatim, matching production open_meta"
        );

        // The §7 guarantee: died keys never reappear in the reconstructed managed map.
        for died in ["temper-title", "temper-slug", "temper-id", "temper-context"] {
            assert!(
                !rb.managed.contains_key(died),
                "{origin_uri}: §7-died key {died} must be absent from readback managed"
            );
        }

        // doc_type is the successor to production's temper-type when present, and always equals the
        // authoritative doctype synthesis stamped (cross-checked against itself + temper-type).
        if let Some(prod_type) = prod_managed_raw.get("temper-type") {
            assert_eq!(
                Value::String(rb.doc_type.clone()),
                *prod_type,
                "{origin_uri}: readback doc_type == production temper-type"
            );
        }

        rb_by_uri.insert(origin_uri, rb);
    }

    // Spot-check R2 (task): the rich case — three surviving managed workflow keys, two open keys, task.
    let r2 = rb_by_uri
        .get("temper://fixture/task-doc")
        .expect("R2 reconstructed");
    let expected_r2_managed: Map<String, Value> = [
        ("temper-stage", "doing"),
        ("temper-mode", "build"),
        ("temper-effort", "M"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
    .collect();
    assert_eq!(
        r2.managed, expected_r2_managed,
        "R2 managed = exactly the three surviving workflow keys, values verbatim"
    );
    let expected_r2_open: Map<String, Value> = [
        ("custom-key", "custom-value"),
        ("another-key", "another-value"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
    .collect();
    assert_eq!(
        r2.open, expected_r2_open,
        "R2 open = its two open_meta keys verbatim"
    );
    assert_eq!(
        r2.doc_type, "task",
        "R2 doc_type is the reconciled task doctype"
    );

    // Spot-check R1 (goal-doc, a concept): no surviving managed keys, no open keys, doctype concept.
    let r1 = rb_by_uri
        .get("temper://fixture/goal-doc")
        .expect("R1 reconstructed");
    assert!(
        r1.managed.is_empty(),
        "R1 carries only died managed keys → empty managed"
    );
    assert!(r1.open.is_empty(), "R1 has no open_meta keys");
    assert_eq!(r1.doc_type, "concept", "R1 doc_type is concept");
}

/// §9 — body-reconstruction read parity (closes the §9 body read floor). For every active fixture
/// resource, the markdown `readback::body` reconstructs from `temper_next` chunks must equal the body
/// production's `get_content` serves today (`ContentResponse.markdown`).
///
/// This read-surface check OVERLAPS the §8 synthesis body-parity gate (`synthesis::parity`) — by design:
/// it exercises the SAME `reconstruct_body` + `new_substrate_chunks` algorithm, but reached as a read
/// (resource_id → body) rather than as the cutover gate's two-source comparison. Sharing one assembler
/// is the point (CONFORM, no second body path); the new floor is that the read surface itself round-trips.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn body_read_parity(pool: sqlx::PgPool) {
    use temper_api::services::resource_service;

    common::seed_and_synthesize(&pool).await;

    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");
    assert_eq!(ids.len(), 4, "4 active fixture resources synthesized");

    for new_id in ids.new_ids() {
        let origin_uri = ids
            .origin_uri_for_new(new_id)
            .expect("synthesized id has an origin_uri")
            .to_string();
        let old_id = ids
            .to_old(new_id)
            .expect("synthesized id maps back to prod");

        // Production body: get_content's assembled markdown (auth-gated for owner P1).
        let prod = resource_service::get_content(&pool, fixture_ids::OWNER_PROFILE, old_id)
            .await
            .expect("production get_content")
            .markdown;

        // Readback body: reconstructed from temper_next chunks via the shared §8 assembler.
        let rb = readback::body(&pool, new_id).await.expect("readback::body");

        assert_eq!(
            rb, prod,
            "{origin_uri}: readback::body == production get_content markdown"
        );
    }

    // Spot-assert R2 (task-doc): the non-vacuous multi-chunk + heading case — an unheaded preamble
    // chunk followed by a depth-2 headed chunk, joined with a blank line.
    let r2_new = ids
        .to_new(fixture_ids::RESOURCE_TASK)
        .expect("R2 (task) has a synthesized id");
    let r2_body = readback::body(&pool, r2_new)
        .await
        .expect("readback::body R2");
    assert_eq!(
        r2_body, "Task intro paragraph.\n\n## Goals\n\nTask goals section body.",
        "R2 reconstructs the preamble + depth-2 heading case verbatim"
    );
}
