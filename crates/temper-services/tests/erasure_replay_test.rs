#![cfg(feature = "test-db")]
//! Beat 3 witnesses: replay of a ledger containing an erasure (spec 2026-08-31 §5, the
//! falsifiable form) and the load-bearing order property — the redaction applies AT THE
//! EVENT'S LEDGER POSITION, never as a trailing pass.
//!
//! The world is built through the REAL write paths (`writes::create_resource_with`,
//! `writes::commit_blob`, `erasure_service::execute_erasure`) so the ledger replay walks is the
//! ledger production wrote, and the replay harness is the SAME `replay::snapshot` /
//! `replay::replay` / `replay::dump_projections` the substrate artifact tests use. The namespace
//! reset replicates the substrate tests' `common::reset_schema` contract (drop → re-apply the
//! migration chain → truncate the kb_% data tables to a seed-free baseline) because that helper
//! lives in temper-substrate's own test tree, which this crate cannot import.
//!
//! The substrate-side pins this file leans on and does not re-prove: `blobs.rs`'
//! `replay_reproduces_a_strike` (a strike + a post-strike re-commit replay to an emptied row
//! beside a fresh live one) and `a_recommit_into_a_struck_row_s_slot_mints_a_fresh_row` (the
//! re-commit rule itself, for the `blob_deleted` arm).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::services::erasure_service::{execute_erasure, ErasureOutcome};
use temper_substrate::blob_store::{blob_pathname, InMemoryBlobStore};
use temper_substrate::content::{self, IncomingChunk};
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, ContextId, EntityId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::replay;
use temper_substrate::writes::{self, CommitBlobParams, CreateParams};

const ALLOWLIST: [&str; 1] = ["image/png"];
const CAP: i64 = 10 * 1024 * 1024;

fn allowlist() -> &'static [String] {
    static ALLOW: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    ALLOW.get_or_init(|| ALLOWLIST.iter().map(|s| to_owned(s)).collect())
}
fn to_owned(s: &str) -> String {
    s.to_string()
}

/// Reset the schema IN THE CURRENT DATABASE to a clean, un-seeded baseline — the substrate
/// tests' `common::reset_schema` contract, applied through the real MIGRATOR (no file-system
/// re-walk, and no scenario-baseline exclusions: this crate runs the full chain).
async fn reset_namespace(pool: &PgPool) {
    use sqlx::Executor;
    pool.execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("drop/recreate public schema");
    temper_substrate::MIGRATOR
        .run(pool)
        .await
        .expect("re-apply the migration chain");
    pool.execute(
        "DO $$ DECLARE r record; BEGIN \
           FOR r IN SELECT tablename FROM pg_tables \
                     WHERE schemaname = 'public' AND tablename LIKE 'kb\\_%' \
           LOOP EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' RESTART IDENTITY CASCADE'; \
           END LOOP; END $$;",
    )
    .await
    .expect("truncate kb_ data tables to a seed-free baseline");
}

/// Minimal profile + its `<handle>@web` emitter entity (the erasure_service fixture shape: the
/// handle is the FULL id, so same-millisecond uuidv7 handles cannot collide).
async fn insert_profile(pool: &PgPool) -> (Uuid, String) {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
                 VALUES ($1, $2, $2, $3, '{}'::jsonb)",
    )
    .bind(id)
    .bind(&handle)
    .bind(format!("{handle}@x.test"))
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("{handle}@web"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    (id, handle)
}

/// A governed (personal) context owned by `subject` — the erasure act's governed home.
async fn insert_personal_context(pool: &PgPool, subject: Uuid, slug: &str) -> ContextId {
    ContextId::from(
        sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
                 VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
        )
        .bind(subject)
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("seed personal context"),
    )
}

/// A single-chunk resource homed in `home`, built through the REAL create path with a
/// bring-your-own chunk (no ONNX). The chunk carries a live embedding + provenance so the
/// erasure has something to null, and a distinct 64-hex hash like the chunker mints.
async fn seed_resource(
    pool: &PgPool,
    subject: Uuid,
    emitter: Uuid,
    home: ContextId,
    title: &str,
    prose: &str,
) -> (Uuid, String) {
    let chunk_hash = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(prose.trim()))
    };
    let chunk = IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash.clone(),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: String::new(),
        heading_depth: 0,
    };
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{title}"),
            body: prose,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: ProfileId::from(subject),
            originator: ProfileId::from(subject),
            emitter: EntityId::from(emitter),
            properties: &[],
            chunks: Some(vec![chunk]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed resource through the create path");
    let resource: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_resources WHERE origin_uri = $1 ORDER BY id")
            .bind(format!("test://{title}"))
            .fetch_one(pool)
            .await
            .expect("created resource row");
    (resource, chunk_hash)
}

fn blob_params(
    home: ContextId,
    subject: Uuid,
    emitter: Uuid,
    bytes: &[u8],
) -> (CommitBlobParams<'static>, String, String) {
    use sha2::Digest;
    let hash = format!("{:x}", sha2::Sha256::digest(bytes));
    let pathname = blob_pathname(&hash);
    (
        CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: AnchorRef::context(home),
            owner: ProfileId::from(subject),
            originator: None,
            content_hash: hash.clone(),
            content_type: "image/png".to_string(),
            content_bytes: bytes.len() as i64,
            max_bytes: CAP,
            allowlist: allowlist(),
            emitter: EntityId::from(emitter),
        },
        hash,
        pathname,
    )
}
use temper_substrate::ids::BlobId;

/// A live blob row committed through the REAL commit path; the caller pre-populates the store
/// (that IS the upload in a fake world).
async fn seed_blob(
    pool: &PgPool,
    store: &InMemoryBlobStore,
    home: ContextId,
    subject: Uuid,
    emitter: Uuid,
    bytes: &[u8],
) -> (Uuid, String) {
    let (p, hash, pathname) = blob_params(home, subject, emitter, bytes);
    store.insert(pathname);
    let blob = writes::commit_blob(pool, store, p)
        .await
        .expect("commit blob");
    (blob.uuid(), hash)
}

/// The live post-erasure shape of the world, read back as scalars so the criterion can assert
/// the redacted rows BOTH in the live namespace and in each replayed one.
struct ErasedWorld {
    chunk: Uuid,
    chunk_hash: String,
    blob: Uuid,
    blob_hash: String,
}

async fn assert_redacted_shape(pool: &PgPool, w: &ErasedWorld) {
    let (content, hash): (String, String) = sqlx::query_as(
        "SELECT cc.content, c.content_hash FROM kb_chunk_content cc \
           JOIN kb_chunks c ON c.id = cc.chunk_id WHERE cc.chunk_id = $1",
    )
    .bind(w.chunk)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(content, "", "the chunk prose is emptied");
    assert_eq!(hash, w.chunk_hash, "the chunk hash is retained (D3)");

    let (emb, prov): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT embedding::text, embedded_with FROM kb_chunks WHERE id = $1")
            .bind(w.chunk)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        (emb, prov),
        (None, None),
        "embedding + provenance nulled together"
    );

    let (p_type, p_hash): (Option<String>, String) =
        sqlx::query_as("SELECT content_type, content_hash FROM kb_blobs WHERE id = $1")
            .bind(w.blob)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(p_type, None, "the blob row is emptied (D5.2)");
    assert_eq!(p_hash, w.blob_hash, "the blob hash is retained");

    let in_set: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_erased_content WHERE content_hash = ANY($1)")
            .bind(&[w.chunk_hash.clone(), w.blob_hash.clone()])
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(in_set, 2, "the erased-content set holds both hashes (D4)");
}

fn diff_projections(before: &[(String, serde_json::Value)], after: &[(String, serde_json::Value)]) {
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }
}

// ── THE criterion (spec §5, the falsifiable form) ─────────────────────────────────────────────

/// FAILS IF replay does not reproduce the redacted projection byte-identically: commit content +
/// blob rows for a subject, erase through the Beat 2 service, replay the ledger into a clean
/// namespace → the projection diff must be exact (including the refilled erased-content set,
/// which is in the diff precisely so this is checked rather than claimed), the redacted shape
/// must hold row-for-row, erase again on the replayed namespace → no-op completion, replay
/// again → still byte-identical.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_an_erasure_is_byte_identical_and_a_replayed_re_erase_is_a_no_op(
    pool: sqlx::PgPool,
) {
    let (subject, _) = insert_profile(&pool).await;
    let (operator, _) = insert_profile(&pool).await;
    temper_services::test_support::grant_governance(&pool, operator).await;
    let home = insert_personal_context(&pool, subject, "notes").await;
    let emitter: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@web'",
    )
    .bind(subject)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (resource, chunk_hash) = seed_resource(
        &pool,
        subject,
        emitter,
        home,
        "secret plans",
        "the secret plan prose",
    )
    .await;
    let bytes = b"\x89PNG-secret-bytes".to_vec();
    let store = InMemoryBlobStore::default();
    let (blob, blob_hash) = seed_blob(&pool, &store, home, subject, emitter, &bytes).await;
    let _ = resource;

    let outcome = execute_erasure(
        &pool,
        ProfileId::from(operator),
        ProfileId::from(subject),
        Uuid::now_v7(),
    )
    .await
    .expect("the operator's act completes");
    let ErasureOutcome::Completed(_completion) = outcome else {
        panic!("must complete, got {outcome:?}");
    };
    let world = ErasedWorld {
        chunk: sqlx::query_scalar("SELECT id FROM kb_chunks WHERE content_hash = $1")
            .bind(&chunk_hash)
            .fetch_one(&pool)
            .await
            .unwrap(),
        chunk_hash,
        blob,
        blob_hash,
    };
    assert_redacted_shape(&pool, &world).await;

    // ── replay #1 into a clean namespace ──
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    reset_namespace(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    diff_projections(&before, &after);
    assert_redacted_shape(&pool, &world).await;

    // ── erase again on the REPLAYED namespace: the no-op completion (spec §5) ──
    let outcome = execute_erasure(
        &pool,
        ProfileId::from(operator),
        ProfileId::from(subject),
        Uuid::now_v7(),
    )
    .await
    .expect("the re-erase on the replayed namespace completes");
    let ErasureOutcome::Completed(re_erase) = outcome else {
        panic!("a re-erase is a completion, never a refusal");
    };
    assert!(re_erase.already_erased, "the no-op completion says so");
    assert!(
        re_erase
            .targets
            .iter()
            .all(|t| t.outcome == "already-erased"),
        "targets ALL report already-erased, got {:?}",
        re_erase.targets
    );
    // First-admit attribution survived the round-trip AND the re-erase: every set row still
    // cites the ORIGINAL completing event, on the replayed namespace as on the live one.
    let distinct: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT erased_by_event_id) FROM kb_erased_content",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        distinct,
        (2, 1),
        "the set holds both hashes, attributed to the ONE first-admitting event — the re-erase's \
         own new completion did not steal attribution (ON CONFLICT DO NOTHING)"
    );

    // ── replay #2 (of the ledger extended by the re-erase): still byte-identical ──
    let before2 = replay::dump_projections(&pool).await.unwrap();
    let snap2 = replay::snapshot(&pool).await.unwrap();
    reset_namespace(&pool).await;
    replay::replay(&pool, &snap2).await.unwrap();
    let after2 = replay::dump_projections(&pool).await.unwrap();
    diff_projections(&before2, &after2);
    assert_redacted_shape(&pool, &world).await;
    let admits: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erased_content")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        admits, 2,
        "the second replay's redaction arm re-fires idempotently: still exactly two admits"
    );
}

// ── the order property: post-erasure re-commits SURVIVE replay ────────────────────────────────

/// FAILS IF the redaction is applied as a trailing pass (or ever widened to reach past a later
/// event): after the erasure event in the ledger, a commit of IDENTICAL blob bytes mints a FRESH
/// LIVE row (the substrate re-commit rule, 20260906000010 — never refused, never deduplicated
/// against the struck one) and a text re-write lands a NEW hash with its prose — and REPLAY
/// must reproduce both as LIVE, beside the emptied pre-erasure rows. A trailing redaction pass
/// over the walked state cannot distinguish "content the erasure emptied" from "content a later
/// event lawfully put"; position does.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn post_erasure_recommits_survive_replay(pool: sqlx::PgPool) {
    let (subject, _) = insert_profile(&pool).await;
    let (operator, _) = insert_profile(&pool).await;
    temper_services::test_support::grant_governance(&pool, operator).await;
    let home = insert_personal_context(&pool, subject, "notes").await;
    let emitter: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@web'",
    )
    .bind(subject)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (resource, chunk_hash) = seed_resource(
        &pool,
        subject,
        emitter,
        home,
        "secret plans",
        "the secret plan prose",
    )
    .await;
    let block: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1 LIMIT 1")
            .bind(resource)
            .fetch_one(&pool)
            .await
            .unwrap();
    let bytes = b"\x89PNG-secret-bytes".to_vec();
    let store = InMemoryBlobStore::default();
    let (blob, blob_hash) = seed_blob(&pool, &store, home, subject, emitter, &bytes).await;

    let outcome = execute_erasure(
        &pool,
        ProfileId::from(operator),
        ProfileId::from(subject),
        Uuid::now_v7(),
    )
    .await
    .expect("completes");
    let ErasureOutcome::Completed(_) = outcome else {
        panic!("must complete");
    };

    // ── AFTER the erasure event, in the same ledger ──
    // (a) The blob re-commit: identical bytes, the store re-put, a FRESH row.
    let (recommit_p, recommit_hash, recommit_path) = blob_params(home, subject, emitter, &bytes);
    store.insert(recommit_path);
    let fresh_blob = writes::commit_blob(&pool, &store, recommit_p)
        .await
        .expect("the re-commit is lawful (never deduplicated against the struck row)");
    assert_ne!(
        fresh_blob.uuid(),
        blob,
        "the re-commit mints a FRESH row, never the struck identity"
    );
    let (f_type, f_hash): (Option<String>, String) =
        sqlx::query_as("SELECT content_type, content_hash FROM kb_blobs WHERE id = $1")
            .bind(fresh_blob.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        f_type.as_deref(),
        Some("image/png"),
        "the fresh row is LIVE"
    );
    assert_eq!(f_hash, recommit_hash);
    assert_eq!(recommit_hash, blob_hash, "identical bytes");

    // (b) The text re-write: NEW prose → NEW hash, through the real revise path
    //     (deferred chunks — the async-embed shape, ONNX-free).
    let new_prose = "a wholly different and lawful assertion";
    let prepared = content::prepare_block_deferred(0, None, new_prose);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block: BlockId::from(block),
            chunks: &prepared.chunks,
            raw: Some(new_prose),
            incorporated: &[],
            replaces_body: false,
            emitter: EntityId::from(emitter),
        },
    )
    .await
    .expect("the re-write proceeds (its hash is not in the erased set)");
    tx.commit().await.unwrap();
    let new_hash: String =
        sqlx::query_scalar("SELECT content_hash FROM kb_chunks WHERE block_id = $1 AND is_current")
            .bind(block)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(
        new_hash, chunk_hash,
        "the re-write's hash is NEW — the erased hash is not reused"
    );

    // ── replay: the pre-erasure rows stay redacted, the post-erasure commits stay LIVE ──
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    reset_namespace(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    diff_projections(&before, &after);

    let (f_type,): (Option<String>,) =
        sqlx::query_as("SELECT content_type FROM kb_blobs WHERE id = $1")
            .bind(fresh_blob.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        f_type.as_deref(),
        Some("image/png"),
        "the post-erasure re-commit replays LIVE, not emptied — the redaction ran at its \
         event's position, before this commit existed"
    );
    let (emptied,): (Option<String>,) =
        sqlx::query_as("SELECT content_type FROM kb_blobs WHERE id = $1")
            .bind(blob)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(emptied.is_none(), "the struck row stays emptied");

    let (prose, is_current): (String, bool) = sqlx::query_as(
        "SELECT cc.content, c.is_current FROM kb_chunk_content cc \
           JOIN kb_chunks c ON c.id = cc.chunk_id \
          WHERE c.content_hash = $1",
    )
    .bind(&new_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        is_current && prose == new_prose,
        "the post-erasure re-write replays LIVE: current, prose intact"
    );

    // The erased hash is dead forever on the text side; the new hash was never admitted to the
    // set. (The blob hash IS in the set — the act struck the bytes — while the re-committed
    // LIVE row beside it is exactly the substrate's ruled shape.)
    let set: Vec<String> =
        sqlx::query_scalar("SELECT content_hash FROM kb_erased_content ORDER BY content_hash")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        set.contains(&chunk_hash) && set.contains(&blob_hash),
        "the set holds the pre-erasure hashes, got {set:?}"
    );
    assert!(
        !set.contains(&new_hash),
        "a hash minted after the erasure never enters the set"
    );
    let _ = world_shape(&pool, &chunk_hash).await;
}

/// Keep the redacted-shape assertion reachable for the second test without re-deriving the
/// chunk id twice: the pre-erasure chunk row must still be emptied (hash kept) after replay.
async fn world_shape(pool: &PgPool, chunk_hash: &str) -> (String, String) {
    sqlx::query_as(
        "SELECT cc.content, c.content_hash FROM kb_chunk_content cc \
           JOIN kb_chunks c ON c.id = cc.chunk_id WHERE c.content_hash = $1",
    )
    .bind(chunk_hash)
    .fetch_one(pool)
    .await
    .expect("the pre-erasure chunk row survives (D3: emptied, not deleted)")
}
