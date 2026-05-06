#![cfg(feature = "test-db")]

//! Regression tests for `temper_cli::actions::show_cache` tier-3 behavior.
//!
//! The original bug (task
//! `2026-05-03-resource-update-via-cli-strips-yaml-frontmatter-and-glues-h1-to-next-heading`):
//! tier-3 wrote `content.markdown` directly to the live vault file, dropping
//! the entire YAML frontmatter block. Subsequent reads — including
//! `Frontmatter::parse_file` from `resource update` — failed with
//! "missing frontmatter block: file must begin with '---'".
//!
//! Both tests below exercise the real tier-3 path against a real Postgres +
//! Axum server and confirm the on-disk file ends with full frontmatter +
//! body.

mod common;

use std::time::{Duration, SystemTime};

use filetime::{set_file_mtime, FileTime};
use temper_cli::actions::show_cache::{self, FreshnessTier, ShowCacheParams};
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// Tier-3 healing path: when the local file has no parseable frontmatter
/// (the corruption mode reported in the bug ticket), `attempt_remote` must
/// rebuild the full file from the server response — `---` fences, all
/// `temper-*` keys, and a body separated from the closing fence by a blank
/// line. Pre-fix this path would write `content.markdown` only and leave the
/// vault file unparseable.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn tier3_rebuilds_full_frontmatter_when_local_file_is_corrupted(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("show-heal")
        .await
        .expect("context create");

    let body = "# Heal Me\n\n## Section\n\nbody text\n".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "h"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Heal Me".to_string(),
        origin_uri: "test://show-heal".to_string(),
        context_name: "show-heal".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "heal-me".to_string(),
        content: body.clone(),
        metadata: None,
        // ManagedMeta uses serde(rename = "temper-*") on its typed fields;
        // the ingest payload's managed_meta JSON must use the canonical
        // `temper-stage` key so it deserializes into ManagedMeta::stage on
        // the way back out (and not into the `extra` passthrough bucket).
        managed_meta: Some(serde_json::json!({"temper-stage": "draft"})),
        open_meta: Some(serde_json::json!({"tags": ["regression"]})),
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    // Stage a CORRUPTED local file at the path tier-3 will write back to.
    let local_path = app.vault_dir.path().join("heal-me.md");
    std::fs::write(&local_path, "# Heal Me## Section\n\nbody text\n").expect("write corrupted");
    let stale = FileTime::from_system_time(SystemTime::now() - Duration::from_secs(120));
    set_file_mtime(&local_path, stale).expect("set stale mtime");

    let result = show_cache::fetch(ShowCacheParams {
        client: &app.client,
        resource_id: seeded.id,
        local_path: &local_path,
        debounce: Duration::from_secs(30),
    })
    .await
    .expect("fetch");

    assert_eq!(
        result.source,
        FreshnessTier::FullFetch,
        "must hit tier-3 when local frontmatter is unparseable"
    );

    let on_disk = std::fs::read_to_string(&local_path).expect("read after fetch");
    assert!(
        on_disk.starts_with("---\n"),
        "tier-3 must write frontmatter fence; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("\n---\n"),
        "tier-3 must close the frontmatter fence; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-id:"),
        "rebuilt frontmatter must include temper-id; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-context: show-heal"),
        "rebuilt frontmatter must include the resource's context; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-type: research"),
        "rebuilt frontmatter must include the resource's doc_type; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("temper-stage: draft"),
        "rebuilt frontmatter must preserve managed_meta from server; got:\n{on_disk}"
    );
    assert!(
        on_disk.contains("- regression"),
        "rebuilt frontmatter must preserve open_meta tags from server; got:\n{on_disk}"
    );
}

/// Tier-3 hash-mismatch path: a freshly-created local file whose
/// `temper-updated` doesn't byte-match the server's `updated` (the common
/// case after `temper resource create` because of client/server clock skew
/// and timestamp rounding) used to be silently truncated to body-only by
/// the next `temper resource show`. Now it must round-trip through tier-3
/// reconstruction — the resulting file must still be a parseable vault
/// document, not a body-only payload.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn tier3_preserves_frontmatter_when_local_temper_updated_diverges(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("show-mismatch")
        .await
        .expect("context create");

    let body = "# Mismatch\n\nbody\n".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "m"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: "Mismatch".to_string(),
        origin_uri: "test://show-mismatch".to_string(),
        context_name: "show-mismatch".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: "mismatch".to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    // Stage a well-formed local file whose temper-updated is in the past
    // (i.e. doesn't match the server's `updated`). Tier-2 will compare and
    // fall through to tier-3.
    let local_path = app.vault_dir.path().join("mismatch.md");
    let local_body = "---\n\
        temper-id: 00000000-0000-0000-0000-000000000999\n\
        temper-type: research\n\
        temper-context: show-mismatch\n\
        temper-updated: 2020-01-01T00:00:00+00:00\n\
        title: Mismatch\n\
        ---\n\n# Mismatch\n\nbody\n";
    std::fs::write(&local_path, local_body).expect("write local");
    let stale = FileTime::from_system_time(SystemTime::now() - Duration::from_secs(120));
    set_file_mtime(&local_path, stale).expect("set stale mtime");

    let result = show_cache::fetch(ShowCacheParams {
        client: &app.client,
        resource_id: seeded.id,
        local_path: &local_path,
        debounce: Duration::from_secs(30),
    })
    .await
    .expect("fetch");

    assert_eq!(
        result.source,
        FreshnessTier::FullFetch,
        "must hit tier-3 when local temper-updated diverges from server"
    );

    let on_disk = std::fs::read_to_string(&local_path).expect("read after fetch");
    assert!(
        on_disk.starts_with("---\n"),
        "tier-3 must NOT strip the frontmatter fence; got:\n{on_disk}"
    );
    assert!(
        temper_core::frontmatter::Frontmatter::try_from(on_disk.as_str()).is_ok(),
        "tier-3 output must be parseable as a vault document; got:\n{on_disk}"
    );
}

/// Phase 5 acceptance gate: the precondition for show-cache tier-2.
///
/// The local CLI computes a canonical-form `managed_hash` over a JSONB that
/// includes `temper-title` / `temper-slug` keys. The server-stored
/// `managed_hash` (in `kb_resource_manifests.managed_hash`) MUST be
/// byte-identical to that local-computed hash for any newly-ingested
/// resource. Without this invariant, tier-2 (the hash-equality short-circuit
/// in `show_cache::attempt_remote`) cannot ever fire — it would always fall
/// through to tier-3, which is the exact bug Phase 5 closes.
///
/// Phase 8 re-enables the tier-2 short-circuit in show_cache; this test is
/// the gate that proves the precondition holds end-to-end against a real
/// Axum + Postgres + temper-client round-trip.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn phase5_local_canonical_hash_matches_server_managed_hash(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("hash-invariant")
        .await
        .expect("context create");

    // Build the canonical managed_meta locally, the way the CLI / MCP
    // send-side wiring does after Phase 5: start with user fields, run the
    // shared helper to inject the canonical identity keys, compute the
    // canonical-form hash. This is the value tier-2 will compare against.
    let title = "Hash Invariant";
    let slug = "hash-invariant";
    let mut canonical_managed_meta = serde_json::json!({"temper-stage": "draft"});
    temper_core::operations::ensure_managed_identity_keys(
        &mut canonical_managed_meta,
        title,
        Some(slug),
    );
    let local_canonical_hash =
        temper_core::hash::compute_managed_hash("research", &canonical_managed_meta);

    let body = "# Hash Invariant\n\nbody\n".to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: format!("{:0>64}", "i"),
        embedding: vec![0.0_f32; 768],
    };
    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: "test://hash-invariant".to_string(),
        context_name: "hash-invariant".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug: slug.to_string(),
        content: body.clone(),
        metadata: None,
        managed_meta: Some(canonical_managed_meta.clone()),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    let seeded = app.client.ingest().create(&payload).await.expect("ingest");

    // Fetch the manifest meta tier — the same path tier-2 will use.
    let meta = app
        .client
        .resources()
        .get_meta(*seeded.id)
        .await
        .expect("get_meta");

    assert_eq!(
        meta.managed_hash, local_canonical_hash,
        "server-stored managed_hash must equal client-side canonical hash; \
         this is the precondition for show-cache tier-2 (Phase 8). \
         server={}, local={}, server_managed_meta={:?}",
        meta.managed_hash, local_canonical_hash, meta.managed_meta
    );
}
