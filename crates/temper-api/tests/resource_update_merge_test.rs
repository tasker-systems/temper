//! Partial managed_meta + open_meta merge semantics.
//!
//! Tests that PATCH /api/resources/{id} with managed_meta or open_meta
//! performs a partial merge: `Some` fields overwrite stored values,
//! `None` fields preserve the stored value. `managed_meta` merges by
//! Property key (incoming wins). The `managed_hash` must be recomputed
//! after any managed_meta change.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a JWT-authenticated profile + resource, seeds a manifest row with
/// the given `managed_meta` JSON, and returns `(token, resource_id_str)`.
///
/// Pattern: create profile with context → generate matching JWT → create
/// resource via HTTP → write manifest row directly for test setup.
async fn setup_resource_with_managed_meta(
    app: &common::TestApp,
    pool: &PgPool,
    managed_meta: serde_json::Value,
) -> (String, String) {
    let email = format!("merge-test-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    // Create a resource owned by this profile.
    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://merge-managed-{}", Uuid::new_v4()),
            "title": "Managed Meta Merge Test",
            "slug": null
        }))
        .send()
        .await
        .expect("create resource failed");

    assert_eq!(
        create_resp.status().as_u16(),
        200,
        "resource create must succeed"
    );

    let created: Value = create_resp.json().await.expect("expected JSON");
    let resource_id_str = created["id"]
        .as_str()
        .expect("id field missing")
        .to_string();

    // Seed the baseline managed_meta via the API (the substrate stores it as
    // kb_properties; a PATCH merges into the create-time managed_meta).
    seed_meta(
        app,
        &token,
        &resource_id_str,
        json!({ "managed_meta": managed_meta }),
    )
    .await;

    (token, resource_id_str)
}

/// Seed/patch a resource's meta via PATCH /api/resources/{id} and assert 200.
async fn seed_meta(app: &common::TestApp, token: &str, resource_id: &str, body: serde_json::Value) {
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("seed meta PATCH failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "seed meta must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );
}

/// Creates a JWT-authenticated profile + resource, seeds a manifest row with
/// the given `open_meta` JSON, and returns `(token, resource_id_str)`.
async fn setup_resource_with_open_meta(
    app: &common::TestApp,
    pool: &PgPool,
    open_meta: serde_json::Value,
) -> (String, String) {
    let email = format!("merge-test-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://merge-open-{}", Uuid::new_v4()),
            "title": "Open Meta Merge Test",
            "slug": null
        }))
        .send()
        .await
        .expect("create resource failed");

    assert_eq!(
        create_resp.status().as_u16(),
        200,
        "resource create must succeed"
    );

    let created: Value = create_resp.json().await.expect("expected JSON");
    let resource_id_str = created["id"]
        .as_str()
        .expect("id field missing")
        .to_string();

    // Seed the baseline open_meta via the API (stored as kb_properties).
    seed_meta(
        app,
        &token,
        &resource_id_str,
        json!({ "open_meta": open_meta }),
    )
    .await;

    (token, resource_id_str)
}

/// Read a resource's managed_meta via GET /api/resources/{id}/meta (the
/// substrate reconstructs it from kb_properties via readback::meta).
async fn fetch_managed_meta(app: &common::TestApp, token: &str, resource_id: &str) -> Value {
    fetch_meta(app, token, resource_id).await["managed_meta"].clone()
}

/// Read a resource's open_meta via GET /api/resources/{id}/meta.
async fn fetch_open_meta(app: &common::TestApp, token: &str, resource_id: &str) -> Value {
    fetch_meta(app, token, resource_id).await["open_meta"].clone()
}

/// GET /api/resources/{id}/meta → the ResourceMetaResponse JSON.
async fn fetch_meta(app: &common::TestApp, token: &str, resource_id: &str) -> Value {
    app.client
        .get(app.url(&format!("/api/resources/{resource_id}/meta")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("get meta failed")
        .json()
        .await
        .expect("meta JSON")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// PATCH with managed_meta containing only `temper-stage` must update stage
/// and preserve untouched fields (temper-mode, temper-goal) in the stored
/// manifest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_meta_partial_update_preserves_untouched_fields(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // Use managed keys whose substrate fate is `Property` (they round-trip as
    // managed_meta via kb_properties). `temper-goal` (→Edge) and `date`
    // (→open-tier for research) deliberately do NOT, so they are not used here.
    let stored = json!({
        "temper-stage": "in-progress",
        "temper-mode": "build",
        "temper-status": "active"
    });
    let (token, resource_id) = setup_resource_with_managed_meta(&app, &pool, stored).await;

    // PATCH only stage.
    let req_body = json!({
        "managed_meta": {
            "temper-stage": "done"
        }
    });

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&req_body)
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "meta-only PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let merged = fetch_managed_meta(&app, &token, &resource_id).await;
    assert_eq!(
        merged["temper-stage"],
        json!("done"),
        "stage must be updated"
    );
    assert_eq!(
        merged["temper-mode"],
        json!("build"),
        "mode must be preserved"
    );
    assert_eq!(
        merged["temper-status"],
        json!("active"),
        "status must be preserved"
    );
}

/// PATCH with managed_meta merges by key: an existing managed key survives when
/// a different key is patched in.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_meta_merges_by_property_key(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // Seed one Property-fate managed key.
    let stored = json!({ "temper-mode": "build" });
    let (token, resource_id) = setup_resource_with_managed_meta(&app, &pool, stored).await;

    // PATCH a different managed key — `temper-mode` must survive.
    let req_body = json!({
        "managed_meta": { "temper-status": "active" }
    });

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&req_body)
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "meta-only PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let merged = fetch_managed_meta(&app, &token, &resource_id).await;
    assert_eq!(
        merged["temper-mode"],
        json!("build"),
        "existing managed key 'temper-mode' must be preserved"
    );
    assert_eq!(
        merged["temper-status"],
        json!("active"),
        "incoming managed key 'temper-status' must be added"
    );
}

/// PATCH with open_meta merges by key: absent-from-incoming keys survive;
/// incoming keys overwrite or extend.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_meta_partial_update_merges_by_key(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let stored = json!({
        "tags": ["rust"],
        "aliases": ["temper-cli"]
    });
    let (token, resource_id) = setup_resource_with_open_meta(&app, &pool, stored).await;

    // PATCH tags only — aliases must survive.
    let req_body = json!({
        "open_meta": { "tags": ["rust", "axum"] }
    });

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&req_body)
        .send()
        .await
        .expect("PATCH request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "meta-only PATCH must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let merged = fetch_open_meta(&app, &token, &resource_id).await;
    assert_eq!(
        merged["tags"],
        json!(["rust", "axum"]),
        "tags must be overwritten by incoming"
    );
    assert_eq!(
        merged["aliases"],
        json!(["temper-cli"]),
        "aliases must be preserved (not in incoming)"
    );
}

// `managed_hash_recomputes_after_merge` was DELETED: the substrate retired the
// `managed_hash` (db_backend sets it `None`; GET /meta returns `managed_hash: ""`),
// so there is no recomputed hash to assert. The managed_meta merge it leaned on is
// still covered by the partial-update tests above.

/// Two non-folded rows for one key collapse to the NEWEST, deterministically.
///
/// `readback::meta`'s query had no ORDER BY and the reader map-inserts per row,
/// so the survivor was whatever order Postgres returned. 13 production resources
/// are in this state — all cogmap-homed, all facets updated via `facet_set`,
/// which appends rather than folding. A resolved question could read as open,
/// and differently between two reads.
///
/// Seeded with raw SQL on purpose: the API's meta path uses `property_set`
/// (fold-then-insert) and cannot reach this state. `uq_kb_properties_active` is
/// unique on (owner, key, VALUE) where not folded, so two rows sharing a key but
/// differing in value are permitted — which is exactly the production shape.
///
/// **The facet goes resolved â reopened, deliberately.** Without an ORDER BY the
/// planner serves this from `uq_kb_properties_active`, whose key order is
/// (owner_table, owner_id, property_key, property_value) â so rows arrive sorted by
/// VALUE, and the jsonb-largest value wins rather than the newest. For the
/// open â resolved direction those two coincide ("open" < "resolved"), so that pair
/// passes with or without the fix and proves nothing. Reopening inverts it: the
/// newest value ("open") is the jsonb-smaller one, so an unordered read returns the
/// superseded "resolved" and only `ORDER BY created` returns "open".
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_collapses_a_repeated_key_to_the_newest_row(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, resource_id_str) =
        setup_resource_with_open_meta(&app, &pool, json!({ "date": "2026-07-03" })).await;
    let resource_id = Uuid::parse_str(&resource_id_str).expect("resource id");

    // Both FKs are NOT NULL REFERENCES kb_events(id); the create already emitted one.
    let event_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("an event exists after resource creation");

    for (created, status) in [
        ("2026-07-03T10:00:00Z", "resolved"),
        ("2026-07-03T11:00:00Z", "open"),
    ] {
        sqlx::query(
            "INSERT INTO kb_properties
                 (owner_table, owner_id, property_key, property_value, weight,
                  asserted_by_event_id, last_event_id, is_folded, created)
             VALUES ('kb_resources', $1, 'facet', $2, 1.0, $3, $3, false, $4::timestamptz)",
        )
        .bind(resource_id)
        .bind(json!({ "status": status }))
        .bind(event_id)
        .bind(created)
        .execute(&pool)
        .await
        .expect("seed facet row");
    }

    // Repeated: an unordered read can return insertion order by luck.
    for _ in 0..5 {
        let open = fetch_open_meta(&app, &token, &resource_id_str).await;
        assert_eq!(
            open["facet"],
            json!({ "status": "open" }),
            "readback::meta must return the newest row for a repeated key, \
             not the jsonb-largest value the unique index happens to order last"
        );
    }
}
