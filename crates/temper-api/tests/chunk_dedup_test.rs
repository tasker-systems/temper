#![cfg(feature = "test-db")]

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn schema_has_kb_resource_revisions_table(pool: PgPool) {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_name = 'kb_resource_revisions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(exists, "kb_resource_revisions table must exist");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_chunks_has_revision_columns(pool: PgPool) {
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'kb_chunks' \
           AND column_name IN ('first_revision_id', 'superseded_revision_id') \
         ORDER BY column_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        cols,
        vec![
            "first_revision_id".to_string(),
            "superseded_revision_id".to_string()
        ]
    );
}

/// Insert a minimal `kb_resources` row for a test fixture. Also creates a
/// profile and context to satisfy FKs. Returns the resource id.
async fn seed_resource(pool: &PgPool) -> Uuid {
    let profile_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, display_name, email, slug, created) \
         VALUES (gen_random_uuid(), 'test', 'test@local', \
                 'test-dedup-' || substr(gen_random_uuid()::text, 1, 8), now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let context_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, kb_owner_table, kb_owner_id, name, created) \
         VALUES (gen_random_uuid(), 'kb_profiles', $1, \
                 'test-ctx-' || substr(gen_random_uuid()::text, 1, 8), now()) RETURNING id",
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

    let resource_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            originator_profile_id, owner_profile_id, created, updated
        ) VALUES (
            gen_random_uuid(), $1, $2,
            'test://r-' || substr(gen_random_uuid()::text, 1, 8),
            'T', 't-' || substr(gen_random_uuid()::text, 1, 8),
            $3, $3, now(), now()
        ) RETURNING id
        "#,
    )
    .bind(context_id)
    .bind(doc_type_id)
    .bind(profile_id)
    .fetch_one(pool)
    .await
    .unwrap();

    resource_id
}

/// Build one chunk jsonb entry. The embedding is a 768-dim zero vector.
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

/// Insert a minimal kb_resource_audits row and return its id.
async fn seed_audit(pool: &PgPool, resource_id: Uuid, body_hash: &str) -> Uuid {
    // kb_resource_audits.event_id is a FK to kb_events — insert that first.
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

/// Count current chunks for a resource.
async fn count_current(pool: &PgPool, resource_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true",
    )
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Count total chunks for a resource (current + superseded).
async fn count_total(pool: &PgPool, resource_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1")
        .bind(resource_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn persist_chunks_creates_revision(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let audit = seed_audit(&pool, rid, "body1").await;
    let chunks = json!([chunk(0, "alpha", "ha"), chunk(1, "beta", "hb")]);

    let rev: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(audit)
        .bind("body1")
        .bind(&chunks)
        .fetch_one(&pool)
        .await
        .unwrap();

    let (chunk_count, body_hash, audit_id): (i32, String, Option<Uuid>) = sqlx::query_as(
        "SELECT chunk_count, body_hash, audit_id FROM kb_resource_revisions WHERE id = $1",
    )
    .bind(rev)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(chunk_count, 2);
    assert_eq!(body_hash, "body1");
    assert_eq!(audit_id, Some(audit));
    assert_eq!(count_current(&pool, rid).await, 2);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_preserves_unchanged_positions(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "body1").await;
    let chunks = json!([
        chunk(0, "alpha", "ha"),
        chunk(1, "beta", "hb"),
        chunk(2, "gamma", "hg")
    ]);
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("body1")
        .bind(&chunks)
        .fetch_one(&pool)
        .await
        .unwrap();

    // A different body_hash on the second call forces the dedup CTE to run
    // (bypassing the replay guard), so we validate that chunks with matching
    // (chunk_index, content_hash) are preserved even when the body differs.
    // Realistic scenario: a whitespace-only body edit that doesn't alter any
    // chunk's extracted content.
    let a2 = seed_audit(&pool, rid, "body1-rev2").await;
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("body1-rev2")
        .bind(&chunks)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_ne!(r1, r2, "second call must create a distinct revision");
    assert_eq!(count_current(&pool, rid).await, 3);
    assert_eq!(
        count_total(&pool, rid).await,
        3,
        "preserved chunks must not be duplicated"
    );

    let first_revs: Vec<Uuid> =
        sqlx::query_scalar("SELECT first_revision_id FROM kb_chunks WHERE resource_id = $1")
            .bind(rid)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        first_revs.iter().all(|r| *r == r1),
        "preserved chunks keep original first_revision_id"
    );
}

/// Replay guard: calling `replace_resource_chunks` with the same body_hash
/// as the resource's most recent revision is a no-op that returns the
/// existing revision's id. Prevents workflow retries from writing chunkless
/// noise revisions.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_replay_returns_existing_revision(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "body1").await;
    let chunks = json!([chunk(0, "alpha", "ha"), chunk(1, "beta", "hb")]);
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("body1")
        .bind(&chunks)
        .fetch_one(&pool)
        .await
        .unwrap();

    let a2 = seed_audit(&pool, rid, "body1").await;
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("body1")
        .bind(&chunks)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(r1, r2, "replay of same body_hash returns existing revision");
    assert_eq!(count_current(&pool, rid).await, 2);
    assert_eq!(
        count_total(&pool, rid).await,
        2,
        "no chunks written on replay"
    );

    let revision_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM kb_resource_revisions WHERE resource_id = $1",
    )
    .bind(rid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(revision_count, 1, "no new revision row written on replay");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_supersedes_changed_content(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_initial = json!([
        chunk(0, "alpha", "ha"),
        chunk(1, "beta", "hb"),
        chunk(2, "gamma", "hg")
    ]);
    let r1: Uuid = sqlx::query_scalar("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("b1")
        .bind(&c_initial)
        .fetch_one(&pool)
        .await
        .unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let c_updated = json!([
        chunk(0, "alpha", "ha"),
        chunk(1, "BETA!", "hb2"),
        chunk(2, "gamma", "hg")
    ]);
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("b2")
        .bind(&c_updated)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count_current(&pool, rid).await, 3);
    assert_eq!(
        count_total(&pool, rid).await,
        4,
        "one new chunk + one superseded"
    );

    let superseded: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT first_revision_id, superseded_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index = 1 AND is_current = false",
    )
    .bind(rid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(superseded.0, r1);
    assert_eq!(superseded.1, Some(r2));

    let current_new: (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT first_revision_id, superseded_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index = 1 AND is_current = true",
    )
    .bind(rid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(current_new.0, r2);
    assert_eq!(current_new.1, None);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_supersedes_removed_positions(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_four = json!([
        chunk(0, "a", "h0"),
        chunk(1, "b", "h1"),
        chunk(2, "c", "h2"),
        chunk(3, "d", "h3"),
    ]);
    sqlx::query_scalar::<_, Uuid>("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("b1")
        .bind(&c_four)
        .fetch_one(&pool)
        .await
        .unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let c_three = json!([
        chunk(0, "a", "h0"),
        chunk(1, "b", "h1"),
        chunk(2, "c", "h2")
    ]);
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("b2")
        .bind(&c_three)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count_current(&pool, rid).await, 3);

    let removed_supersede: Option<Uuid> = sqlx::query_scalar(
        "SELECT superseded_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index = 3",
    )
    .bind(rid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(removed_supersede, Some(r2));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_adds_new_positions(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_two = json!([chunk(0, "a", "h0"), chunk(1, "b", "h1")]);
    sqlx::query_scalar::<_, Uuid>("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("b1")
        .bind(&c_two)
        .fetch_one(&pool)
        .await
        .unwrap();

    let a2 = seed_audit(&pool, rid, "b2").await;
    let c_four = json!([
        chunk(0, "a", "h0"),
        chunk(1, "b", "h1"),
        chunk(2, "c", "h2"),
        chunk(3, "d", "h3"),
    ]);
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("b2")
        .bind(&c_four)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count_current(&pool, rid).await, 4);

    let new_positions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT first_revision_id FROM kb_chunks \
         WHERE resource_id = $1 AND chunk_index IN (2, 3) AND is_current = true",
    )
    .bind(rid)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(new_positions.len(), 2);
    assert!(new_positions.iter().all(|r| *r == r2));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn replace_chunks_empty_input_supersedes_all(pool: PgPool) {
    let rid = seed_resource(&pool).await;
    let a1 = seed_audit(&pool, rid, "b1").await;
    let c_three = json!([
        chunk(0, "a", "h0"),
        chunk(1, "b", "h1"),
        chunk(2, "c", "h2")
    ]);
    sqlx::query_scalar::<_, Uuid>("SELECT persist_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a1)
        .bind("b1")
        .bind(&c_three)
        .fetch_one(&pool)
        .await
        .unwrap();

    let a2 = seed_audit(&pool, rid, "").await;
    let r2: Uuid = sqlx::query_scalar("SELECT replace_resource_chunks($1, $2, $3, $4)")
        .bind(rid)
        .bind(a2)
        .bind("")
        .bind(json!([]))
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count_current(&pool, rid).await, 0);

    let chunk_count: i32 =
        sqlx::query_scalar("SELECT chunk_count FROM kb_resource_revisions WHERE id = $1")
            .bind(r2)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(chunk_count, 0);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_assigns_first_revision_to_every_chunk(pool: PgPool) {
    // Any chunk existing after all migrations run must have first_revision_id set.
    // The sqlx::test harness applies migrations in order, including the backfill,
    // so seed-data chunks (inserted by earlier migrations) must be populated.
    let null_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kb_chunks WHERE first_revision_id IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        null_count, 0,
        "backfill must leave zero chunks with null first_revision_id"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_chunks_first_revision_id_is_not_null(pool: PgPool) {
    let is_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_name = 'kb_chunks' AND column_name = 'first_revision_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(is_nullable, "NO");
}
