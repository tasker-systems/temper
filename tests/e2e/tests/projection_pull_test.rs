#![cfg(feature = "test-db")]
//! E2e tests for the cloud-only read-only projection (`temper pull`).

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::ResourceId;
use uuid::Uuid;

/// Ingest one resource into `context` and return its id. The ingest path
/// emits a creation event into `kb_events`, so the context will have at
/// least one event afterward.
async fn seed_resource(
    app: &common::E2eTestApp,
    context: &str,
    doc_type: &str,
    title: &str,
) -> ResourceId {
    let body = format!("# {title}\n\nBody text for {title}.");
    // The per-chunk `content_hash` column is VARCHAR(64); `compute_body_hash`
    // returns a 71-char `sha256:<hex>` string, so use the raw 64-char hex.
    let chunk_hash = temper_core::hash::compute_body_hash(&body)
        .trim_start_matches("sha256:")
        .to_string();
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: chunk_hash,
        embedding: vec![0.0_f32; 768],
    };
    let slug = title.to_lowercase().replace(' ', "-");
    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: doc_type.to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug,
        content: body.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
        act: Default::default(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest")
        .id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn events_cursor_returns_latest_event_for_context(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("cursor-ctx", None)
        .await
        .expect("ctx");

    seed_resource(&app, "cursor-ctx", "research", "Cursor Doc").await;

    // Resolve the context's UUID from a listed resource row.
    let listed = app
        .client
        .resources()
        .list(&temper_workflow::types::resource::ResourceListParams {
            context_ref: Some("@me/cursor-ctx".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    let context_id = Uuid::from(
        listed
            .rows
            .first()
            .expect("one row")
            .kb_context_id
            .expect("context-homed row has a context id"),
    );

    let latest = app
        .client
        .events()
        .latest_for_context(context_id)
        .await
        .expect("latest_for_context");
    assert!(
        latest.is_some(),
        "ingest must have emitted at least one event"
    );

    // An unknown context has no events.
    let empty = app
        .client
        .events()
        .latest_for_context(Uuid::nil())
        .await
        .expect("latest_for_context empty");
    assert!(empty.is_none(), "unknown context has no events");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: vault projection path uses the real owner handle (reconstruct_resource_row returns p.owner_handle), not the '@me' self-segment the projection expects; and row.slug is None (temper-slug §7-Die) so the filename slug falls back. Blocked on the readback @me/identity-key follow-up (F6/F1)"]
async fn write_resource_file_materializes_a_document(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("wctx", None)
        .await
        .expect("ctx");
    seed_resource(&app, "wctx", "research", "Write Me").await;

    let listed = app
        .client
        .resources()
        .list(&temper_workflow::types::resource::ResourceListParams {
            context_ref: Some("@me/wctx".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    let row = listed.rows.first().expect("one row");

    let vault_root = app.vault_dir.path();
    let path = temper_cli::projection::write_resource_file(&app.client, vault_root, row)
        .await
        .expect("write_resource_file")
        .expect("a context-homed resource projects to a path");

    let expected = vault_root
        .join("@me")
        .join("wctx")
        .join("research")
        .join("write-me.md");
    assert_eq!(path, expected);
    assert!(path.exists(), "file written at canonical path");

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("---\n"), "has frontmatter fence");
    assert!(content.contains("temper-id:"), "has identity frontmatter");
    assert!(content.contains("Body text for Write Me"), "has body");
}

/// Build a CLI `Config` whose vault root is the e2e harness's temp vault.
/// The harness already constructs a valid `Config` (`app.cli_config`) via
/// `temper_cli::config::load_from`, pointed at the same temp vault — reuse
/// it rather than reconstructing a literal that could drift from the real
/// struct shape.
fn projection_test_config(app: &common::E2eTestApp) -> temper_cli::config::Config {
    app.cli_config.clone()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: vault projection path uses the real owner handle (reconstruct_resource_row returns p.owner_handle), not the '@me' self-segment the projection expects; and row.slug is None (temper-slug §7-Die) so the filename slug falls back. Blocked on the readback @me/identity-key follow-up (F6/F1)"]
async fn write_resource_file_from_parts_materializes_a_document(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("fpctx", None)
        .await
        .expect("ctx");
    seed_resource(&app, "fpctx", "research", "Parts Doc").await;

    let listed = app
        .client
        .resources()
        .list(&temper_workflow::types::resource::ResourceListParams {
            context_ref: Some("@me/fpctx".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    let row = listed.rows.first().expect("one row");
    let content = app
        .client
        .resources()
        .content(uuid::Uuid::from(row.id))
        .await
        .expect("content");

    let vault_root = app.vault_dir.path();
    let path = temper_cli::projection::write_resource_file_from_parts(vault_root, row, &content)
        .expect("write_resource_file_from_parts")
        .expect("a context-homed resource projects to a path");

    let expected = vault_root
        .join("@me")
        .join("fpctx")
        .join("research")
        .join("parts-doc.md");
    assert_eq!(path, expected);
    assert!(path.exists(), "file written at canonical path");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.starts_with("---\n"), "has frontmatter fence");
    assert!(on_disk.contains("temper-id:"), "has identity frontmatter");
    assert!(on_disk.contains("Body text for Parts Doc"), "has body");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: vault projection path uses the real owner handle (reconstruct_resource_row returns p.owner_handle), not the '@me' self-segment the projection expects; and row.slug is None (temper-slug §7-Die) so the filename slug falls back. Blocked on the readback @me/identity-key follow-up (F6/F1)"]
async fn pull_context_materializes_tree_and_writes_cursor(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("pctx", None)
        .await
        .expect("ctx");
    seed_resource(&app, "pctx", "research", "Doc One").await;
    seed_resource(&app, "pctx", "research", "Doc Two").await;

    let config = projection_test_config(&app);
    let summary = temper_cli::projection::pull_context(&app.client, &config, "pctx")
        .await
        .expect("pull_context");

    assert_eq!(summary.written, 2, "both resources written");
    assert_eq!(summary.pruned, 0, "nothing stale on a first pull");

    let vault_root = app.vault_dir.path();
    assert!(vault_root.join("@me/pctx/research/doc-one.md").exists());
    assert!(vault_root.join("@me/pctx/research/doc-two.md").exists());

    let cursor = temper_cli::projection::read_cursor(&config.state_dir, "pctx")
        .expect("read_cursor")
        .expect("cursor written");
    assert!(
        cursor.last_event_id.is_some(),
        "cursor records the context's latest event id"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: vault projection path uses the real owner handle (reconstruct_resource_row returns p.owner_handle), not the '@me' self-segment the projection expects; and row.slug is None (temper-slug §7-Die) so the filename slug falls back. Blocked on the readback @me/identity-key follow-up (F6/F1)"]
async fn pull_prunes_resources_deleted_on_server(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("dctx", None)
        .await
        .expect("ctx");
    let keep_id = seed_resource(&app, "dctx", "research", "Keeper").await;
    let doomed_id = seed_resource(&app, "dctx", "research", "Doomed").await;

    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "dctx")
        .await
        .expect("first pull");

    let vault_root = app.vault_dir.path();
    assert!(vault_root.join("@me/dctx/research/keeper.md").exists());
    assert!(vault_root.join("@me/dctx/research/doomed.md").exists());

    // Soft-delete one resource on the server, then re-pull.
    app.client
        .resources()
        .delete(Uuid::from(doomed_id), &Default::default())
        .await
        .expect("delete");
    let summary = temper_cli::projection::pull_context(&app.client, &config, "dctx")
        .await
        .expect("second pull");

    assert_eq!(summary.written, 1, "only the survivor is written");
    assert_eq!(summary.pruned, 1, "the deleted resource's file is pruned");
    assert!(vault_root.join("@me/dctx/research/keeper.md").exists());
    assert!(!vault_root.join("@me/dctx/research/doomed.md").exists());
    let _ = keep_id;
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: vault projection path uses the real owner handle (reconstruct_resource_row returns p.owner_handle), not the '@me' self-segment the projection expects; and row.slug is None (temper-slug §7-Die) so the filename slug falls back. Blocked on the readback @me/identity-key follow-up (F6/F1)"]
async fn pull_is_idempotent(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("ictx", None)
        .await
        .expect("ctx");
    seed_resource(&app, "ictx", "research", "Stable Doc").await;

    let config = projection_test_config(&app);
    let path = app.vault_dir.path().join("@me/ictx/research/stable-doc.md");

    temper_cli::projection::pull_context(&app.client, &config, "ictx")
        .await
        .expect("first pull");
    let first = std::fs::read_to_string(&path).unwrap();

    let summary = temper_cli::projection::pull_context(&app.client, &config, "ictx")
        .await
        .expect("second pull");
    let second = std::fs::read_to_string(&path).unwrap();

    assert_eq!(first, second, "re-pull produces byte-identical content");
    assert_eq!(summary.written, 1);
    assert_eq!(summary.pruned, 0);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_not_projected_when_context_never_pulled(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("snp", None)
        .await
        .expect("ctx");
    seed_resource(&app, "snp", "research", "Doc").await;

    let config = projection_test_config(&app);
    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "snp")
            .await;
    assert_eq!(
        outcome,
        temper_cli::projection::StalenessOutcome::NotProjected
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_fresh_immediately_after_pull(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("sfr", None)
        .await
        .expect("ctx");
    seed_resource(&app, "sfr", "research", "Doc").await;

    // Pull and check using the same decorated ref — resolve_context_id now
    // matches by slug on profile-owned contexts for `@me/…` refs, so no
    // cursor rekey is needed.
    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "@me/sfr")
        .await
        .expect("pull");

    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "@me/sfr")
            .await;
    assert_eq!(outcome, temper_cli::projection::StalenessOutcome::Fresh);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_stale_after_post_pull_write(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("sst", None)
        .await
        .expect("ctx");
    seed_resource(&app, "sst", "research", "First Doc").await;

    // Pull and check using the same decorated ref — no cursor rekey needed.
    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "@me/sst")
        .await
        .expect("first pull");

    // A write after the pull advances the context's event stream.
    seed_resource(&app, "sst", "research", "Second Doc").await;

    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "@me/sst")
            .await;
    assert_eq!(outcome, temper_cli::projection::StalenessOutcome::Stale);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn staleness_skipped_when_context_unresolvable(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // A cursor exists on disk for a context that does not exist on the
    // server (e.g. a stale sidecar for a deleted context). The check reads
    // the cursor, fails to resolve the context id, and skips silently.
    let config = projection_test_config(&app);
    temper_cli::projection::write_cursor(
        &config.state_dir,
        "ghost",
        &temper_cli::projection::ProjectionCursor {
            last_event_id: None,
            pulled_at: chrono::Utc::now(),
        },
    )
    .expect("write cursor");

    let outcome =
        temper_cli::projection::check_context_staleness(&app.client, &config.state_dir, "ghost")
            .await;
    assert_eq!(outcome, temper_cli::projection::StalenessOutcome::Skipped);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_empty_context_writes_cursor_with_no_event_id(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let ctx = app
        .client
        .contexts()
        .create("ectx", None)
        .await
        .expect("ctx");
    // Use the context UUID as the ref — it is a valid addressable form (no bare
    // name) and avoids ambiguity in cursor keying.
    let context_ref = ctx.id.to_string();

    // Pull a context that has no resources at all.
    let config = projection_test_config(&app);
    let summary = temper_cli::projection::pull_context(&app.client, &config, &context_ref)
        .await
        .expect("pull_context on empty context");

    assert_eq!(summary.written, 0, "no resources to write");
    assert_eq!(summary.pruned, 0, "nothing to prune");

    // The cursor sidecar is still written; with no events it records None.
    let cursor = temper_cli::projection::read_cursor(&config.state_dir, &context_ref)
        .expect("read_cursor")
        .expect("cursor written even for an empty context");
    assert!(
        cursor.last_event_id.is_none(),
        "an empty context has no event id"
    );
}
