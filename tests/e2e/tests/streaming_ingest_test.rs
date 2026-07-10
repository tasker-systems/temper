#![cfg(feature = "test-db")]

//! End-to-end coverage for streaming, resumable multi-block ingestion (Beat 4).
//!
//! Beats 1-3 (committed, green) built: the persistence + event ledger for segmented ingest
//! (`block_created` / `resource_finalized`), the segmented API surface (`begin`/`append`/
//! `finalize`/`list-blocks`, `crates/temper-api/tests/segments_handler_test.rs`), and the CLI's
//! streaming orchestration (`ingest_mode` threshold — a body over `SEGMENT_BUDGET_BYTES`
//! (256 KiB) drives `begin_segmented`/`append_block`/`finalize` with a `.temper/ingest/<id>.json`
//! resume manifest; a body at or under the budget is the unchanged one-shot `create_resource`
//! path — `crates/temper-cli/src/actions/ingest.rs`).
//!
//! This file drives that whole arc through the real Axum server + Postgres + the actual
//! `temper` CLI code paths (via `temper_cli::commands::resource::create`, the same
//! `spawn_blocking` pattern `cloud_writes_test.rs` uses) and the raw `temper_client::TemperClient`
//! segmented-ingest primitives:
//!
//! 1. `segmented_create_roundtrips_large_body` — a >1 MiB body drives the CLI's segmented path
//!    end to end; the reconstructed body, the FTS index, and the event ledger (`block_created`
//!    count > 1, `resource_finalized` count == 1) all prove the segmented path ran and landed
//!    correctly.
//! 2. `small_body_stays_one_shot` — a tiny body's one-shot create fires ZERO `block_created`/
//!    `resource_finalized` events — the no-regression guard for the common case. Driven directly
//!    via `client.ingest().create()` with bring-your-own (ONNX-free) chunks so this test runs in
//!    the plain `test-db` tier without ONNX Runtime — matching `segments_handler_test.rs`'s
//!    ONNX-free convention. `ingest_mode`'s threshold arithmetic itself is already unit-tested in
//!    `actions::ingest`'s `ingest_mode_at_or_under_budget_is_one_shot`; this test's job is the
//!    event-ledger regression guard, not re-proving the threshold math.
//! 3. `interrupted_ingest_resumes_only_the_gap` — the hardest one. `run_segmented_create` (the
//!    CLI's segmented orchestration) has no bail-out point: it always runs
//!    begin→append(everything missing)→finalize in a single call, so there is no way to drive a
//!    genuinely partial ingest through it or through a real process kill from this harness. This
//!    test instead drives the lower-level `temper_client::TemperClient` segmented primitives
//!    directly (`begin_segmented`, `append_block`, `list_blocks`, `finalize`) — landing segment 0
//!    plus one more segment, deliberately leaving a gap, then resuming via the same
//!    `list_blocks` + `actions::ingest_manifest::resume_gap` diff the CLI's resume logic uses —
//!    and proves the resume is both complete (every missing seq lands, finalize succeeds) and
//!    idempotent (the already-landed segment fires no duplicate `block_created`). This exercises
//!    the transport/resume invariant (server-side idempotent gap-fill via the ledger), not the
//!    CLI process-kill scenario; see the test's own doc comment for what a fuller version would
//!    still need.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

// ---------------------------------------------------------------------------
// Shared helpers (used by all three tests)
// ---------------------------------------------------------------------------

/// Count `event_type`-typed `kb_events` rows whose JSONB `payload->>'resource_id'` matches
/// `resource_id`. `ResourceId` is `#[serde(transparent)]` (`temper_core::types::ids`), so it
/// serializes as a bare UUID string in the payload — the same query pattern already verified in
/// `crates/temper-substrate/tests/streaming_ingest_test.rs` (Beat 1's own event-ledger tests).
async fn count_events(pool: &PgPool, event_type: &str, resource_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = $1 AND (e.payload->>'resource_id')::uuid = $2",
    )
    .bind(event_type)
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("count_events({event_type}): {e}"))
}

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free.
/// Mirrors `segments_handler_test.rs`'s helper of the same name exactly, so the small-body
/// one-shot regression guard can run without ONNX Runtime.
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

// ---------------------------------------------------------------------------
// Test 1: segmented create round-trips a >1 MiB body (Task 4.1)
// ---------------------------------------------------------------------------

/// Shared env-var builder for cloud-mode CLI invocations — verbatim copy of
/// `cloud_writes_test.rs`'s `cloud_env` (each e2e test file is its own binary; there is no
/// shared home for this tiny helper besides `common`, which doesn't own CLI-env wiring).
#[cfg(feature = "test-embed")]
fn cloud_env<'a>(
    api_url: &'a str,
    token: &'a str,
    global_config: &'a str,
) -> [(&'static str, Option<&'a str>); 4] {
    [
        ("TEMPER_API_URL", Some(api_url)),
        ("TEMPER_TOKEN", Some(token)),
        ("TEMPER_GLOBAL_CONFIG", Some(global_config)),
        ("TEMPER_AUTH_PATH", None),
    ]
}

/// Resolve a resource's id by title — mirrors `cloud_writes_test.rs`'s `created_id_for_title`,
/// but returns a `Uuid` directly (rather than a `String`) since the event-ledger queries in this
/// file bind a typed `Uuid`.
#[cfg(feature = "test-embed")]
async fn resource_id_for_title(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM kb_resources WHERE title = $1 AND is_active ORDER BY created DESC LIMIT 1",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("resource_id_for_title({title}): {e}"))
}

/// Build a markdown body that exceeds `target_bytes` (well above `SEGMENT_BUDGET_BYTES` =
/// 256 KiB), with one numbered `## Section N` heading per chunk of filler prose, plus a final
/// section carrying a distinctive `last_phrase` — used both for the body round-trip assertion
/// (proves the LAST segment reconstructs correctly, not just the first) and the FTS search
/// assertion (proves segments beyond block 0 are search-indexed). Sized at runtime (a `while
/// body.len() < target_bytes` loop) rather than precomputed from a filler string's exact byte
/// length, so the target is met regardless of the filler text chosen.
#[cfg(feature = "test-embed")]
fn generate_large_markdown(target_bytes: usize, last_phrase: &str) -> (String, usize) {
    const FILLER: &str =
        "The quick brown fox jumps over the lazy dog, padding this section well past the \
         segmentation budget so the streaming ingest pipeline must split the document into \
         multiple blocks via the segmented begin/append/finalize path.\n";
    let mut body = String::from("# Big Document\n\n");
    let mut section = 0usize;
    while body.len() < target_bytes {
        body.push_str(&format!("## Section {section}\n\n"));
        for _ in 0..40 {
            body.push_str(FILLER);
        }
        body.push('\n');
        section += 1;
    }
    // The final heading carries the distinctive phrase.
    body.push_str(&format!("## Section {section}\n\n"));
    body.push_str(last_phrase);
    body.push_str("\n\n");
    (body, section + 1)
}

/// A >1 MiB body drives the CLI's segmented (multi-block) create path end to end: the CLI's
/// `ingest_mode` threshold (`crates/temper-cli/src/actions/ingest.rs`) picks `Segmented` for a
/// body this large, `run_segmented_create` streams it through `begin_segmented`/`append_block`/
/// `finalize`, and this test verifies all three surfaces the segmented path is supposed to light
/// up: the reconstructed body, the FTS index, and the event ledger.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn segmented_create_roundtrips_large_body(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("bigctx", None)
        .await
        .expect("create bigctx context");

    // A rare, distinctive word (not "search-stop-word-safe" like a hyphenated slug would be) —
    // avoids any ambiguity in how Postgres FTS tokenizes the query.
    let last_phrase =
        "an extraordinarily rare zzyzx platypus wanders through the far edge of the archive";
    let (body, num_sections) = generate_large_markdown(1_200_000, last_phrase);
    assert!(
        body.len() > 1_048_576,
        "test body must exceed 1 MiB; got {} bytes",
        body.len()
    );

    let big_path = app.vault_dir.path().join("big.md");
    std::fs::write(&big_path, &body).expect("write big.md");
    let body_flag = format!("@{}", big_path.to_str().unwrap());

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    // Drive the CLI create on a blocking thread (it builds its own tokio runtime and
    // `.block_on()`s synchronously — same pattern `cloud_writes_test.rs` uses throughout).
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "research",
                    title: "Big",
                    context: Some("@me/bigctx"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: None,
                    body_flag: Some(body_flag),
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                    sources: Vec::new(),
                    sources_as_edges: false,
                    no_source: false,
                },
            )
            .expect("segmented cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    let resource_id = resource_id_for_title(&pool, "Big").await;

    // ---- Assertion: the segmented path ran (more than one block_created event) ----
    // Segment 0 lands as part of `resource_created` (folded in, not a separate `block_created`);
    // segments 1..N-1 each fire their own `block_created`. A >1 MiB body over a 256 KiB budget
    // produces several segments beyond 0, so this must be > 1, not just >= 1.
    let block_created = count_events(&pool, "block_created", resource_id).await;
    assert!(
        block_created > 1,
        "segmented create of a >1 MiB body must fire more than one block_created event; got {block_created}"
    );

    // ---- Assertion: resource_finalized fired exactly once ----
    assert_eq!(
        count_events(&pool, "resource_finalized", resource_id).await,
        1,
        "finalize must fire exactly one resource_finalized event"
    );

    // ---- Assertion: the reconstructed body round-trips (heading structure + last-section
    // phrase, not byte-exact — the known chunk-boundary newline normalization is tolerated) ----
    let content = app
        .client
        .resources()
        .content(resource_id)
        .await
        .expect("fetch reconstructed content");
    for i in 0..num_sections {
        let heading = format!("## Section {i}");
        assert!(
            content.markdown.contains(heading.as_str()),
            "reconstructed body missing heading: {heading}"
        );
    }
    assert!(
        content.markdown.contains(last_phrase),
        "reconstructed body must contain the last-section phrase verbatim"
    );

    // ---- Assertion: a phrase from the LAST segment is FTS-searchable ----
    let results = app
        .client
        .search()
        .text_query("zzyzx", Some("@me/bigctx".to_string()), None, Some(10))
        .await
        .expect("text search");
    assert!(
        results.iter().any(|r| r.resource_id == resource_id),
        "search for a last-segment phrase must return the segmented resource; got {:?}",
        results.iter().map(|r| r.title.as_str()).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 2: a small body stays one-shot — zero new events (Task 4.1 no-regression)
// ---------------------------------------------------------------------------

/// A body at/under the segment budget must fire ZERO `block_created`/`resource_finalized`
/// events — proving Beat 3's threshold seam doesn't regress the common (small-body) case into
/// spurious segmented-path event traffic. Driven directly via `client.ingest().create()` with
/// bring-your-own chunks (no `segmented` field set — the plain one-shot `/api/ingest` shape) so
/// this test needs no ONNX Runtime and runs in the base `test-db` tier.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn small_body_stays_one_shot(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("smallctx", None)
        .await
        .expect("create smallctx context");

    let text = "A small body, well under the segment budget — the one-shot path, unchanged.";
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Small One Shot".to_string(),
        origin_uri: format!("test://small-one-shot-{}", Uuid::new_v4()),
        context_ref: "@me/smallctx".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: text.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(one_chunk_packed(text, "11")),
        sources: Vec::new(),
        act: Default::default(),
    };
    let created = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("one-shot create");
    let resource_id = uuid::Uuid::from(created.id);

    assert_eq!(
        count_events(&pool, "block_created", resource_id).await,
        0,
        "a small one-shot create must fire zero block_created events"
    );
    assert_eq!(
        count_events(&pool, "resource_finalized", resource_id).await,
        0,
        "a small one-shot create must fire zero resource_finalized events"
    );
}

// ---------------------------------------------------------------------------
// Test 3: interrupted ingest resumes only the gap (Task 4.2)
// ---------------------------------------------------------------------------

/// A locally-planned segment: its raw text, its chunked `ChunkData` (needed for embedding), and
/// its local `(seq, block-merkle)` identity. Reimplements — from the same `pub` `temper_ingest`
/// primitives — what Beat 3's crate-private `plan_segments`/`SegmentPlan`
/// (`crates/temper-cli/src/actions/ingest.rs`) compute internally, since that helper isn't
/// reachable from this external test crate. Needed here (rather than reusing
/// `run_segmented_create`) because `run_segmented_create` has no bail-out point — it always runs
/// begin→append(everything missing)→finalize in a single call, so it cannot itself produce a
/// genuinely partial/interrupted ingest for this test to resume from.
#[cfg(feature = "test-embed")]
struct LocalSegment {
    text: String,
    chunked: Vec<temper_ingest::chunk::ChunkData>,
    info: temper_core::types::ingest::SegmentInfo,
}

#[cfg(feature = "test-embed")]
fn plan_local_segments(content: &str, budget: usize) -> Vec<LocalSegment> {
    let raw: Vec<temper_ingest::stream::Segment> =
        temper_ingest::stream::segment_reader(std::io::Cursor::new(content.as_bytes()), budget)
            .collect::<std::io::Result<Vec<_>>>()
            .expect("segment_reader over an in-memory Cursor never fails");
    raw.into_iter()
        .map(|seg| {
            let chunked = temper_ingest::chunk::chunk_markdown_with_prefix(
                &seg.text,
                &seg.initial_breadcrumb,
            );
            let chunk_hashes: Vec<String> =
                chunked.iter().map(|c| c.content_hash.clone()).collect();
            let info = temper_core::types::ingest::SegmentInfo {
                seq: seg.seq,
                content_hash: temper_ingest::merkle::block_merkle(&chunk_hashes),
            };
            LocalSegment {
                text: seg.text,
                chunked,
                info,
            }
        })
        .collect()
}

/// Embed + pack a segment's chunks into the `chunks_packed` wire format — the test's own copy of
/// `actions::ingest::embed_and_pack` (crate-private in `temper-cli`), using real ONNX embeddings
/// via `temper_ingest::embed::embed_texts` (hence this whole test is `test-embed` gated).
#[cfg(feature = "test-embed")]
fn embed_and_pack_chunks(chunks: &[temper_ingest::chunk::ChunkData]) -> String {
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = temper_ingest::embed::embed_texts(&texts).expect("embed_texts");
    let packed: Vec<PackedChunk> = chunks
        .iter()
        .zip(embeddings)
        .map(|(c, embedding)| PackedChunk {
            chunk_index: c.chunk_index,
            header_path: c.header_path.clone(),
            heading_depth: c.heading_depth,
            content: c.content.clone(),
            content_hash: c.content_hash.clone(),
            embedding,
        })
        .collect();
    pack_chunks(&packed).expect("pack_chunks")
}

/// A body sized to split into several segments under a small custom budget — small enough to
/// keep the (ONNX-gated) test fast, large enough to leave a multi-segment gap to resume.
#[cfg(feature = "test-embed")]
fn build_resume_content(num_sections: usize) -> String {
    const FILLER: &str =
        "Filler prose line for the interrupted-ingest resume test, padding each section.\n";
    let mut body = String::from("# Resume Test\n\n");
    for i in 0..num_sections {
        body.push_str(&format!("## Section {i}\n\n"));
        for _ in 0..40 {
            body.push_str(FILLER);
        }
        body.push('\n');
    }
    body
}

/// Lands segment 0 (via `begin_segmented`) plus segment 1 only, deliberately skipping segments
/// `2..N-1` — simulating an ingest interrupted after two blocks. Then "resumes" the same way the
/// CLI's `run_segmented_create` does — `list_blocks` to see what's actually durable, diff against
/// the locally-planned segments via `actions::ingest_manifest::resume_gap` (the exact function
/// the CLI's resume path uses), append only what's missing, then finalize — and proves:
///   1. the resume appends exactly the missing seqs (the gap diff is correct),
///   2. re-running the gap diff against the fully-landed set is empty (idempotent — no attempt to
///      re-send the already-landed segment 1),
///   3. the total `block_created` count after resume equals exactly one per non-zero segment (no
///      *extra* event from a duplicate append of the already-landed segment), and
///   4. finalize succeeds and fires exactly one `resource_finalized`.
///
/// Limitation vs. a real process-kill test: this drives the transport/resume primitives directly
/// (`TemperClient::ingest()` + `ingest_manifest::resume_gap`) rather than actually killing a
/// spawned `temper` CLI process mid-ingest and re-invoking `temper resource create` against the
/// same source file. A fuller version would need a real fault-injection seam in
/// `run_segmented_create` (e.g. an env-var-gated "stop after N appends" hook) plus a harness that
/// spawns the CLI as a subprocess, kills it after N appends, and re-runs `temper resource create`
/// against the same file to exercise `find_resumable`'s on-disk-manifest matching end to end — no
/// such seam exists in Beat 3's code today (confirmed: no `fault`/`inject`/`interrupt`-named hook
/// in `actions/ingest.rs` or `actions/ingest_manifest.rs`).
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn interrupted_ingest_resumes_only_the_gap(pool: PgPool) {
    use temper_core::types::ingest::{AppendBlockPayload, FinalizePayload, SegmentedBegin};

    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("resumectx", None)
        .await
        .expect("create resumectx context");

    const BUDGET: usize = 4096;
    let content = build_resume_content(10);
    let segments = plan_local_segments(&content, BUDGET);
    assert!(
        segments.len() >= 4,
        "test needs several segments to leave a meaningful gap; got {}",
        segments.len()
    );

    // ---- Begin: land segment 0 via begin_segmented ----
    let chunks0 = embed_and_pack_chunks(&segments[0].chunked);
    let begin_payload = IngestPayload {
        segmented: Some(SegmentedBegin {
            total_blocks_hint: Some(segments.len() as u32),
            block_budget: BUDGET as u32,
            source_hash: None,
        }),
        goal: None,
        title: "Interrupted Resume Test".to_string(),
        origin_uri: format!("test://resume-{}", Uuid::new_v4()),
        context_ref: "@me/resumectx".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: segments[0].text.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(chunks0),
        act: Default::default(),
        sources: Vec::new(),
    };
    let begin = app
        .client
        .ingest()
        .begin_segmented(&begin_payload)
        .await
        .expect("begin_segmented");
    let resource_id = begin.resource_id;

    // ---- Land ONLY segment 1 — deliberately skip 2..N-1, simulating an interrupted ingest ----
    let chunks1 = embed_and_pack_chunks(&segments[1].chunked);
    let append1 = AppendBlockPayload {
        seq: 1,
        content: segments[1].text.clone(),
        content_hash: temper_core::hash::sha256_hex(segments[1].text.as_bytes()),
        chunks_packed: Some(chunks1),
    };
    app.client
        .ingest()
        .append_block(resource_id, &append1)
        .await
        .expect("append seq 1");

    // ---- Assert the partial state: exactly one block_created, zero resource_finalized ----
    assert_eq!(
        count_events(&pool, "block_created", resource_id).await,
        1,
        "only segment 1's append should have landed so far"
    );
    assert_eq!(
        count_events(&pool, "resource_finalized", resource_id).await,
        0,
        "an interrupted ingest must not be finalized"
    );

    // ---- Resume: list what's actually durable, diff, append only the gap ----
    let landed = app
        .client
        .ingest()
        .list_blocks(resource_id)
        .await
        .expect("list_blocks")
        .blocks;
    let local_infos: Vec<_> = segments.iter().map(|s| s.info.clone()).collect();
    let missing = temper_cli::actions::ingest_manifest::resume_gap(&local_infos, &landed);
    assert_eq!(
        missing,
        (2..segments.len() as u32).collect::<Vec<_>>(),
        "the resume gap must be exactly the skipped seqs 2..N-1"
    );

    for seq in &missing {
        let idx = *seq as usize;
        let packed = embed_and_pack_chunks(&segments[idx].chunked);
        let append = AppendBlockPayload {
            seq: *seq,
            content: segments[idx].text.clone(),
            content_hash: temper_core::hash::sha256_hex(segments[idx].text.as_bytes()),
            chunks_packed: Some(packed),
        };
        app.client
            .ingest()
            .append_block(resource_id, &append)
            .await
            .unwrap_or_else(|e| panic!("resume append seq {seq}: {e}"));
    }

    // ---- Idempotency: the gap diff against the now-fully-landed set is empty ----
    let landed_after = app
        .client
        .ingest()
        .list_blocks(resource_id)
        .await
        .expect("list_blocks after resume")
        .blocks;
    assert!(
        temper_cli::actions::ingest_manifest::resume_gap(&local_infos, &landed_after).is_empty(),
        "after resume, every planned segment must be accounted for as landed"
    );

    // ---- No duplicate block_created: exactly one per non-zero seq, no re-send of segment 1 ----
    let expected_block_created = segments.len() as i64 - 1;
    assert_eq!(
        count_events(&pool, "block_created", resource_id).await,
        expected_block_created,
        "resuming must not re-fire block_created for the already-landed segment 1"
    );

    // ---- Finalize succeeds and fires exactly once ----
    let expected_body_hash = temper_ingest::merkle::resource_body_hash(
        &segments
            .iter()
            .map(|s| s.info.content_hash.clone())
            .collect::<Vec<_>>(),
    );
    app.client
        .ingest()
        .finalize(
            resource_id,
            &FinalizePayload {
                expected_blocks: segments.len() as u32,
                expected_body_hash,
            },
        )
        .await
        .expect("finalize");

    assert_eq!(
        count_events(&pool, "resource_finalized", resource_id).await,
        1,
        "finalize must fire exactly one resource_finalized event"
    );
}
