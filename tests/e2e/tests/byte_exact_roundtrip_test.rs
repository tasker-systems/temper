#![cfg(all(feature = "test-db", feature = "test-embed"))]

//! W2 PR 4 — byte-exact readback.
//!
//! The payoff slice of the byte-fidelity arc (#420 set 3): a body uploaded through the **real CLI**
//! (the production caller — client-side chunk + embed, then one-shot `create_resource` or the
//! segmented `begin`/`append`/`finalize` path) must read back **byte-for-byte** —
//! `sha256(PUT) == sha256(GET)` — off the verbatim block store PR 3 laid down, and the resource must
//! report `body_storage = verbatim` so the guarantee is *surfaced*, not merely latent.
//!
//! This drives the CLI directly (`temper_cli::commands::resource::create`, the `spawn_blocking`
//! pattern `streaming_ingest_test.rs` / `cloud_writes_test.rs` use), NOT a hand-built `IngestPayload`
//! — a hand-built payload could carry raw bytes that never went through the real segmenter/chunker,
//! so it could not prove the production path is byte-exact. It asserts against the **API `content`
//! endpoint** (`readback::body`), never the projected vault file: `normalize_body_for_vault`
//! (`actions/ingest.rs`) prepends a `\n` for the frontmatter separator — a projection concern, not a
//! storage one.
//!
//! `test-embed`-gated in full: every body here rides the CLI's client-side embed path (ONNX), even
//! the tiny one-shot ones.

mod common;

use sqlx::PgPool;
use temper_core::hash::sha256_hex;
use temper_workflow::types::resource::BodyStorage;
use uuid::Uuid;

/// Shared env-var builder for cloud-mode CLI invocations — mirrors `streaming_ingest_test.rs` /
/// `cloud_writes_test.rs`'s `cloud_env` (each e2e test file is its own binary; there is no shared
/// home for this tiny helper besides `common`, which doesn't own CLI-env wiring).
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

/// Resolve a resource's id by title — most-recent-active wins, so each body under test uses a
/// distinct title. Mirrors `streaming_ingest_test.rs`'s helper.
async fn resource_id_for_title(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM kb_resources WHERE title = $1 AND is_active ORDER BY created DESC LIMIT 1",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("resource_id_for_title({title}): {e}"))
}

/// A body that spans a segment boundary (> `SEGMENT_BUDGET_BYTES`), so the CLI picks the segmented
/// (`begin`/`append`/`finalize`) path and the verbatim readback must `string_agg` across >1 block in
/// `seq` order to reconstruct the original bytes. Sized to the *minimum* that crosses one budget
/// (⇒ 2 segments) — enough to prove the cross-block concat is byte-exact, small enough to stay well
/// under nextest's cap (the harness embeds ~single-core; keep the chunk count modest).
fn large_multi_block_body() -> String {
    const FILLER: &str =
        "The quick brown fox jumps over the lazy dog, padding this section past the segmentation \
         budget so the streaming ingest pipeline splits the document across multiple blocks.\n";
    let target = temper_ingest::stream::SEGMENT_BUDGET_BYTES + 64 * 1024; // ~320 KiB ⇒ 2 segments
    let mut body = String::from("# Big Document\n\n");
    let mut section = 0usize;
    while body.len() < target {
        body.push_str(&format!("## Section {section}\n\n"));
        for _ in 0..40 {
            body.push_str(FILLER);
        }
        body.push('\n');
        section += 1;
    }
    body
}

/// Every body — tiny one-shot and large segmented alike — round-trips byte-for-byte and reports
/// `body_storage = verbatim`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn byte_exact_roundtrip(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("bytectx", None)
        .await
        .expect("create bytectx context");

    // Trailing-newline present/absent, CRLF throughout, non-ASCII, and a > budget segmented body.
    let bodies: Vec<String> = vec![
        "# T\n\nalpha\nbeta\n".to_string(),
        "# T\n\nalpha\nbeta".to_string(),
        "# T\r\n\r\nalpha\r\nbeta\r\n".to_string(),
        "# T\n\nnaïve — ünïcode ✅\n".to_string(),
        large_multi_block_body(),
    ];

    for (i, body) in bodies.iter().enumerate() {
        let title = format!("Byte {i}");
        let path = app.vault_dir.path().join(format!("body-{i}.md"));
        std::fs::write(&path, body).expect("write body file");
        let body_flag = format!("@{}", path.to_str().unwrap());

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
        let title_owned = title.clone();

        // Drive the real CLI create on a blocking thread (it builds its own tokio runtime and
        // `.block_on()`s synchronously — the pattern `cloud_writes_test.rs` uses throughout).
        tokio::task::spawn_blocking(move || {
            temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
                temper_cli::commands::resource::create(
                    &cli_config,
                    temper_cli::commands::resource::CreateResourceArgs {
                        open_meta: None,
                        goal: None,
                        doc_type: "research",
                        title: &title_owned,
                        context: Some("@me/bytectx"),
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
                .expect("cloud create should succeed")
            })
        })
        .await
        .expect("spawn_blocking joined");

        let id = resource_id_for_title(&pool, &title).await;

        // Byte-exact: the content endpoint returns exactly the bytes we PUT.
        let content = app
            .client
            .resources()
            .content(id)
            .await
            .expect("fetch content");
        assert_eq!(
            sha256_hex(content.markdown.as_bytes()),
            sha256_hex(body.as_bytes()),
            "body {i} ({title}) did not round-trip byte-exact:\n  put={body:?}\n  got={:?}",
            content.markdown,
        );

        // ...and the guarantee is surfaced: verbatim, not derived. Guards against an inert feature —
        // a byte-exact read that silently fell back to the derived reconstruction would still match
        // for these bodies, but would not carry the verbatim guarantee.
        let detail = app.client.resources().get(id).await.expect("show");
        assert_eq!(
            detail.row.body_storage,
            Some(BodyStorage::Verbatim),
            "body {i} ({title}) must report body_storage = verbatim",
        );
    }
}
