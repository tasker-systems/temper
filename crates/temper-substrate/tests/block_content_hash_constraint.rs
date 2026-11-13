#![cfg(feature = "test-db")]
//! Witnesses for migration 20261127000010's backfill and CHECK, run against the migration's OWN
//! statement text (the `-- BACKFILL` and `-- CHECK` blocks are extracted from the file), so the
//! test cannot drift from what ships.
//!
//! - The CHECK refuses a path that empties a block's bytes and forgets its hash.
//! - The backfill rewrites a row emptied before the migration (bytes `''`, the old text's hash)
//!   and leaves a live row's hash alone; the CHECK then validates over the result.
//!
//! The redactions themselves are witnessed in `resource_erasure_act.rs` and
//! `block_history_scrub.rs`.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../migrations/20261127000010_block_content_hash_follows_redaction.sql");
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The statement between `-- <tag> BEGIN` and `-- <tag> END` in the migration.
fn block(tag: &str) -> &'static str {
    let begin = format!("-- {tag} BEGIN\n");
    let end = format!("-- {tag} END");
    let from = MIGRATION.find(&begin).expect("begin marker") + begin.len();
    let to = MIGRATION[from..].find(&end).expect("end marker") + from;
    &MIGRATION[from..to]
}

fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(s))
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

/// A one-block verbatim resource through the real create path; returns its current revision id.
async fn verbatim_revision(pool: &PgPool, slug: &str, prose: &str) -> Uuid {
    let (owner, emitter) = system_actor(pool).await;
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    );
    let chunk = IncomingChunk {
        chunk_index: 0,
        content_hash: sha256_hex(prose.trim()),
        content: prose.to_owned(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: String::new(),
        heading_depth: 0,
    };
    let resource: ResourceId = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &format!("test://{slug}"),
            body: prose,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create through the real path");
    sqlx::query_scalar(
        "SELECT b.current_revision_id FROM kb_content_blocks b \
          JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id \
         WHERE b.resource_id = $1 AND NOT b.is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .expect("the resource is verbatim")
}

async fn row(pool: &PgPool, revision: Uuid) -> (String, String) {
    sqlx::query_as(
        "SELECT content, content_hash FROM kb_block_content WHERE block_revision_id = $1",
    )
    .bind(revision)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_check_refuses_emptied_bytes_that_keep_their_hash(pool: PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let rev = verbatim_revision(&pool, "check-refuses", "SSN 123-45-6789").await;

    let err = sqlx::query("UPDATE kb_block_content SET content = '' WHERE block_revision_id = $1")
        .bind(rev)
        .execute(&pool)
        .await
        .expect_err("emptying the bytes without the hash is refused");
    let db = err.as_database_error().expect("a database error");
    assert_eq!(
        db.code().as_deref(),
        Some("23514"),
        "a check violation: {db}"
    );
    assert_eq!(db.constraint(), Some("ck_kb_block_content_emptied_hash"));

    sqlx::query(
        "UPDATE kb_block_content SET content = '', content_hash = encode(sha256(''::bytea), 'hex') \
          WHERE block_revision_id = $1",
    )
    .bind(rev)
    .execute(&pool)
    .await
    .expect("emptying both together is accepted");
    assert_eq!(
        row(&pool, rev).await,
        (String::new(), EMPTY_SHA256.to_owned())
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_backfill_rewrites_rows_emptied_before_it_and_only_those(pool: PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let erased = verbatim_revision(&pool, "backfill-erased", "SSN 123-45-6789").await;
    let live = verbatim_revision(&pool, "backfill-live", "an unrelated live body").await;
    let live_before = row(&pool, live).await;

    // The state before the migration: no CHECK, and a redaction that emptied the bytes and kept
    // the hash of the text.
    sqlx::query("ALTER TABLE kb_block_content DROP CONSTRAINT ck_kb_block_content_emptied_hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE kb_block_content SET content = '' WHERE block_revision_id = $1")
        .bind(erased)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        row(&pool, erased).await,
        (String::new(), sha256_hex("SSN 123-45-6789")),
        "the pre-migration shape: the emptied row names its old text"
    );

    sqlx::raw_sql(block("BACKFILL"))
        .execute(&pool)
        .await
        .expect("the migration's backfill runs");
    assert_eq!(
        row(&pool, erased).await,
        (String::new(), EMPTY_SHA256.to_owned()),
        "the emptied row's hash no longer names the erased text"
    );
    assert_eq!(
        row(&pool, live).await,
        live_before,
        "a live row is untouched"
    );

    sqlx::raw_sql(block("CHECK"))
        .execute(&pool)
        .await
        .expect("the migration's CHECK validates over the backfilled table");
}
