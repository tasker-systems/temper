#![cfg(feature = "test-db")]

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

// Inline helpers, same pattern as chunk_dedup_test.rs. Keeping them
// inline rather than in common/ to avoid cross-binary coupling.
async fn seed_resource(pool: &PgPool) -> Uuid {
    let profile_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, display_name, email, slug, created) \
         VALUES (gen_random_uuid(), 'rev', 'rev@local', \
                 'rev-' || substr(gen_random_uuid()::text, 1, 8), now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let context_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, kb_owner_table, kb_owner_id, name, created) \
         VALUES (gen_random_uuid(), 'kb_profiles', $1, \
                 'ctx-' || substr(gen_random_uuid()::text, 1, 8), now()) RETURNING id",
    )
    .bind(profile_id)
    .fetch_one(pool)
    .await
    .unwrap();

    let doc_type_id: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_doc_types WHERE name = 'research' LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();

    sqlx::query_scalar(
        "INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug, \
             originator_profile_id, owner_profile_id, created, updated) \
         VALUES (gen_random_uuid(), $1, $2, \
             'rev://r-' || substr(gen_random_uuid()::text, 1, 8), 'T', \
             't-' || substr(gen_random_uuid()::text, 1, 8), \
             $3, $3, now(), now()) RETURNING id",
    )
    .bind(context_id)
    .bind(doc_type_id)
    .bind(profile_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_audit(pool: &PgPool, resource_id: Uuid, body_hash: &str) -> Uuid {
    let event_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events \
         (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created) \
         VALUES (gen_random_uuid(), \
             (SELECT originator_profile_id FROM kb_resources WHERE id = $1), \
             'test-device', \
             (SELECT kb_context_id FROM kb_resources WHERE id = $1), \
             $1, 'update_body', '{}', now()) RETURNING id",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query_scalar(
        "INSERT INTO kb_resource_audits \
         (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action) \
         VALUES ($1, $2, \
             (SELECT originator_profile_id FROM kb_resources WHERE id = $1), \
             'test-device', $3, 'mh', 'oh', 'update_body') \
         RETURNING id",
    )
    .bind(resource_id)
    .bind(event_id)
    .bind(body_hash)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn chunk(index: i32, content: &str, hash: &str) -> Value {
    let zeros: Vec<f32> = vec![0.0; 768];
    let emb_str = format!(
        "[{}]",
        zeros
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    json!({
        "chunk_index": index,
        "header_path": "",
        "heading_depth": 0,
        "content": content,
        "content_hash": hash,
        "embedding": emb_str,
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_chunks_at_revision_returns_original_state(pool: PgPool) {
    let rid = seed_resource(&pool).await;

    let a1 = seed_audit(&pool, rid, "b1").await;
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("b1")
        .bind(json!([chunk(0, "ORIG-0", "o0"), chunk(1, "ORIG-1", "o1")]))
        .fetch_one(&pool)
        .await
        .unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let _r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("b2")
        .bind(json!([chunk(0, "ORIG-0", "o0"), chunk(1, "NEW-1", "n1")]))
        .fetch_one(&pool)
        .await
        .unwrap();

    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT chunk_index, content FROM resource_chunks_at_revision($1, $2) ORDER BY chunk_index",
    )
    .bind(rid)
    .bind(r1)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (0, "ORIG-0".to_string()));
    assert_eq!(rows[1], (1, "ORIG-1".to_string()));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_chunks_at_revision_unknown_returns_empty(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM resource_chunks_at_revision($1, gen_random_uuid())",
    )
    .bind(rid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
}
