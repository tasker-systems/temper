//! Partial managed_meta + open_meta merge semantics.
//!
//! Tests that PATCH /api/resources/{id} with managed_meta or open_meta
//! performs a partial merge: `Some` fields overwrite stored values,
//! `None` fields preserve the stored value. The extra bucket in
//! `ManagedMeta` merges by key (incoming wins). The `managed_hash`
//! must be recomputed after any managed_meta change.
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
/// Pattern: generate JWT → call auth/me (auto-creates profile) → create
/// resource via HTTP → write manifest row directly for test setup.
async fn setup_resource_with_managed_meta(
    app: &common::TestApp,
    pool: &PgPool,
    managed_meta: serde_json::Value,
) -> (String, String) {
    let sub = format!("test-sub-merge-{}", Uuid::new_v4());
    let email = format!("merge-test-{}@example.com", Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // Auto-provision the profile.
    let _ = app
        .client
        .get(app.url("/api/auth/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("auth/me failed");

    // Create a resource owned by this profile.
    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": common::fixtures::TEMPER_CONTEXT_ID,
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
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
    let resource_id = Uuid::parse_str(&resource_id_str).expect("invalid uuid");

    // Seed the manifest row with the desired managed_meta.
    let managed_hash = temper_core::hash::compute_managed_hash("research", &managed_meta);
    let open_meta = json!({});
    let open_hash = temper_core::hash::compute_open_hash(&open_meta);
    sqlx::query(
        r#"INSERT INTO kb_resource_manifests
            (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
           VALUES ($1, 'test-body-hash', $2, $3, $4, $5, now())
           ON CONFLICT (resource_id) DO UPDATE
               SET managed_meta = $2, managed_hash = $4, updated = now()"#,
    )
    .bind(resource_id)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(pool)
    .await
    .expect("seed manifest row");

    (token, resource_id_str)
}

/// Creates a JWT-authenticated profile + resource, seeds a manifest row with
/// the given `open_meta` JSON, and returns `(token, resource_id_str)`.
async fn setup_resource_with_open_meta(
    app: &common::TestApp,
    pool: &PgPool,
    open_meta: serde_json::Value,
) -> (String, String) {
    let sub = format!("test-sub-merge-{}", Uuid::new_v4());
    let email = format!("merge-test-{}@example.com", Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    let _ = app
        .client
        .get(app.url("/api/auth/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("auth/me failed");

    let create_resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": common::fixtures::TEMPER_CONTEXT_ID,
            "kb_doc_type_id": common::fixtures::RESEARCH_DOC_TYPE_ID,
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
    let resource_id = Uuid::parse_str(&resource_id_str).expect("invalid uuid");

    let managed_meta = json!({});
    let managed_hash = temper_core::hash::compute_managed_hash("research", &managed_meta);
    let open_hash = temper_core::hash::compute_open_hash(&open_meta);
    sqlx::query(
        r#"INSERT INTO kb_resource_manifests
            (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
           VALUES ($1, 'test-body-hash', $2, $3, $4, $5, now())
           ON CONFLICT (resource_id) DO UPDATE
               SET open_meta = $3, open_hash = $5, updated = now()"#,
    )
    .bind(resource_id)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(pool)
    .await
    .expect("seed manifest row");

    (token, resource_id_str)
}

/// Fetch the stored managed_meta JSONB value from kb_resource_manifests.
async fn fetch_managed_meta(pool: &PgPool, resource_id: Uuid) -> serde_json::Value {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("fetch managed_meta")
}

/// Fetch the stored open_meta JSONB value from kb_resource_manifests.
async fn fetch_open_meta(pool: &PgPool, resource_id: Uuid) -> serde_json::Value {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT open_meta FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("fetch open_meta")
}

/// Fetch the stored managed_hash from kb_resource_manifests.
async fn fetch_managed_hash(pool: &PgPool, resource_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT managed_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("fetch managed_hash")
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

    let stored = json!({
        "temper-stage": "in-progress",
        "temper-mode": "build",
        "temper-goal": "g1"
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

    let rid = Uuid::parse_str(&resource_id).unwrap();
    let merged = fetch_managed_meta(&pool, rid).await;
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
    assert_eq!(merged["temper-goal"], json!("g1"), "goal must be preserved");
}

/// PATCH with managed_meta that includes extra-bucket keys must merge by key:
/// existing extras survive; incoming extras are added.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_meta_extra_bucket_merges_by_key(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    // Pre-seed an extra "date" key (session-style, lives in the flatten bucket).
    let stored = json!({ "date": "2026-04-13" });
    let (token, resource_id) = setup_resource_with_managed_meta(&app, &pool, stored).await;

    // PATCH with a different extra key — "date" must survive.
    let req_body = json!({
        "managed_meta": { "custom": "value" }
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

    let rid = Uuid::parse_str(&resource_id).unwrap();
    let merged = fetch_managed_meta(&pool, rid).await;
    assert_eq!(
        merged["date"],
        json!("2026-04-13"),
        "existing extra key 'date' must be preserved"
    );
    assert_eq!(
        merged["custom"],
        json!("value"),
        "incoming extra key 'custom' must be added"
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

    let rid = Uuid::parse_str(&resource_id).unwrap();
    let merged = fetch_open_meta(&pool, rid).await;
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

/// When managed_meta changes, the stored managed_hash must be recomputed and
/// differ from the value before the PATCH.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_hash_recomputes_after_merge(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let stored = json!({ "temper-stage": "in-progress" });
    let (token, resource_id) = setup_resource_with_managed_meta(&app, &pool, stored).await;

    let rid = Uuid::parse_str(&resource_id).unwrap();
    let before = fetch_managed_hash(&pool, rid).await;

    // PATCH stage to "done" — managed_meta changes, so hash must change.
    let req_body = json!({
        "managed_meta": { "temper-stage": "done" }
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

    let after = fetch_managed_hash(&pool, rid).await;
    assert_ne!(
        before, after,
        "managed_hash must change when managed_meta changes"
    );
}
