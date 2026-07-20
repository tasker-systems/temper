//! Segmented ingest driven the way an MCP caller drives it: through the production tool functions
//! (`TemperMcpService` → `ensure_profile_from_parts` → `tools::ingest::*`), with no client-side
//! chunks, no embedder, and no `.temper/` manifest.
//!
//! The load-bearing assertion is `segmented_server_chunked_ingest_equals_a_one_shot_create`: a
//! document ingested segment-by-segment must be indistinguishable from the same document created in
//! one shot. That single equivalence covers breadcrumb continuity across block boundaries, segment
//! reassembly, and merkle agreement at once.
//!
//! `test-embed` because the server chunks and embeds every segment (the caller cannot).
#![cfg(all(feature = "test-db", feature = "test-embed"))]

mod common;

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::{BlocksResponse, SegmentedBeginResponse};
use temper_mcp::service::TemperMcpService;
use temper_mcp::tools::ingest::{
    IngestAppendInput, IngestBeginInput, IngestBlocksInput, IngestFinalizeInput,
};
use temper_mcp::tools::resources::CreateResourceInput;

/// A document split at heading boundaries, the way a segmenting client cuts it.
///
/// Every assertion here compares a segmented body against a **one-shot body**, never against the
/// raw source text: `reconstruct_body` re-emits headings from chunk metadata and normalizes blank
/// lines, so reconstructed text is not byte-identical to its input. Comparing the two ingest paths
/// to each other is both the property we care about and robust to that normalization (and to the
/// known heading-duplication bug, temper task 019f4694, which affects both sides equally).
fn corpus() -> Vec<&'static str> {
    vec![
        "# Manual\n\nIntro line.\n\n## Setup\n\nInstall it.\n",
        "## Usage\n\nRun it.\n\n## Caveats\n\nMind the gap.\n",
        "## Appendix\n\nReferences follow.\n",
    ]
}

fn sha(text: &str) -> String {
    temper_core::hash::sha256_hex(text.as_bytes())
}

/// Build an MCP service whose profile cache is seeded from synthetic JWT claims — the production
/// caller path, mirroring `mcp_round_trip_test`.
async fn mcp_service(pool: &sqlx::PgPool) -> TemperMcpService {
    use temper_services::auth_config::{AuthConfig, AuthMode};
    use temper_services::config::ApiConfig;
    use temper_services::state::{AppState, JwksKeyStore};

    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: common::TEST_AUDIENCE.to_string(),
            mode: AuthMode::ExternalIdp,
        },
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
        embed_dispatch_secret: None,
        vercel_connect: None,
        slack_link: None,
        slack_mint_secret: None,
    };
    let svc = TemperMcpService::new(AppState::new(pool.clone(), jwks_store, api_config));

    let req = axum::http::Request::builder()
        // The MCP JWT middleware injects the raw bearer alongside the claims; the auth
        // seam needs it for the email ladder's /userinfo rung. Synthetic parts must
        // carry both or the service rejects the request as unwired.
        .extension(temper_mcp::middleware::BearerToken("synthetic".to_string()))
        .extension(temper_services::auth::RawJwtClaims {
            sub: "e2e-test-user".to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            iat: 0,
        })
        .body(())
        .expect("build request");
    let (req_parts, ()) = req.into_parts();
    svc.ensure_profile_from_parts(&req_parts)
        .await
        .expect("seed profile cache");
    svc
}

/// The tool functions return `CallToolResult`, whose payload is the JSON text of the typed
/// response. Parse it back so assertions speak in types, not strings. Taken as `impl Serialize`
/// rather than naming `rmcp::model::CallToolResult`, so this crate need not link rmcp.
fn parse_tool_json<T: serde::de::DeserializeOwned>(result: impl serde::Serialize) -> T {
    let value = serde_json::to_value(&result).expect("tool result serializes");
    let text = value["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool returned no text content part: {value}"))
        .to_owned();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse tool json: {e}\nbody: {text}"))
}

fn begin_input(context_ref: &str, title: &str, segment0: &str) -> IngestBeginInput {
    IngestBeginInput {
        create: CreateResourceInput {
            context_ref: Some(context_ref.to_string()),
            cogmap: None,
            doc_type_name: "research".to_string(),
            title: title.to_string(),
            content: Some(segment0.to_string()),
            sources: None,
            goal: None,
            origin_uri: Some(format!("mcp://test/{title}")),
            owner: None,
            managed_meta: None,
            open_meta: None,
            act: Default::default(),
        },
        content_hash: sha(segment0),
        block_budget: Some(1024),
        total_blocks_hint: Some(3),
        source_hash: None,
    }
}

async fn append(svc: &TemperMcpService, resource: &str, seq: u32, text: &str) -> BlocksResponse {
    append_with_sources(svc, resource, seq, text, None).await
}

/// Append one segment, optionally carrying per-block provenance sources — the `ingest_append`
/// sources path (issue #354).
async fn append_with_sources(
    svc: &TemperMcpService,
    resource: &str,
    seq: u32,
    text: &str,
    sources: Option<Vec<String>>,
) -> BlocksResponse {
    parse_tool_json(
        temper_mcp::tools::ingest::ingest_append(
            svc,
            IngestAppendInput {
                resource: resource.to_string(),
                seq,
                content: text.to_string(),
                content_hash: sha(text),
                sources,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("ingest_append seq {seq}: {e:?}")),
    )
}

async fn blocks(svc: &TemperMcpService, resource: &str) -> BlocksResponse {
    parse_tool_json(
        temper_mcp::tools::ingest::ingest_blocks(
            svc,
            IngestBlocksInput {
                resource: resource.to_string(),
            },
        )
        .await
        .expect("ingest_blocks"),
    )
}

async fn finalize(svc: &TemperMcpService, resource: &str, expected_blocks: u32, body_hash: &str) {
    temper_mcp::tools::ingest::ingest_finalize(
        svc,
        IngestFinalizeInput {
            resource: resource.to_string(),
            expected_blocks,
            expected_body_hash: body_hash.to_string(),
        },
    )
    .await
    .expect("ingest_finalize");
}

/// Reassembled body text, through the production read selector (`readback::body` under the hood) —
/// the same path `get_resource --include-content` serves.
async fn body_text(pool: &sqlx::PgPool, resource: uuid::Uuid, profile: uuid::Uuid) -> String {
    temper_services::backend::substrate_read::get_content_select(
        pool,
        ProfileId::from(profile),
        ResourceId::from(resource),
    )
    .await
    .expect("get_content_select")
    .markdown
}

/// Every chunk's `(header_path, heading_depth, content_hash)` in body order — the semantic content
/// of the resource, independent of how it was blocked up for transport.
async fn chunk_shape(
    pool: &sqlx::PgPool,
    resource: uuid::Uuid,
) -> Vec<(Option<String>, Option<i16>, String)> {
    sqlx::query_as::<_, (Option<String>, Option<i16>, String)>(
        "SELECT c.header_path, c.heading_depth, c.content_hash FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
          WHERE c.resource_id = $1 AND c.is_current AND NOT b.is_folded \
          ORDER BY b.seq, c.chunk_index",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("chunk shape")
}

async fn stored_body_hash(pool: &sqlx::PgPool, resource: uuid::Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT body_hash FROM kb_resources WHERE id = $1")
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("body_hash")
}

/// Run a full segmented session over the corpus, returning the resource id.
async fn ingest_segmented(svc: &TemperMcpService, context_ref: &str, title: &str) -> uuid::Uuid {
    let segments = corpus();
    let begin: SegmentedBeginResponse = parse_tool_json(
        temper_mcp::tools::ingest::ingest_begin(svc, begin_input(context_ref, title, segments[0]))
            .await
            .expect("ingest_begin"),
    );
    let resource = begin.resource_id.to_string();

    let mut body_hash = begin.body_hash;
    for (i, segment) in segments.iter().enumerate().skip(1) {
        body_hash = append(svc, &resource, i as u32, segment).await.body_hash;
    }
    finalize(svc, &resource, segments.len() as u32, &body_hash).await;
    begin.resource_id
}

// ── The load-bearing assertion ─────────────────────────────────────────────────

/// One-shot `create_resource` of the whole document, through the same MCP surface — an
/// independent reference for the equivalence assertion. The server chunks it in a single pass.
async fn one_shot_create(svc: &TemperMcpService, context_ref: &str, body: &str) -> uuid::Uuid {
    let result = temper_mcp::tools::resources::create_resource(
        svc,
        CreateResourceInput {
            context_ref: Some(context_ref.to_string()),
            cogmap: None,
            doc_type_name: "research".to_string(),
            title: "Reference".to_string(),
            content: Some(body.to_string()),
            sources: None,
            goal: None,
            origin_uri: Some("mcp://test/reference".to_string()),
            owner: None,
            managed_meta: None,
            open_meta: None,
            act: Default::default(),
        },
    )
    .await
    .expect("one-shot create_resource");

    let json: serde_json::Value = parse_tool_json(result);
    json["resource"]["id"]
        .as_str()
        .expect("create response carries the resource id")
        .parse()
        .expect("resource id is a uuid")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn segmented_server_chunked_ingest_equals_a_one_shot_create(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("profile").id;
    app.client
        .contexts()
        .create("mcp-segmented", None)
        .await
        .expect("context create");

    let svc = mcp_service(&pool).await;

    // The reference: one call, whole document, server chunks it in one pass.
    let reference = one_shot_create(&svc, "@me/mcp-segmented", &corpus().concat()).await;

    // The subject: begin + N appends + finalize, server chunks each segment independently and
    // carries the heading breadcrumb across every block boundary.
    let segmented = ingest_segmented(&svc, "@me/mcp-segmented", "Segmented").await;

    assert_eq!(
        body_text(&pool, segmented, profile).await,
        body_text(&pool, reference, profile).await,
        "a segmented body must reassemble to the one-shot body"
    );
    assert_eq!(
        chunk_shape(&pool, segmented).await,
        chunk_shape(&pool, reference).await,
        "same chunks, same breadcrumbs, same content hashes — breadcrumbs must stay continuous \
         across block boundaries, and segment-wise chunking must agree with whole-document chunking"
    );

    // `body_hash` is a merkle over PER-BLOCK hashes ordered by seq (`_recompute_resource_body_hash`),
    // so block structure is part of it by construction. A 3-block segmented resource and a 1-block
    // one-shot resource with identical chunks therefore have different body_hashes — by design, not
    // by accident. Pinned so nobody "fixes" it into an equality later.
    assert_ne!(
        stored_body_hash(&pool, segmented).await,
        stored_body_hash(&pool, reference).await,
        "body_hash folds in block structure; identical content in different block counts differs"
    );
}

// ── Resume ─────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_interrupted_segmented_ingest_resumes_from_the_server_alone(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("profile").id;
    app.client
        .contexts()
        .create("mcp-resume", None)
        .await
        .expect("context create");

    let svc = mcp_service(&pool).await;
    let segments = corpus();

    let begin: SegmentedBeginResponse = parse_tool_json(
        temper_mcp::tools::ingest::ingest_begin(
            &svc,
            begin_input("@me/mcp-resume", "Interrupted", segments[0]),
        )
        .await
        .expect("ingest_begin"),
    );
    let resource = begin.resource_id.to_string();
    append(&svc, &resource, 1, segments[1]).await;
    // Segment 2 never lands — the process died here.

    // Resume with no local manifest: ask the server what it has.
    let landed = blocks(&svc, &resource).await;
    let have: Vec<u32> = landed.blocks.iter().map(|b| b.seq).collect();
    assert_eq!(have, vec![0, 1], "the server knows exactly what landed");

    let missing: Vec<usize> = (0..segments.len())
        .filter(|i| !have.contains(&(*i as u32)))
        .collect();
    assert_eq!(missing, vec![2]);

    let mut body_hash = landed.body_hash;
    for i in missing {
        body_hash = append(&svc, &resource, i as u32, segments[i])
            .await
            .body_hash;
    }
    finalize(&svc, &resource, segments.len() as u32, &body_hash).await;

    // A resumed session must be indistinguishable from one that never broke. The reference is an
    // uninterrupted SEGMENTED ingest — same block structure, so the merkle is comparable too.
    let reference = ingest_segmented(&svc, "@me/mcp-resume", "Uninterrupted").await;
    assert_eq!(
        body_text(&pool, begin.resource_id, profile).await,
        body_text(&pool, reference, profile).await,
        "a resumed body equals a body ingested without interruption"
    );
    assert_eq!(
        stored_body_hash(&pool, begin.resource_id).await,
        stored_body_hash(&pool, reference).await,
        "and agrees with it on the merkle — resume re-sends only the gap, changing nothing else"
    );
}

// ── Idempotent replay ──────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn re_appending_a_landed_segment_is_an_idempotent_no_op(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile");
    app.client
        .contexts()
        .create("mcp-idempotent", None)
        .await
        .expect("context create");

    let svc = mcp_service(&pool).await;
    let segments = corpus();

    let begin: SegmentedBeginResponse = parse_tool_json(
        temper_mcp::tools::ingest::ingest_begin(
            &svc,
            begin_input("@me/mcp-idempotent", "Idempotent", segments[0]),
        )
        .await
        .expect("ingest_begin"),
    );
    let resource = begin.resource_id.to_string();

    let first = append(&svc, &resource, 1, segments[1]).await;
    let again = append(&svc, &resource, 1, segments[1]).await;

    assert_eq!(
        first.blocks.len(),
        again.blocks.len(),
        "a replayed append lands no duplicate block"
    );
    assert_eq!(
        first.body_hash, again.body_hash,
        "body_hash is unchanged by a replay — which is what makes retry safe"
    );
}

// ── Integrity ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_append_whose_content_does_not_hash_to_its_declared_hash_is_rejected(
    pool: sqlx::PgPool,
) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile");
    app.client
        .contexts()
        .create("mcp-badhash", None)
        .await
        .expect("context create");

    let svc = mcp_service(&pool).await;
    let segments = corpus();
    let begin: SegmentedBeginResponse = parse_tool_json(
        temper_mcp::tools::ingest::ingest_begin(
            &svc,
            begin_input("@me/mcp-badhash", "BadHash", segments[0]),
        )
        .await
        .expect("ingest_begin"),
    );

    let err = temper_mcp::tools::ingest::ingest_append(
        &svc,
        IngestAppendInput {
            resource: begin.resource_id.to_string(),
            seq: 1,
            content: segments[1].to_string(),
            content_hash: "deadbeef".to_string(),
            sources: None,
        },
    )
    .await
    .expect_err("a mismatched content_hash must be rejected");

    assert!(
        format!("{err:?}").contains("content_hash"),
        "the error must name the offending field: {err:?}"
    );

    let landed = blocks(&svc, &begin.resource_id.to_string()).await;
    assert_eq!(landed.blocks.len(), 1, "a rejected append lands nothing");
}

// ── Per-block provenance (issue #354) ────────────────────────────────────────────

/// The itemized per-block provenance for a resource, read the service-direct way (the same path the
/// HTTP `/provenance` endpoint and the CLI `--provenance` view use).
async fn provenance(
    pool: &sqlx::PgPool,
    profile: uuid::Uuid,
    resource: uuid::Uuid,
) -> Vec<temper_core::types::provenance::BlockProvenanceRow> {
    temper_services::backend::substrate_read::resource_block_provenance_select(
        pool,
        ProfileId::from(profile),
        resource,
    )
    .await
    .expect("resource_block_provenance_select")
}

/// The acceptance criterion for issue #354: a document ingested `begin → N × append(sources=[…]) →
/// finalize` records per-block provenance, block-aligned. Each appended segment's source lands on
/// *that* block (seq), and the un-attributed begin block (seq 0) records nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn appended_segments_record_block_aligned_provenance(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("profile").id;
    app.client
        .contexts()
        .create("mcp-prov", None)
        .await
        .expect("context create");

    let svc = mcp_service(&pool).await;
    let segments = corpus();

    // Begin carries no sources (block 0 is un-attributed).
    let begin: SegmentedBeginResponse = parse_tool_json(
        temper_mcp::tools::ingest::ingest_begin(
            &svc,
            begin_input("@me/mcp-prov", "Attributed", segments[0]),
        )
        .await
        .expect("ingest_begin"),
    );
    let resource = begin.resource_id.to_string();

    // Two appends, each attributing its own segment to a distinct external source. Mixed-case host
    // is preserved raw in `source_uri` (normalized only in the server-side dedup key).
    let src_one = "https://Example.com/issue/1";
    let src_two = "https://example.com/PR/2";
    append_with_sources(
        &svc,
        &resource,
        1,
        segments[1],
        Some(vec![src_one.to_string()]),
    )
    .await;
    // finalize echoes the body_hash after the *last* append.
    let body_hash = append_with_sources(
        &svc,
        &resource,
        2,
        segments[2],
        Some(vec![src_two.to_string()]),
    )
    .await
    .body_hash;
    finalize(&svc, &resource, segments.len() as u32, &body_hash).await;

    let rows = provenance(&pool, profile, begin.resource_id).await;

    // One row per attributed append, none for the un-attributed begin block.
    assert_eq!(
        rows.len(),
        2,
        "one provenance row per attributed append, zero for the begin block; got {rows:?}"
    );
    assert!(
        rows.iter().all(|r| r.block_seq != 0),
        "block 0 (the un-attributed begin) records no provenance; got {rows:?}"
    );

    // Block alignment: each source is on its own block, in append order.
    let by_seq = |seq: i32| {
        rows.iter()
            .find(|r| r.block_seq == seq)
            .unwrap_or_else(|| panic!("no provenance row for block_seq {seq}; got {rows:?}"))
    };
    let b1 = by_seq(1);
    assert_eq!(b1.source_kind, "remote");
    assert_eq!(b1.source_uri.as_deref(), Some(src_one));
    assert_eq!(b1.accretion_seq, 0, "single source per block sits at seq 0");

    let b2 = by_seq(2);
    assert_eq!(b2.source_kind, "remote");
    assert_eq!(b2.source_uri.as_deref(), Some(src_two));

    // The two sources landed on distinct blocks — the whole point of per-block (not per-resource)
    // attribution.
    assert_ne!(
        b1.block_id, b2.block_id,
        "each append's source is recorded against its own content block"
    );
}
