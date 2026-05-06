//! Phase 6 — managed_meta canonical data migration tests.
//!
//! Verifies the SQL migration `migrations/<timestamp>_managed_meta_canonical_keys.sql`
//! correctly rewrites legacy JSONB shapes and resets affected hashes to the
//! empty-string sentinel.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const MIGRATION_A_PATH: &str = "../../migrations/20260505235303_managed_meta_canonical_keys.sql";

async fn apply_migration_a(pool: &PgPool) {
    let sql = std::fs::read_to_string(MIGRATION_A_PATH)
        .expect("Migration A SQL file should exist at the expected path");
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .expect("Migration A should apply cleanly");
}

async fn insert_legacy_session_row(pool: &PgPool, title: &str, slug: &str, date: &str) -> Uuid {
    // Look up the session doctype id, profile id, and a context id
    // from the seeded test fixtures (matches the Phase 5 / meta_reconcile_test pattern).
    let doc_type_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = 'session'")
            .fetch_one(pool)
            .await
            .expect("session doctype must be seeded");

    let profile_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one profile must be seeded");

    let context_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_contexts LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one context must be seeded");

    let resource_id = Uuid::now_v7();
    let origin_uri = format!("phase6-test://{resource_id}");

    sqlx::query(
        "INSERT INTO kb_resources
         (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
          originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7, now(), now())",
    )
    .bind(resource_id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(&origin_uri)
    .bind(title)
    .bind(slug)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("insert kb_resources");

    let legacy_managed: Value = json!({
        "title": title,
        "slug": slug,
        "date": date,
        "temper-stage": "in-progress",
    });

    sqlx::query(
        "INSERT INTO kb_resource_manifests
         (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
         VALUES ($1, 'sha256:legacy', $2, '{}'::JSONB,
                 'sha256:legacy_managed', 'sha256:legacy_open', now())",
    )
    .bind(resource_id)
    .bind(legacy_managed)
    .execute(pool)
    .await
    .expect("insert kb_resource_manifests");

    resource_id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn phase6_migration_a_renames_legacy_keys_and_moves_date(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let id = insert_legacy_session_row(&pool, "Legacy Title", "legacy-slug", "2026-01-15").await;

    apply_migration_a(&pool).await;

    let row = sqlx::query_as::<_, (Value, Value, String, String)>(
        "SELECT managed_meta, open_meta, managed_hash, open_hash
         FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("row should still exist");

    let (managed_meta, open_meta, managed_hash, open_hash) = row;

    assert_eq!(
        managed_meta.get("temper-title").and_then(|v| v.as_str()),
        Some("Legacy Title"),
        "temper-title should be present with renamed value"
    );
    assert_eq!(
        managed_meta.get("temper-slug").and_then(|v| v.as_str()),
        Some("legacy-slug"),
        "temper-slug should be present with renamed value"
    );
    assert!(
        managed_meta.get("title").is_none(),
        "bare `title` should be stripped from managed_meta"
    );
    assert!(
        managed_meta.get("slug").is_none(),
        "bare `slug` should be stripped from managed_meta"
    );
    assert!(
        managed_meta.get("date").is_none(),
        "`date` should be stripped from managed_meta on session rows"
    );
    assert_eq!(
        open_meta.get("date").and_then(|v| v.as_str()),
        Some("2026-01-15"),
        "`date` should be moved into open_meta on session rows"
    );
    assert_eq!(
        managed_hash, "",
        "managed_hash should be reset to empty-string sentinel"
    );
    assert_eq!(
        open_hash, "",
        "open_hash should be reset to empty-string sentinel"
    );

    // Other managed-tier keys preserved.
    assert_eq!(
        managed_meta.get("temper-stage").and_then(|v| v.as_str()),
        Some("in-progress"),
        "non-renamed managed-tier keys should be preserved"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn phase6_migration_a_idempotent_on_already_canonical_rows(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let doc_type_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = 'session'")
            .fetch_one(&pool)
            .await
            .expect("session doctype must be seeded");
    let profile_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("at least one profile must be seeded");
    let context_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_contexts LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("at least one context must be seeded");

    let resource_id = Uuid::now_v7();
    let origin_uri = format!("phase6-test://{resource_id}");

    sqlx::query(
        "INSERT INTO kb_resources
         (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
          originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, 'Canonical', 'canonical', $5, $5, now(), now())",
    )
    .bind(resource_id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(&origin_uri)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("insert kb_resources");

    let canonical_managed: Value = json!({
        "temper-title": "Canonical",
        "temper-slug": "canonical",
        "temper-stage": "done",
    });
    let canonical_open: Value = json!({"date": "2026-04-01"});

    sqlx::query(
        "INSERT INTO kb_resource_manifests
         (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
         VALUES ($1, 'sha256:body', $2, $3, 'sha256:canonical_m', 'sha256:canonical_o', now())",
    )
    .bind(resource_id)
    .bind(&canonical_managed)
    .bind(&canonical_open)
    .execute(&pool)
    .await
    .expect("insert kb_resource_manifests");

    apply_migration_a(&pool).await;

    let row = sqlx::query_as::<_, (Value, Value, String, String)>(
        "SELECT managed_meta, open_meta, managed_hash, open_hash
         FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .expect("row should still exist");

    let (stored_managed, stored_open, stored_mhash, stored_ohash) = row;

    assert_eq!(
        stored_managed, canonical_managed,
        "canonical managed_meta should be untouched by Migration A"
    );
    assert_eq!(
        stored_open, canonical_open,
        "canonical open_meta should be untouched by Migration A"
    );
    assert_eq!(
        stored_mhash, "sha256:canonical_m",
        "managed_hash should NOT be reset on canonical rows"
    );
    assert_eq!(
        stored_ohash, "sha256:canonical_o",
        "open_hash should NOT be reset on canonical rows"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn phase6_migration_a_does_not_move_date_for_non_dated_doctypes(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    // A `task` row with a stray `date` in managed_meta should NOT have it moved
    // to open_meta — only session/research/decision/concept doctypes get the move.
    let doc_type_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = 'task'")
        .fetch_one(&pool)
        .await
        .expect("task doctype must be seeded");
    let profile_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("at least one profile must be seeded");
    let context_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_contexts LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("at least one context must be seeded");

    let resource_id = Uuid::now_v7();
    let origin_uri = format!("phase6-test://{resource_id}");

    sqlx::query(
        "INSERT INTO kb_resources
         (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
          originator_profile_id, owner_profile_id, created, updated)
         VALUES ($1, $2, $3, $4, 'Stray Date Task', 'stray-date-task', $5, $5, now(), now())",
    )
    .bind(resource_id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(&origin_uri)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("insert kb_resources");

    let stray_managed: Value = json!({"title": "Stray Date Task", "date": "2026-04-01"});

    sqlx::query(
        "INSERT INTO kb_resource_manifests
         (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
         VALUES ($1, 'sha256:body', $2, '{}'::JSONB,
                 'sha256:legacy_m', 'sha256:legacy_o', now())",
    )
    .bind(resource_id)
    .bind(&stray_managed)
    .execute(&pool)
    .await
    .expect("insert kb_resource_manifests");

    apply_migration_a(&pool).await;

    let row = sqlx::query_as::<_, (Value, Value, String, String)>(
        "SELECT managed_meta, open_meta, managed_hash, open_hash
         FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .expect("row should still exist");

    let (managed_meta, open_meta, managed_hash, open_hash) = row;

    // Title rename DID happen.
    assert_eq!(
        managed_meta.get("temper-title").and_then(|v| v.as_str()),
        Some("Stray Date Task")
    );
    // But date was NOT moved (task is not in the dated-doctype set).
    assert_eq!(
        managed_meta.get("date").and_then(|v| v.as_str()),
        Some("2026-04-01"),
        "date should remain in managed_meta for non-dated doctypes"
    );
    assert!(
        open_meta.get("date").is_none(),
        "date should NOT be added to open_meta for non-dated doctypes"
    );
    assert_eq!(
        managed_hash, "",
        "managed_hash should be reset by the title-rename pass even for non-dated doctypes"
    );
    assert_eq!(
        open_hash, "sha256:legacy_o",
        "open_hash should NOT be reset when only managed_meta was changed"
    );
}
