//! Integration test — segmented (multi-block) ingest through the real `DbBackend`
//! (Beat 2 Task 2.2): begin (block 0 via the ordinary create path, bring-your-own chunks so it's
//! ONNX-free) → append seq 1 → `list_blocks` reflects both → `finalize_ingest` succeeds against
//! the actual multi-block merkle. A second test proves `append_block` denies a non-owning
//! profile (auth-before-write, WS2) before any write lands.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::ingest::{pack_chunks, AppendBlockPayload, FinalizePayload, PackedChunk};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write path's
/// `resolve_emitter` + visibility gate require). Mirrors `open_meta_roundtrip_test.rs`'s inlined
/// fixture — each test-target crate keeps its own copy so it has no cross-target test-harness
/// dependency.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free, mirrors
/// `temper-substrate/tests/streaming_ingest_test.rs`'s `one_chunk` fixture at the wire-payload
/// layer (`PackedChunk`/`pack_chunks`) instead of the substrate-native `IncomingChunk`.
fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

/// Seed a profile + context + a segmented resource whose block 0 has landed, returning the
/// backend bound to the owning profile and the created resource.
async fn seed_segmented_resource(
    pool: &PgPool,
    email: &str,
    slug: &str,
) -> (DbBackend, temper_workflow::types::resource::ResourceRow) {
    let (profile, context) = seed_profile_with_context(pool, email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let created = backend
        .create_resource(CreateResource {
            slug: slug.to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: Some(one_chunk_packed("first segment", "aa")),
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create block 0")
        .value;
    (backend, created)
}

/// Seed a resource whose block 0 is chunked **server-side** from `body` (no caller-supplied
/// chunks), so its trailing chunk carries a real `header_path` for the append path's breadcrumb
/// carry to pick up. Requires ONNX (the server embeds inline), hence `test-embed`.
#[cfg(feature = "test-embed")]
async fn seed_resource_with_body(
    pool: &PgPool,
    email: &str,
    slug: &str,
    body: &str,
) -> (DbBackend, temper_workflow::types::resource::ResourceRow) {
    use temper_workflow::operations::BodyUpdate;
    let (profile, context) = seed_profile_with_context(pool, email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let created = backend
        .create_resource(CreateResource {
            slug: slug.to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: slug.to_string(),
            body: Some(BodyUpdate {
                content: body.to_string(),
                content_hash: None,
                chunks_packed: None,
                sources: Vec::new(),
                content_block: None,
            }),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some(format!("test://{slug}")),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create block 0 (server-chunked)")
        .value;
    (backend, created)
}

// The MCP caller: no chunker, no embedder. It sends raw segment text and the server chunks it,
// seeding the heading breadcrumb from the prior block so `header_path` stays continuous across the
// block boundary.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn server_chunks_an_append_with_no_packed_chunks_and_carries_the_breadcrumb(pool: PgPool) {
    // Block 0 ends inside "## Section", so block 1's chunks must inherit "Title > Section".
    let (backend, created) = seed_resource_with_body(
        &pool,
        "server-chunk@example.com",
        "zz-server-chunk",
        "# Title\n\nalpha\n\n## Section\n\nbeta\n",
    )
    .await;

    let text = "beta continues here\n";
    backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                content: text.to_string(),
                chunks_packed: None,
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect("server-side chunking lands the block");

    let paths: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT c.header_path FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
          WHERE b.resource_id = $1 AND b.seq = 1 AND c.is_current ORDER BY c.chunk_index",
    )
    .bind(created.id.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(!paths.is_empty(), "the appended block must have chunks");
    assert_eq!(
        paths[0].as_deref(),
        Some("Title > Section"),
        "a server-chunked segment inherits the prior block's trailing breadcrumb"
    );
}

// A server-chunked append needs prose to chunk; an empty body would otherwise reach `block_append`
// and surface as a raw "empty chunk set" database exception.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn append_with_no_chunks_and_empty_content_is_rejected(pool: PgPool) {
    let (backend, created) =
        seed_segmented_resource(&pool, "empty-append@example.com", "zz-empty-append").await;

    let err = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: String::new(),
                content_hash: temper_core::hash::sha256_hex(b""),
                chunks_packed: None,
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect_err("an empty server-chunked append must be rejected");

    assert!(
        matches!(err, TemperError::BadRequest(ref m) if m.contains("non-empty content")),
        "expected a caller-legible BadRequest, got {err:?}"
    );
}

// Begin is one command, not three: the surfaces must not compose create + record_ingestion_source
// + list_blocks themselves (the HTTP handler used to, which MCP would have had to duplicate).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn begin_segmented_ingest_lands_block_zero_and_records_the_source(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "begin@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let out = backend
        .begin_segmented_ingest(
            CreateResource {
                slug: "zz-begin-probe".to_string(),
                doctype: "research".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: "ZZ begin probe".to_string(),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                goal: None,
                origin_uri: Some("test://begin-probe".to_string()),
                chunks_packed: Some(one_chunk_packed("first segment", "aa")),
                content_hash: None,
                act: ActContext::default(),
                origin: Surface::Mcp,
            },
            temper_core::types::ingest::SegmentedBegin {
                total_blocks_hint: Some(2),
                block_budget: 262_144,
                source_hash: Some("sha256:abc".to_owned()),
            },
        )
        .await
        .expect("begin succeeds")
        .value;

    assert_eq!(out.blocks.len(), 1, "block 0 landed");
    assert_eq!(out.blocks[0].seq, 0);
    assert!(
        !out.body_hash.is_empty(),
        "begin returns the live body_hash, so a session that appends nothing can still finalize"
    );

    let source_hash: Option<String> =
        sqlx::query_scalar("SELECT source_hash FROM kb_ingestion_records WHERE resource_id = $1")
            .bind(out.resource_id)
            .fetch_one(&pool)
            .await
            .expect("begin wrote the ingestion record");
    assert_eq!(source_hash.as_deref(), Some("sha256:abc"));
}

// Every other Backend write threads `cmd.origin`; append/finalize hardcoded Surface::ApiHttp.
// Harmless while the API was the only caller — wrong the moment MCP appends a block, which would
// then be attributed to the `web` emitter. The surface marker lives in `kb_entities.name` as
// `<handle>@<surface>` (see `writes::resolve_emitter`), not in a column of its own.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_mcp_append_is_attributed_to_the_mcp_emitter(pool: PgPool) {
    let (backend, created) =
        seed_segmented_resource(&pool, "mcp-emitter@example.com", "zz-mcp-emitter").await;

    let text = "second segment";
    backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: text.to_string(),
                content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                chunks_packed: Some(one_chunk_packed(text, "bb")),
                sources: Vec::new(),
            },
            Surface::Mcp,
        )
        .await
        .expect("append succeeds");

    let emitter_name: String = sqlx::query_scalar(
        "SELECT e.name FROM kb_events ev \
           JOIN kb_entities e ON e.id = ev.emitter_entity_id \
           JOIN kb_event_types t ON t.id = ev.event_type_id \
          WHERE t.name = 'block_created' \
          ORDER BY ev.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("a block_created event was emitted");

    assert!(
        emitter_name.ends_with("@mcp"),
        "an MCP append must not be attributed to web; got {emitter_name}"
    );
}

// The declared segment-text hash is the one integrity check a caller that does not chunk locally
// can honor, so every caller honors it: a mismatch is rejected before anything lands.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn append_rejects_a_content_hash_that_does_not_match_content(pool: PgPool) {
    let (backend, created) =
        seed_segmented_resource(&pool, "hash-mismatch@example.com", "zz-hash-mismatch").await;

    let err = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: "second segment".to_string(),
                content_hash: "deadbeef".to_string(), // not sha256("second segment")
                chunks_packed: Some(one_chunk_packed("second segment", "bb")),
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect_err("a mismatched content_hash must be rejected");

    assert!(
        matches!(err, TemperError::BadRequest(ref m) if m.contains("content_hash")),
        "expected BadRequest naming content_hash, got {err:?}"
    );

    let blocks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded",
    )
    .bind(created.id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(blocks, 1, "a rejected append must land nothing");
}

// `body_hash` is what a non-chunking caller echoes back at finalize, so append must report the
// same value finalize will compare against.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn append_returns_the_live_body_hash(pool: PgPool) {
    let (backend, created) =
        seed_segmented_resource(&pool, "body-hash@example.com", "zz-body-hash").await;

    let text = "second segment";
    let out = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: text.to_string(),
                content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                chunks_packed: Some(one_chunk_packed(text, "bb")),
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect("append with a correct hash succeeds")
        .value;

    let stored: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(created.id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        out.body_hash, stored,
        "BlocksResponse.body_hash must be the value finalize will compare against"
    );

    // And it round-trips: echoing it back finalizes cleanly.
    backend
        .finalize_ingest(
            created.id,
            FinalizePayload {
                expected_blocks: 2,
                expected_body_hash: out.body_hash,
            },
            Surface::ApiHttp,
        )
        .await
        .expect("the echoed body_hash finalizes");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn segmented_ingest_begin_append_list_finalize(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "segmented@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    // Begin: block 0 lands via the ordinary create path. `chunks_packed` (bring-your-own chunks)
    // means the substrate builds the block from the chunks and never touches ONNX.
    let created = backend
        .create_resource(CreateResource {
            slug: "zz-segmented-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "ZZ segmented probe".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://segmented-probe".to_string()),
            chunks_packed: Some(one_chunk_packed("first segment", "aa")),
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create block 0")
        .value;

    // Append seq 1.
    let appended = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: "second segment".to_string(),
                content_hash: temper_core::hash::sha256_hex(b"second segment"),
                chunks_packed: Some(one_chunk_packed("second segment", "bb")),
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect("append seq 1")
        .value;
    assert_eq!(
        appended.blocks.len(),
        2,
        "append reports both currently-landed segments"
    );
    assert_eq!(appended.blocks[0].seq, 0);
    assert_eq!(appended.blocks[1].seq, 1);

    // Re-append the SAME segment — idempotent (no duplicate, same reported set).
    let reappended = backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: "second segment".to_string(),
                content_hash: temper_core::hash::sha256_hex(b"second segment"),
                chunks_packed: Some(one_chunk_packed("second segment", "bb")),
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect("re-append seq 1 is a no-op")
        .value;
    assert_eq!(reappended.blocks.len(), 2, "re-append lands no duplicate");

    // list_blocks reflects the same landed set, including the merkle content_hash.
    let listed = backend
        .list_blocks(created.id)
        .await
        .expect("list_blocks")
        .value;
    assert_eq!(listed.blocks.len(), 2);
    assert_eq!(listed.blocks[0].seq, 0);
    assert_eq!(listed.blocks[1].seq, 1);
    assert_eq!(
        listed.blocks[1].content_hash,
        appended.blocks[1].content_hash
    );

    // Finalize against the actual multi-block merkle `_recompute_resource_body_hash` maintains.
    let actual_hash: String = sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id=$1")
        .bind(created.id.uuid())
        .fetch_one(&pool)
        .await
        .expect("fetch body_hash");

    backend
        .finalize_ingest(
            created.id,
            FinalizePayload {
                expected_blocks: 2,
                expected_body_hash: actual_hash,
            },
            Surface::ApiHttp,
        )
        .await
        .expect("finalize");

    // Wrong expected_blocks is rejected (mirrors Beat 1's `finalize_validates_block_count_and_hash`).
    let bad = backend
        .finalize_ingest(
            created.id,
            FinalizePayload {
                expected_blocks: 5,
                expected_body_hash: "deadbeef".to_string(),
            },
            Surface::ApiHttp,
        )
        .await;
    assert!(bad.is_err(), "wrong expected_blocks/hash must error");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn append_by_non_owning_profile_is_forbidden(pool: PgPool) {
    let (owner, context) = seed_profile_with_context(&pool, "segmented-owner@example.com").await;
    let owner_backend = DbBackend::new(pool.clone(), ProfileId::from(owner));

    let created = owner_backend
        .create_resource(CreateResource {
            slug: "zz-segmented-auth-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "ZZ segmented auth probe".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://segmented-auth-probe".to_string()),
            chunks_packed: Some(one_chunk_packed("first segment", "cc")),
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create block 0")
        .value;

    let (other, _other_context) =
        seed_profile_with_context(&pool, "segmented-other@example.com").await;
    let other_backend = DbBackend::new(pool.clone(), ProfileId::from(other));

    // Q has no grant on P's resource — `can_modify_resource` must deny the append before any
    // write (auth-before-writes).
    let err = other_backend
        .append_block(
            created.id,
            AppendBlockPayload {
                seq: 1,
                content: "second segment".to_string(),
                content_hash: temper_core::hash::sha256_hex(b"second segment"),
                chunks_packed: Some(one_chunk_packed("second segment", "dd")),
                sources: Vec::new(),
            },
            Surface::ApiHttp,
        )
        .await
        .expect_err("non-owner append must be denied");
    assert!(
        matches!(err, TemperError::Forbidden),
        "expected Forbidden, got {err:?}"
    );

    // The same denial applies to list_blocks (brief: gated the same as append/finalize — an
    // in-progress segmented ingest's landed set is caller-private).
    let list_err = other_backend
        .list_blocks(created.id)
        .await
        .expect_err("non-owner list_blocks must be denied");
    assert!(
        matches!(list_err, TemperError::Forbidden),
        "expected Forbidden, got {list_err:?}"
    );
}
