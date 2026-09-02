#![cfg(feature = "test-db")]
//! The `--preserve-source` CLI hook, end to end (S6 ledger: "E2E tier exercising
//! `--preserve-source` end-to-end"). The service-layer and HTTP-layer pieces were witnessed
//! at S2–S4; what was never exercised is the whole CLI-native chain in one pass:
//!
//! ```text
//! file bytes ──CLI──▶ multipart commit ──API──▶ blob store
//!                    text extraction ──API──▶ resource body
//!                    derivation_source edge (resource → blob)
//!                    blob get ──API──▶ streamed bytes ──▶ file
//! ```
//!
//! Byte integrity is the point: the file the CLI reads and the file `blob get --out` writes
//! must be the SAME bytes — the wire-level proof the S2–S5 witnesses carried piecemeal. The
//! store is the in-process [`temper_substrate::blob_store::InMemoryBlobStore`] (via
//! `common::setup_with_blob_store`); nothing reaches a real provider.
//!
//! The source file is the committed text-layer PDF fixture (`temper-ingest/tests/fixtures/
//! simple.pdf`): its extension is one of the D9 allowlist's six types (so the preserve hook's
//! extension-guessed media type is admissible — `.md`/`.txt` would refuse client-side), and
//! its text extraction is proven verbatim by the extractor's own test, so the body assertion
//! below rides known content.

mod common;

use std::path::Path;

use common::run_temper_cli;

/// The committed text-layer PDF, copied to a `.pdf` temp path (extension intact — the
/// preserve hook guesses the media type from it).
fn source_pdf() -> tempfile::NamedTempFile {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/temper-ingest/tests/fixtures/simple.pdf");
    let bytes = std::fs::read(&fixture).expect("read the committed PDF fixture");
    let f = tempfile::NamedTempFile::with_suffix(".pdf").expect("temp pdf");
    std::fs::write(f.path(), &bytes).expect("write temp pdf");
    f
}

/// Run the CLI, assert success, parse stdout as exactly one JSON document.
async fn cli_json(app: &common::E2eTestApp, args: &[&str]) -> serde_json::Value {
    let output = run_temper_cli(app, args).await.expect("cli run");
    assert!(
        output.status.success(),
        "cli {args:?} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "cli {args:?} did not emit exactly one JSON document ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn preserve_source_commits_exact_bytes_and_asserts_the_derivation_edge(pool: sqlx::PgPool) {
    let app = common::setup_with_blob_store(pool).await;

    // A real context, so the create home and the blob home are the same readable anchor.
    let ctx = app
        .client
        .contexts()
        .create("preserve-source-e2e", None)
        .await
        .expect("context create failed");
    let ctx_id = ctx.id.to_string();

    let src = source_pdf();
    let src_bytes = std::fs::read(src.path()).expect("read source bytes");
    let src_arg = src.path().display().to_string();

    // The whole hook in one command: create (body = extracted text) → blob commit →
    // derivation_source edge.
    let created = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "research",
            "--title",
            "Preserve Source E2E",
            "--from",
            &src_arg,
            "--preserve-source",
            "--context",
            &ctx_id,
            "--format",
            "json",
        ],
    )
    .await;

    let resource_id = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("create response carries the generic id: {created}"))
        .to_string();
    let blob_id = created["preserved_source"]["blob_id"]
        .as_str()
        .unwrap_or_else(|| panic!("create response carries preserved_source.blob_id: {created}"))
        .to_string();
    let edge_handle = created["preserved_source"]["edge_handle"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("create response carries preserved_source.edge_handle: {created}")
        })
        .to_string();

    // The body is the EXTRACTION, not the bytes: the PDF's text rode the create.
    let shown = cli_json(
        &app,
        &["resource", "show", &resource_id, "--format", "json"],
    )
    .await;
    let content = shown["content"]
        .as_str()
        .unwrap_or_else(|| panic!("show carries the body: {shown}"));
    assert!(
        content.contains("Temper Cloud Architecture"),
        "the body is the PDF's extracted text: {content}"
    );

    // Byte integrity: the blob's bytes come back EXACTLY as committed — streamed through
    // the API's read-through, written by the CLI.
    let out = tempfile::NamedTempFile::with_suffix(".pdf").expect("temp out");
    let out_arg = out.path().display().to_string();
    let get = run_temper_cli(&app, &["blob", "get", &blob_id, "--out", &out_arg])
        .await
        .expect("blob get run");
    assert!(
        get.status.success(),
        "blob get failed: stderr={}",
        String::from_utf8_lossy(&get.stderr)
    );
    let roundtripped = std::fs::read(out.path()).expect("read the blob-get output");
    assert_eq!(
        roundtripped, src_bytes,
        "blob get must return the source file's exact bytes"
    );

    // The derivation edge: resource → blob, `derivation_source`, express/forward — and the
    // ack's edge handle IS the edge the relations read returns.
    let relations = app
        .client
        .blobs()
        .relations(blob_id.parse().expect("blob id parses"))
        .await
        .expect("blob relations read");
    assert_eq!(
        relations.len(),
        1,
        "exactly one relation on the preserved blob: {relations:?}"
    );
    let rel = &relations[0];
    assert_eq!(rel.peer_table, "kb_resources", "peer table: {rel:?}");
    assert_eq!(
        rel.peer_id.to_string(),
        resource_id,
        "the edge points at the created resource"
    );
    assert_eq!(
        rel.label.as_deref(),
        Some("derivation_source"),
        "the derivation-source label: {rel:?}"
    );
    assert_eq!(
        rel.edge_id.to_string(),
        edge_handle,
        "the ack's edge handle is the edge the read returns"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn preserve_source_refuses_a_url_from_before_any_create(pool: sqlx::PgPool) {
    let app = common::setup_with_blob_store(pool).await;

    let ctx = app
        .client
        .contexts()
        .create("preserve-source-url-refusal", None)
        .await
        .expect("context create failed");
    let ctx_id = ctx.id.to_string();

    let title = "Preserve Source URL Refusal";
    let output = run_temper_cli(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "research",
            "--title",
            title,
            "--from",
            "https://example.com/source.pdf",
            "--preserve-source",
            "--context",
            &ctx_id,
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(
        !output.status.success(),
        "a URL --from with --preserve-source must fail fast"
    );
    // In --format json mode the CLI renders the refusal as an ErrorPayload on STDOUT (the
    // agent-parseable stream), never stderr — so the remedy is asserted there.
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "the refusal renders as one JSON ErrorPayload ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let message = payload["message"]
        .as_str()
        .unwrap_or_else(|| panic!("error payload carries message: {payload}"));
    assert!(
        message.contains("preserves a local file's original bytes"),
        "the refusal names the remedy: {message}"
    );

    // Fail-fast means BEFORE the create: no resource row, no half-made anything.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE title = $1")
        .bind(title)
        .fetch_one(&app.pool)
        .await
        .expect("count resources");
    assert_eq!(count, 0, "the refusal fires before any create");
}
