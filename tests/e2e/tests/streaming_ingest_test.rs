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
        embedded_with: None,
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

/// A body spanning several segment budgets drives the CLI's segmented (multi-block) create path end
/// to end: the CLI's `ingest_mode` threshold (`crates/temper-cli/src/actions/ingest.rs`) picks
/// `Segmented` for a body over `SEGMENT_BUDGET_BYTES`, `run_segmented_create` streams it through
/// `begin_segmented`/`append_block`/`finalize`, and this test verifies all three surfaces the
/// segmented path is supposed to light up: the reconstructed body, the FTS index, and the event
/// ledger. Sized to the *minimum* that fires >1 `block_created` (see the body-size note below) — big
/// enough to be genuinely multi-segment, small enough not to sit at nextest's 300 s cap.
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
    // Sized to span >=3 segment budgets (so segments 1..N fire >1 `block_created` — the assertion
    // below), NOT to be "large" for its own sake. The old 1.2 MB / ~939-chunk body existed to stress
    // throughput, but throughput here is a DEBUG-BUILD ARTIFACT: this harness embeds ~single-core
    // (~4 min for that body) while the release CLI is multi-core (~55 s) — so the size was testing the
    // harness's unrepresentative slowness, and at ~939 chunks it sat right at nextest's 300 s cap and
    // tipped over on a loaded CI runner. ~640 KiB still exercises the exact multi-segment path
    // (begin → several appends → finalize → roundtrip) with comfortable headroom. Real embed
    // throughput is the perf sub-thread of the parent task, not this test's job.
    let target = 3 * temper_ingest::stream::SEGMENT_BUDGET_BYTES - 128 * 1024; // 640 KiB ⇒ 3 segments
    let (body, num_sections) = generate_large_markdown(target, last_phrase);
    assert!(
        // >2 budgets ⇒ >=3 segments ⇒ >1 block_created (segment 0 rides `resource_created`).
        body.len() > 2 * temper_ingest::stream::SEGMENT_BUDGET_BYTES,
        "test body must span >2 segment budgets (got {} bytes, budget {})",
        body.len(),
        temper_ingest::stream::SEGMENT_BUDGET_BYTES
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
            // This helper really did embed, so it declares the model — exactly as the CLI does. Leaving
            // it None would make the e2e exercise the *undeclared* path while production exercises the
            // declared one, which is the sort of divergence this whole field exists to prevent.
            embedded_with: Some(temper_ingest::embed::EXPECTED_MODEL_SHA256.to_owned()),
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
        sources: Vec::new(),
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
            sources: Vec::new(),
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
                expected_content_hash: None,
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

/// **W2 PR 1 (#420 set 3): an interrupted ingest is not a document.**
///
/// The bug this closes: a ~1.2 MB ingest was killed mid-upload and left a resource holding 93.2% of
/// its source, `status: ok` everywhere it was visible — listed, searchable, indistinguishable from a
/// complete document without diffing the source. A knowledge base that cannot tell you what it does
/// not have is worse than one that fails loudly.
///
/// Drives the same begin→append-without-finalize shape as `interrupted_ingest_resumes_only_the_gap`,
/// then asserts the four properties that make the partial *honest*:
///   1. it is **absent from list**,
///   2. it is **absent from search**,
///   3. it is **still addressable via `show`**, which reports `ingest_state = "in_progress"` — hidden
///      is not deleted; the owner must be able to see and resume it, and
///   4. `finalize` flips it to `complete` and it reappears in list.
///
/// Plus the regression guard that the *first* draft of this work would have failed: a completed
/// one-shot resource in the same context stays visible throughout. The tempting backfill heuristic —
/// "multi-block AND no `resource_finalized` ⇒ incomplete" — matches every cognitive-map charter in
/// production (including the L0 kernel), because `charter_set` projects a multi-block set and never
/// finalizes. MULTI-BLOCK DOES NOT MEAN SEGMENTED. Hence: no backfill, and `in_progress` is only ever
/// *born* at a segmented begin.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn interrupted_ingest_is_not_a_document(pool: PgPool) {
    use temper_core::types::ingest::{AppendBlockPayload, FinalizePayload, SegmentedBegin};
    use temper_workflow::types::resource::ResourceListParams;

    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("partialctx", None)
        .await
        .expect("create partialctx context");

    // A complete, ordinary one-shot resource in the same context. It must stay visible in list and
    // search the entire time — the guard that the completeness gate hides ONLY the partial.
    let whole_body = "# Complete Neighbour\n\nThis document is whole and must stay visible.\n";
    let whole = app
        .client
        .ingest()
        .create(&IngestPayload {
            segmented: None,
            goal: None,
            title: "Complete Neighbour".to_string(),
            origin_uri: format!("test://whole-{}", Uuid::new_v4()),
            context_ref: "@me/partialctx".to_string(),
            home_cogmap_id: None,
            doc_type_name: "research".to_string(),
            content_hash: None,
            content: whole_body.to_string(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: None,
            act: Default::default(),
            sources: Vec::new(),
        })
        .await
        .expect("one-shot create");
    let whole_id = uuid::Uuid::from(whole.id);

    // A one-shot create is ATOMIC — there is no interruption window, so it is born complete.
    let whole_row = app
        .client
        .resources()
        .get(whole_id)
        .await
        .expect("show the whole resource")
        .row;
    assert_eq!(
        whole_row.ingest_state,
        Some(temper_workflow::types::IngestState::Complete),
        "a one-shot create is atomic and must be born `complete`"
    );

    // ---- Now: begin a segmented ingest, land one more block, and DIE. No finalize. ----
    const BUDGET: usize = 4096;
    let content = build_resume_content(10);
    let segments = plan_local_segments(&content, BUDGET);
    assert!(segments.len() >= 4, "need a multi-segment body");

    let begin_payload = IngestPayload {
        segmented: Some(SegmentedBegin {
            total_blocks_hint: Some(segments.len() as u32),
            block_budget: BUDGET as u32,
            source_hash: None,
        }),
        goal: None,
        title: "Killed Mid Upload".to_string(),
        origin_uri: format!("test://partial-{}", Uuid::new_v4()),
        context_ref: "@me/partialctx".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: segments[0].text.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(embed_and_pack_chunks(&segments[0].chunked)),
        act: Default::default(),
        sources: Vec::new(),
    };
    let begin = app
        .client
        .ingest()
        .begin_segmented(&begin_payload)
        .await
        .expect("begin_segmented");
    let partial_id = begin.resource_id;

    app.client
        .ingest()
        .append_block(
            partial_id,
            &AppendBlockPayload {
                seq: 1,
                content: segments[1].text.clone(),
                content_hash: temper_core::hash::sha256_hex(segments[1].text.as_bytes()),
                chunks_packed: Some(embed_and_pack_chunks(&segments[1].chunked)),
                sources: Vec::new(),
            },
        )
        .await
        .expect("append seq 1");
    // ...and then the process is killed. Segments 2..N never arrive; finalize never runs.

    async fn list_ids(app: &common::E2eTestApp) -> Vec<uuid::Uuid> {
        app.client
            .resources()
            .list(&ResourceListParams {
                context_ref: Some("@me/partialctx".to_string()),
                limit: Some(50),
                ..Default::default()
            })
            .await
            .expect("list")
            .rows
            .into_iter()
            .map(|r| uuid::Uuid::from(r.id))
            .collect()
    }

    // ---- 1. Absent from list ----
    let ids = list_ids(&app).await;
    assert!(
        !ids.contains(&partial_id),
        "an unfinalized segmented ingest must NOT be listed — that is the whole bug"
    );
    assert!(
        ids.contains(&whole_id),
        "the completeness gate must hide ONLY the partial; the whole neighbour stays listed"
    );

    // ---- 2. Absent from search ----
    // "Filler prose line" is the distinctive phrase in every landed segment of the partial, so a hit
    // would mean its chunks reached the index as if the document were whole.
    let hits = app
        .client
        .search()
        .text_query(
            "Filler prose line padding section",
            Some("@me/partialctx".into()),
            None,
            Some(20),
        )
        .await
        .expect("search");
    assert!(
        !hits.iter().any(|h| h.resource_id == partial_id),
        "an unfinalized segmented ingest must NOT surface in search"
    );

    // ---- 3. Still addressable via `show`, and honest about why it is hidden ----
    let shown = app
        .client
        .resources()
        .get(partial_id)
        .await
        .expect("show must still work on a partial — hidden is not deleted")
        .row;
    assert_eq!(
        shown.ingest_state,
        Some(temper_workflow::types::IngestState::InProgress),
        "`show` must say the resource is incomplete, so a reader can tell a partial from a document"
    );

    // ---- 4. Finalize completes it, and it reappears ----
    for seq in 2..segments.len() as u32 {
        let idx = seq as usize;
        app.client
            .ingest()
            .append_block(
                partial_id,
                &AppendBlockPayload {
                    seq,
                    content: segments[idx].text.clone(),
                    content_hash: temper_core::hash::sha256_hex(segments[idx].text.as_bytes()),
                    chunks_packed: Some(embed_and_pack_chunks(&segments[idx].chunked)),
                    sources: Vec::new(),
                },
            )
            .await
            .expect("append the gap");
    }
    app.client
        .ingest()
        .finalize(
            partial_id,
            &FinalizePayload {
                expected_blocks: segments.len() as u32,
                expected_body_hash: temper_ingest::merkle::resource_body_hash(
                    &segments
                        .iter()
                        .map(|s| s.info.content_hash.clone())
                        .collect::<Vec<_>>(),
                ),
                expected_content_hash: None,
            },
        )
        .await
        .expect("finalize");

    let shown = app
        .client
        .resources()
        .get(partial_id)
        .await
        .expect("show after finalize")
        .row;
    assert_eq!(
        shown.ingest_state,
        Some(temper_workflow::types::IngestState::Complete),
        "finalize must project `complete` — the event is no longer projection-less"
    );
    assert!(
        list_ids(&app).await.contains(&partial_id),
        "a finalized resource must reappear in list"
    );

    // The differential that proves the search assertion above was not vacuous: the SAME query over
    // the SAME corpus now finds it. Nothing changed but `ingest_state` — so the earlier absence was
    // the completeness gate doing its job, not a query that never matched.
    let hits = app
        .client
        .search()
        .text_query(
            "Filler prose line padding section",
            Some("@me/partialctx".into()),
            None,
            Some(20),
        )
        .await
        .expect("search after finalize");
    assert!(
        hits.iter().any(|h| h.resource_id == partial_id),
        "after finalize the same query must find it — otherwise the absence above proved nothing"
    );
}

// ---------------------------------------------------------------------------
// Test: the CLI DISCARDS a corrupt upload on a finalize integrity failure (W2 PR 5)
// ---------------------------------------------------------------------------

/// **W2 PR 5: the CLI's integrity-failure RECOVERY path, driven end to end.**
///
/// When finalize returns `422 CONTENT_INTEGRITY` (the stored bytes don't match the source hash),
/// `run_segmented_create` must NOT loop by resuming into the same failure — it must DISCARD the
/// poisoned `in_progress` resource + the local resume manifest and surface an actionable error.
///
/// Forcing a real mismatch without a prod seam: the CLI computes a correct hash by construction, so
/// this test (1) lands every segment via the client primitives WITHOUT finalizing, (2) writes the
/// resume manifest the CLI will find, (3) CORRUPTS one block's stored RAW bytes directly in
/// `kb_block_content` — which leaves the block's chunk-merkle (`block_body_hash`) untouched, so the
/// CLI's `resume_gap` still sees every block as landed and skips straight to finalize — then (4)
/// drives the real `temper resource create`, which resumes → finalizes → 422 → recovery. This
/// simulates exactly the at-rest byte corruption the check exists to catch.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_discards_a_corrupt_upload_on_integrity_failure(pool: PgPool) {
    use temper_core::types::ingest::{AppendBlockPayload, SegmentedBegin};

    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("corruptctx", None)
        .await
        .expect("create corruptctx context");

    // A body over SEGMENT_BUDGET_BYTES ⇒ the CLI takes the segmented (run_segmented_create) path,
    // the only path with the integrity-recovery branch. Minimal size that crosses one budget.
    let budget = temper_ingest::stream::SEGMENT_BUDGET_BYTES;
    let (content, _sections) =
        generate_large_markdown(budget + 64 * 1024, "corrupt-recovery-test tail phrase");
    let source_hash = temper_core::hash::sha256_hex(content.as_bytes());
    let segments = plan_local_segments(&content, budget);
    assert!(
        segments.len() >= 2,
        "need >=2 segments; got {}",
        segments.len()
    );

    // (1) Land ALL segments via the primitives, NOT finalized — an in_progress resource.
    let begin = app
        .client
        .ingest()
        .begin_segmented(&IngestPayload {
            segmented: Some(SegmentedBegin {
                total_blocks_hint: Some(segments.len() as u32),
                block_budget: budget as u32,
                source_hash: Some(source_hash.clone()),
            }),
            goal: None,
            title: "Corrupt Upload".to_string(),
            origin_uri: format!("test://corrupt-{}", Uuid::new_v4()),
            context_ref: "@me/corruptctx".to_string(),
            home_cogmap_id: None,
            doc_type_name: "research".to_string(),
            content_hash: None,
            content: segments[0].text.clone(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: Some(embed_and_pack_chunks(&segments[0].chunked)),
            act: Default::default(),
            sources: Vec::new(),
        })
        .await
        .expect("begin_segmented");
    let resource_id = begin.resource_id;
    for seg in segments.iter().skip(1) {
        app.client
            .ingest()
            .append_block(
                resource_id,
                &AppendBlockPayload {
                    seq: seg.info.seq,
                    content: seg.text.clone(),
                    content_hash: temper_core::hash::sha256_hex(seg.text.as_bytes()),
                    chunks_packed: Some(embed_and_pack_chunks(&seg.chunked)),
                    sources: Vec::new(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("append seq {}: {e}", seg.info.seq));
    }

    // (2) Write the resume manifest the CLI's find_resumable will match (source_hash + budget +
    // segmenter_version, finalized=false).
    let landed = app
        .client
        .ingest()
        .list_blocks(resource_id)
        .await
        .expect("list_blocks")
        .blocks;
    let manifest_path =
        temper_cli::actions::ingest_manifest::manifest_path(app.vault_dir.path(), resource_id);
    temper_cli::actions::ingest_manifest::store(
        &manifest_path,
        &temper_cli::actions::ingest_manifest::IngestManifest {
            resource_id,
            source_hash: source_hash.clone(),
            block_budget: budget as u32,
            segmenter_version: temper_cli::actions::ingest_manifest::SEGMENTER_VERSION,
            correlation_id: begin.correlation_id,
            blocks: landed,
            finalized: false,
        },
    )
    .expect("store manifest");

    // (3) Corrupt seg 0's stored RAW bytes. This changes what the finalize integrity check hashes
    // (concat of raw block bytes) WITHOUT touching the block chunk-merkle, so resume_gap stays empty
    // and the CLI skips to finalize rather than trying (and failing) to re-append.
    sqlx::query(
        "UPDATE kb_block_content SET content = content || 'CORRUPT' \
         WHERE block_revision_id = ( \
             SELECT current_revision_id FROM kb_content_blocks \
             WHERE resource_id = $1 AND seq = 0 AND NOT is_folded)",
    )
    .bind(resource_id)
    .execute(&pool)
    .await
    .expect("corrupt stored bytes");

    // (4) Drive the REAL CLI create → find_resumable → resume → finalize → 422 → recovery.
    let big_path = app.vault_dir.path().join("corrupt.md");
    std::fs::write(&big_path, &content).expect("write corrupt.md");
    let body_flag = format!("@{}", big_path.to_str().unwrap());
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config = app
        .vault_dir
        .path()
        .join("no-such-config.toml")
        .to_str()
        .unwrap()
        .to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "research",
                    title: "Corrupt Upload",
                    context: Some("@me/corruptctx"),
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
        })
    })
    .await
    .expect("spawn_blocking joined");

    // The CLI must surface a distinct ContentIntegrity error (not a generic error, not success).
    match result {
        Err(temper_core::error::TemperError::ContentIntegrity(msg)) => assert!(
            msg.contains("integrity") && msg.contains("discarded"),
            "the error must explain what happened and that the upload was discarded; got: {msg}"
        ),
        other => panic!("expected a ContentIntegrity error from the CLI, got: {other:?}"),
    }

    // The poisoned resource was DISCARDED (soft-deleted) so it cannot linger as an un-finalizable
    // in_progress leak, and it was never finalized.
    let active: Option<bool> =
        sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
            .bind(resource_id)
            .fetch_optional(&pool)
            .await
            .expect("query is_active");
    assert_eq!(
        active,
        Some(false),
        "the poisoned resource must be discarded (is_active=false), not left in_progress forever"
    );
    assert_eq!(
        count_events(&pool, "resource_finalized", resource_id).await,
        0,
        "the corrupt upload must never have been finalized"
    );

    // The local resume manifest was removed, so a re-run starts a clean fresh upload rather than
    // resuming straight back into the same integrity failure.
    assert!(
        !manifest_path.exists(),
        "the resume manifest must be removed on an integrity failure"
    );
}
