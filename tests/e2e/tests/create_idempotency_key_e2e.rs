#![cfg(feature = "test-db")]
//! A `temper resource create` re-issued with the same `--idempotency-key` returns the first
//! write's resource, never a second; different keys create two.
//!
//! The server claims `(owner, idempotency_key)` atomically with the create (issue #581); this
//! drives the real binary against the in-process API, so the key travels the CLI's own path. A
//! retry after a lost answer is exactly this: a second invocation carrying the first one's key.
//! That one invocation sends one key on every attempt, and that a network error names it, is
//! witnessed in `crates/temper-cli/tests/create_idempotency_key_test.rs`.
//!
//! A body over `SEGMENT_BUDGET_BYTES` takes the segmented path, which resumes an interrupted
//! attempt from a local manifest under the key that attempt saved. The last three tests hold the
//! key's promise there: a key the caller chose is refused rather than silently replaced, a minted
//! key gives way to the saved one and the hint names the saved one, and a keyed large create
//! re-issued is still one resource.

mod common;

use serde_json::Value;
use uuid::Uuid;

const CONTEXT: &str = "idempotency-key-ctx";

async fn setup(pool: sqlx::PgPool) -> common::E2eTestApp {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create(CONTEXT, None)
        .await
        .expect("ctx create");
    app
}

/// Run `temper --format json resource create --type session …` with `key`, plus `extra` args.
async fn run_create(
    app: &common::E2eTestApp,
    title: &str,
    key: &str,
    extra: &[&str],
) -> std::process::Output {
    let context = format!("@me/{CONTEXT}");
    let mut args = vec![
        "--format",
        "json",
        "resource",
        "create",
        "--type",
        "session",
        "--title",
        title,
        "--context",
        &context,
        "--idempotency-key",
        key,
    ];
    args.extend_from_slice(extra);
    common::run_temper_cli(app, &args)
        .await
        .expect("spawn the temper binary")
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON document ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

async fn create(app: &common::E2eTestApp, title: &str, key: &str, extra: &[&str]) -> Value {
    let output = run_create(app, title, key, extra).await;
    assert!(
        output.status.success(),
        "create failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}

async fn count_titled(pool: &sqlx::PgPool, title: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_resources WHERE title = $1 AND is_active")
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("count resources")
}

/// A markdown body over one segment budget, so the create takes the segmented path.
fn large_body() -> String {
    let budget = temper_ingest::stream::SEGMENT_BUDGET_BYTES;
    let mut body = String::from("# Large Keyed Body\n\n");
    let mut section = 0usize;
    while body.len() < budget + 64 * 1024 {
        body.push_str(&format!("## Section {section}\n\n"));
        for _ in 0..40 {
            body.push_str(
                "A filler sentence long enough to push this document past the segment budget.\n",
            );
        }
        body.push('\n');
        section += 1;
    }
    body
}

/// Write `content` to a file in the app's vault, returning the `--body @<path>` value.
fn body_file(app: &common::E2eTestApp, content: &str) -> String {
    let path = app.vault_dir.path().join("large-keyed-body.md");
    std::fs::write(&path, content).expect("write body file");
    format!("@{}", path.display())
}

/// Leave an unfinished resume manifest for `content` under `saved_key`: with no `resource_id`, as
/// an attempt interrupted before its begin was acknowledged leaves one; with one, as an attempt
/// interrupted after it leaves one.
fn pending_manifest(
    app: &common::E2eTestApp,
    content: &str,
    saved_key: Uuid,
    resource_id: Option<Uuid>,
) -> std::path::PathBuf {
    use temper_cli::actions::ingest_manifest::{self, IngestManifest, SEGMENTER_VERSION};

    let path = ingest_manifest::manifest_path(app.vault_dir.path(), saved_key);
    ingest_manifest::store(
        &path,
        &IngestManifest {
            idempotency_key: Some(saved_key),
            resource_id,
            source_hash: temper_core::hash::sha256_hex(content.as_bytes()),
            block_budget: temper_ingest::stream::SEGMENT_BUDGET_BYTES as u32,
            segmenter_version: SEGMENTER_VERSION,
            correlation_id: None,
            blocks: Vec::new(),
            finalized: false,
        },
    )
    .expect("store manifest");
    path
}

/// Land every segment of `content` under `key` through the server's own primitives, with zero
/// vectors, and finalize it when `finalize` is set: the state an earlier attempt with that key
/// left, without paying for a real embed. Valid because the resume diff and finalize compare
/// chunk and byte hashes, never vectors. Returns the resource's id.
async fn land_segments(
    app: &common::E2eTestApp,
    title: &str,
    content: &str,
    key: Uuid,
    finalize: bool,
) -> Uuid {
    use temper_core::types::ingest::{
        pack_chunks, AppendBlockPayload, FinalizePayload, IngestPayload, PackedChunk,
        SegmentedBegin,
    };

    let budget = temper_ingest::stream::SEGMENT_BUDGET_BYTES;
    let source_hash = temper_core::hash::sha256_hex(content.as_bytes());
    let segments: Vec<(String, Vec<temper_ingest::chunk::ChunkData>)> =
        temper_ingest::stream::segment_reader(std::io::Cursor::new(content.as_bytes()), budget)
            .map(|seg| {
                let seg = seg.expect("an in-memory segment read never fails");
                let chunked = temper_ingest::chunk::chunk_markdown_with_prefix(
                    &seg.text,
                    &seg.initial_breadcrumb,
                );
                (seg.text, chunked)
            })
            .collect();
    assert!(segments.len() >= 2, "the body crosses the segment budget");
    let zero_packed = |chunks: &[temper_ingest::chunk::ChunkData]| {
        let packed: Vec<PackedChunk> = chunks
            .iter()
            .map(|c| PackedChunk {
                chunk_index: c.chunk_index,
                header_path: c.header_path.clone(),
                heading_depth: c.heading_depth,
                content: c.content.clone(),
                content_hash: c.content_hash.clone(),
                embedding: vec![0.0_f32; 768],
                embedded_with: None,
            })
            .collect();
        pack_chunks(&packed).expect("pack_chunks")
    };

    let begin = app
        .client
        .ingest()
        .begin_segmented(&IngestPayload {
            idempotency_key: Some(key),
            segmented: Some(SegmentedBegin {
                total_blocks_hint: Some(segments.len() as u32),
                block_budget: budget as u32,
                source_hash: Some(source_hash.clone()),
            }),
            goal: None,
            title: title.to_string(),
            origin_uri: String::new(),
            context_ref: format!("@me/{CONTEXT}"),
            home_cogmap_id: None,
            doc_type_name: "session".to_string(),
            content_hash: None,
            content: segments[0].0.clone(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: Some(zero_packed(&segments[0].1)),
            act: Default::default(),
            sources: Vec::new(),
        })
        .await
        .expect("begin_segmented");
    let mut body_hash = begin.body_hash;
    for (seq, (text, chunked)) in segments.iter().enumerate().skip(1) {
        body_hash = app
            .client
            .ingest()
            .append_block(
                begin.resource_id,
                &AppendBlockPayload {
                    seq: seq as u32,
                    content: text.clone(),
                    content_hash: temper_core::hash::sha256_hex(text.as_bytes()),
                    chunks_packed: Some(zero_packed(chunked)),
                    sources: Vec::new(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("append seq {seq}: {e}"))
            .body_hash;
    }
    if finalize {
        app.client
            .ingest()
            .finalize(
                begin.resource_id,
                &FinalizePayload {
                    expected_blocks: segments.len() as u32,
                    expected_body_hash: body_hash,
                    expected_content_hash: Some(source_hash),
                },
            )
            .await
            .expect("finalize");
    }
    begin.resource_id
}

/// FAILS IF a create re-issued with its first key mints a second resource or answers with a
/// different resource than the first; or if two different keys are collapsed into one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_same_key_twice_is_one_resource_and_different_keys_are_two(pool: sqlx::PgPool) {
    let app = setup(pool.clone()).await;
    let title = "Idempotency Key Replay Probe";
    let key = Uuid::now_v7().to_string();

    let first = create(&app, title, &key, &[]).await;
    let replay = create(&app, title, &key, &[]).await;
    assert_eq!(
        replay["id"], first["id"],
        "a replayed key answers with the first resource"
    );
    for field in [
        "ref",
        "title",
        "doc_type_name",
        "context_ref",
        "body_hash",
        "created",
    ] {
        assert_eq!(replay[field], first[field], "{field}: {first} vs {replay}");
    }
    assert_eq!(count_titled(&pool, title).await, 1, "one resource");

    let other = create(&app, title, &Uuid::now_v7().to_string(), &[]).await;
    assert_ne!(
        other["id"], first["id"],
        "a different key is a different create"
    );
    assert_eq!(count_titled(&pool, title).await, 2, "two resources");
}

/// FAILS IF a large create given a key resumes an interrupted attempt saved under another key:
/// the given key would go unsent and unbound, so a later retry with it would create a second
/// resource. The refusal names the saved key and sends nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_given_key_that_conflicts_with_a_pending_resume_is_refused(pool: sqlx::PgPool) {
    let app = setup(pool.clone()).await;
    let title = "Idempotency Key Resume Conflict";
    let content = large_body();
    let saved = Uuid::now_v7();
    let manifest = pending_manifest(&app, &content, saved, None);
    let given = Uuid::now_v7().to_string();

    let body = body_file(&app, &content);
    let output = run_create(&app, title, &given, &["--body", &body]).await;
    assert!(!output.status.success(), "a conflicting key is refused");
    let refusal = stdout_json(&output);
    assert_eq!(refusal["code"], "bad-request", "{refusal}");
    let message = refusal["message"].as_str().expect("message");
    assert!(
        message.contains(&format!("--idempotency-key {saved}")),
        "{message}"
    );
    assert!(message.contains(&given), "{message}");
    assert_eq!(count_titled(&pool, title).await, 0, "nothing was created");
    assert!(manifest.is_file(), "the saved attempt is left to resume");
}

/// FAILS IF a large create with a minted key, resuming an attempt saved under another key, does
/// not converge on that key's resource, or leaves the hint naming the minted key the server never
/// saw instead of the saved one.
///
/// The interrupted attempt landed every segment (with zero vectors) and was cut off before its
/// finalize, so the resume embeds nothing: it reads the landed blocks and finalizes. Key pinning
/// and the hint are the whole of what this witnesses, and neither needs a real embed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_minted_key_gives_way_to_the_saved_key_and_the_hint_names_it(pool: sqlx::PgPool) {
    use temper_cli::reconcile_hint::LostAckProne;

    let app = setup(pool.clone()).await;
    let title = "Idempotency Key Minted Resume";
    let content = large_body();
    let saved = Uuid::now_v7();
    let landed = land_segments(&app, title, &content, saved, false).await;
    pending_manifest(&app, &content, saved, Some(landed));
    let minted = Uuid::now_v7();

    let body_flag = body_file(&app, &content);
    let api_url = app.base_url();
    let token = app.token.clone();
    let global_config = app
        .vault_dir
        .path()
        .join("no-such-config.toml")
        .display()
        .to_string();
    let cli_config = app.cli_config.clone();
    let context = format!("@me/{CONTEXT}");
    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url.as_str())),
                ("TEMPER_TOKEN", Some(token.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config.as_str())),
                ("TEMPER_AUTH_PATH", None),
            ],
            || {
                temper_cli::commands::resource::create(
                    &cli_config,
                    temper_cli::commands::resource::CreateResourceArgs {
                        doc_type: "session",
                        title,
                        context: Some(&context),
                        cogmap: None,
                        mode: None,
                        effort: None,
                        open_meta: None,
                        goal: None,
                        task: None,
                        body_flag: Some(body_flag),
                        from: None,
                        sources: Vec::new(),
                        sources_as_edges: false,
                        no_source: false,
                        preserve_source: false,
                        idempotency_key: Some(minted),
                        idempotency_key_given: false,
                        format: temper_cli::format::OutputFormat::Json,
                        act: Default::default(),
                    },
                )
            },
        )
    })
    .await
    .expect("spawn_blocking joined");
    result.expect("the minted-key create resumes the saved attempt");
    assert_eq!(count_titled(&pool, title).await, 1, "one resource");

    assert_eq!(
        LostAckProne::ResourceCreate {
            idempotency_key: Some(minted)
        }
        .as_dispatched(),
        LostAckProne::ResourceCreate {
            idempotency_key: Some(saved)
        },
        "the hint names the key the resume sent"
    );

    // The saved key is the one the server bound: re-issuing with it is the same resource.
    let replay = create(&app, title, &saved.to_string(), &[]).await;
    assert_eq!(count_titled(&pool, title).await, 1, "{replay}");
}

/// FAILS IF a large (segmented) create re-issued with its first key creates a second resource, or
/// a manifest naming a resource that has since finished sends the run appending into it.
///
/// The first write is landed through the primitives with zero vectors; the CLI's replay still
/// embeds its segment 0 before the begin it re-sends, so this one needs ONNX.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_keyed_large_create_reissued_is_one_resource(pool: sqlx::PgPool) {
    let app = setup(pool.clone()).await;
    let title = "Idempotency Key Large Replay";
    let content = large_body();
    let key = Uuid::now_v7().to_string();
    let body = body_file(&app, &content);

    let first_id = land_segments(&app, title, &content, key.parse().expect("uuid"), true).await;
    let replay = create(&app, title, &key, &["--body", &body]).await;
    let first = serde_json::json!({ "id": first_id.to_string() });
    assert_eq!(replay["id"], first["id"], "{first} vs {replay}");
    assert_eq!(count_titled(&pool, title).await, 1, "one resource");

    // A manifest whose last write was lost after the server finalized: it still reads pending and
    // names the finished resource, so the resume must return it rather than append.
    use temper_cli::actions::ingest_manifest::{self, IngestManifest, SEGMENTER_VERSION};
    let id: Uuid = first["id"].as_str().expect("id").parse().expect("uuid id");
    let key: Uuid = key.parse().expect("uuid key");
    let blocks = app
        .client
        .ingest()
        .list_blocks(id)
        .await
        .expect("list_blocks")
        .blocks;
    ingest_manifest::store(
        &ingest_manifest::manifest_path(app.vault_dir.path(), key),
        &IngestManifest {
            idempotency_key: Some(key),
            resource_id: Some(id),
            source_hash: temper_core::hash::sha256_hex(content.as_bytes()),
            block_budget: temper_ingest::stream::SEGMENT_BUDGET_BYTES as u32,
            segmenter_version: SEGMENTER_VERSION,
            correlation_id: None,
            blocks,
            finalized: false,
        },
    )
    .expect("store stale manifest");
    let resumed = create(&app, title, &key.to_string(), &["--body", &body]).await;
    assert_eq!(resumed["id"], first["id"], "{first} vs {resumed}");
    assert_eq!(count_titled(&pool, title).await, 1, "still one resource");
}
