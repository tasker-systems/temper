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
        embedded_with: None,
    };
    let slug = title.to_lowercase().replace(' ', "-");
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: doc_type.to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        content: body.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest")
        .id
}

/// The canonical projection path for a seeded resource.
///
/// The filename is a **bounded decorated ref** — `sluggify(title)` capped at
/// `PROJECTION_SLUG_MAX_BYTES`, then `-<uuid>` — so that an agent-authored title
/// of any length names a file the OS will accept. Derive it here rather than
/// spelling a stem out: a hardcoded literal drifts silently the next time the
/// bound or the scheme moves.
fn projected(
    vault_root: &std::path::Path,
    context: &str,
    doc_type: &str,
    title: &str,
    id: ResourceId,
) -> std::path::PathBuf {
    let stem = temper_workflow::operations::decorated_ref_bounded(
        title,
        id,
        temper_cli::projection::PROJECTION_SLUG_MAX_BYTES,
    );
    vault_root
        .join("@me")
        .join(context)
        .join(doc_type)
        .join(format!("{stem}.md"))
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
    let write_me = seed_resource(&app, "wctx", "research", "Write Me").await;

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
    let me = temper_cli::projection::self_owner_ref(&app.client).await;
    let path =
        temper_cli::projection::write_resource_file(&app.client, vault_root, row, me.as_deref())
            .await
            .expect("write_resource_file")
            .expect("a context-homed resource projects to a path");

    let expected = projected(vault_root, "wctx", "research", "Write Me", write_me);
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
    let me = temper_cli::projection::self_owner_ref(&app.client).await;
    let path = temper_cli::projection::write_resource_file_from_parts(
        vault_root,
        row,
        &content,
        me.as_deref(),
    )
    .expect("write_resource_file_from_parts")
    .expect("a context-homed resource projects to a path");

    let expected = projected(vault_root, "fpctx", "research", "Parts Doc", row.id);
    assert_eq!(path, expected);
    assert!(path.exists(), "file written at canonical path");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.starts_with("---\n"), "has frontmatter fence");
    assert!(on_disk.contains("temper-id:"), "has identity frontmatter");
    assert!(on_disk.contains("Body text for Parts Doc"), "has body");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
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
    let one = seed_resource(&app, "pctx", "research", "Doc One").await;
    let two = seed_resource(&app, "pctx", "research", "Doc Two").await;

    let config = projection_test_config(&app);
    let summary = temper_cli::projection::pull_context(&app.client, &config, "@me/pctx")
        .await
        .expect("pull_context");

    assert_eq!(summary.written, 2, "both resources written");
    assert_eq!(summary.pruned, 0, "nothing stale on a first pull");

    let vault_root = app.vault_dir.path();
    assert!(projected(vault_root, "pctx", "research", "Doc One", one).exists());
    assert!(projected(vault_root, "pctx", "research", "Doc Two", two).exists());

    let cursor = temper_cli::projection::read_cursor(&config.state_dir, "pctx")
        .expect("read_cursor")
        .expect("cursor written");
    assert!(
        cursor.last_event_id.is_some(),
        "cursor records the context's latest event id"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
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
    temper_cli::projection::pull_context(&app.client, &config, "@me/dctx")
        .await
        .expect("first pull");

    let vault_root = app.vault_dir.path();
    let keeper = projected(vault_root, "dctx", "research", "Keeper", keep_id);
    let doomed = projected(vault_root, "dctx", "research", "Doomed", doomed_id);
    assert!(keeper.exists());
    assert!(doomed.exists());

    // Soft-delete one resource on the server, then re-pull.
    app.client
        .resources()
        .delete(Uuid::from(doomed_id), &Default::default())
        .await
        .expect("delete");
    let summary = temper_cli::projection::pull_context(&app.client, &config, "@me/dctx")
        .await
        .expect("second pull");

    assert_eq!(summary.written, 1, "only the survivor is written");
    assert_eq!(summary.pruned, 1, "the deleted resource's file is pruned");
    assert!(keeper.exists());
    assert!(!doomed.exists());
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
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
    let stable = seed_resource(&app, "ictx", "research", "Stable Doc").await;

    let config = projection_test_config(&app);
    let path = projected(
        app.vault_dir.path(),
        "ictx",
        "research",
        "Stable Doc",
        stable,
    );

    temper_cli::projection::pull_context(&app.client, &config, "@me/ictx")
        .await
        .expect("first pull");
    let first = std::fs::read_to_string(&path).unwrap();

    let summary = temper_cli::projection::pull_context(&app.client, &config, "@me/ictx")
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

/// An emptied context prunes the directory the **writer** wrote to, whatever the
/// context's name and slug are.
///
/// A context named `"Temper KB"` has slug `"temper-kb"` — `create` derives the
/// slug with `sluggify` and stores the name canonicalized but otherwise intact.
/// The writer builds `<vault>/@me/Temper KB/…` off `context_name`; the prune path
/// used to fall back to the **slug** half of the ref whenever the context listed
/// no rows, so emptying it swept a `temper-kb` directory that never existed while
/// the real tree survived and looked live.
///
/// Emptying is what makes the two branches disagree: with a row in hand the name
/// is right there, so only the no-rows path could be wrong, and only a context
/// whose name and slug differ can show it. Both halves are load-bearing — a
/// same-spelled context passes against the old code.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_prunes_an_emptied_context_whose_name_and_slug_differ(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let ctx = app
        .client
        .contexts()
        .create("Temper KB", None)
        .await
        .expect("ctx");
    assert_eq!(ctx.slug, "temper-kb", "the slug is derived, and differs");
    assert_eq!(ctx.name, "Temper KB", "the name is stored intact");

    let doomed = seed_resource(&app, "temper-kb", "research", "Only Doc").await;

    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "@me/temper-kb")
        .await
        .expect("first pull");

    let vault_root = app.vault_dir.path();
    // The writer keys the directory on the *name*, not the slug.
    let file = projected(vault_root, "Temper KB", "research", "Only Doc", doomed);
    assert!(
        file.exists(),
        "writer materialized the context under its name, at {}",
        file.display()
    );

    // Empty the context on the server, then re-pull. The context now lists no
    // rows, so the directory name has to come from somewhere other than a row.
    app.client
        .resources()
        .delete(Uuid::from(doomed), &Default::default())
        .await
        .expect("delete");
    let summary = temper_cli::projection::pull_context(&app.client, &config, "@me/temper-kb")
        .await
        .expect("second pull");

    assert_eq!(summary.written, 0, "the context is empty");
    assert_eq!(
        summary.pruned, 1,
        "the emptied context's file is pruned, not orphaned under its real directory"
    );
    assert!(
        !file.exists(),
        "the stale projection file survived at {}",
        file.display()
    );
}

/// `temper --vault <path> pull <ctx>` writes to `<path>`.
///
/// The one command whose entire job is writing to a vault root was also the one
/// `config::load` call site that dropped the flag — `pull::run` called
/// `config::load(None)`, so the write silently went to the *configured* vault
/// instead. The type signature now holds the value (dropping it is a compile
/// error), but nothing asserted it is honoured, and a parameter can be threaded
/// and then ignored.
///
/// This drives the real binary, so it witnesses the whole chain — clap's global
/// `--vault`, `main.rs`'s dispatch, `pull::run`, `config::load` — rather than any
/// one link. `TEMPER_VAULT` is set to the configured vault deliberately: it is
/// what masked the defect originally (`config::load` reads it directly), so the
/// flag has to beat both the config file and the env var, and asserting the
/// projection is absent from that directory is what makes a fallback visible
/// rather than merely unasserted.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_writes_to_the_vault_the_flag_names(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("vctx", None)
        .await
        .expect("ctx");
    let doc = seed_resource(&app, "vctx", "research", "Flagged Doc").await;

    let elsewhere = tempfile::TempDir::new().expect("override vault");
    let configured = app.vault_dir.path().to_path_buf();

    let out = common::run_temper_cli_with_env(
        &app,
        &[("TEMPER_VAULT", configured.to_str().expect("utf-8 path"))],
        &[
            "--vault",
            elsewhere.path().to_str().expect("utf-8 path"),
            "pull",
            "@me/vctx",
        ],
    )
    .await
    .expect("run temper pull");
    assert!(
        out.status.success(),
        "pull failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let honoured = projected(elsewhere.path(), "vctx", "research", "Flagged Doc", doc);
    assert!(
        honoured.exists(),
        "--vault was not honoured: nothing at {}",
        honoured.display()
    );
    let ignored = projected(&configured, "vctx", "research", "Flagged Doc", doc);
    assert!(
        !ignored.exists(),
        "pull wrote to the configured vault despite --vault, at {}",
        ignored.display()
    );
}

/// Pulling a **retired** context leaves the local tree alone.
///
/// The two branches met here. Retirement (PR #777) makes a context invisible to
/// every read path, so the address a caller still holds stops naming anything —
/// and a pull is a command whose job is to delete local files that are no longer
/// on the server. That combination is where a vault gets swept for a context
/// whose every resource is still there, merely retired.
///
/// **Two independent things have to hold, and only the first fires today.** The
/// resource list refuses an unresolvable ref outright, so the pull errors before
/// it can prune anything. Behind that, the prune path can no longer guess a
/// directory name: this branch removed the fallback to the ref's *slug* half, so
/// a context it cannot name prunes nothing. Assert both — the error is what
/// protects the tree now, and the surviving file is what still protects it if
/// the list is ever softened to return zero rows instead of a 404.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pulling_a_retired_context_leaves_its_projection_alone(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let ctx = app
        .client
        .contexts()
        .create("rctx", None)
        .await
        .expect("ctx");
    let doc = seed_resource(&app, "rctx", "research", "Retired Doc").await;

    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "@me/rctx")
        .await
        .expect("first pull");

    let file = projected(app.vault_dir.path(), "rctx", "research", "Retired Doc", doc);
    assert!(file.exists(), "materialized at {}", file.display());

    // Retire the context, then pull the address the caller still holds.
    // Retirement also mangles the slug, so `@me/rctx` names nothing either way.
    app.client
        .contexts()
        .delete(Uuid::from(ctx.id))
        .await
        .expect("retire");

    let err = temper_cli::projection::pull_context(&app.client, &config, "@me/rctx")
        .await
        .expect_err("a retired context is not readable, and the pull must say so");
    let msg = err.to_string();
    assert!(
        msg.contains("not found or not readable"),
        "the refusal must name the unreadable context, got: {msg}"
    );

    assert!(
        file.exists(),
        "pull deleted the projection of a retired context's still-live resource, at {}",
        file.display()
    );
}
