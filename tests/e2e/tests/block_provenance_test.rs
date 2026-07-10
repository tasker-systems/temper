#![cfg(all(feature = "test-db", feature = "test-embed"))]

//! End-to-end coverage for block-provenance surface parity (T7b).
//!
//! Drives the full spine at the production caller's level: the CLI `resource create`/`update`
//! `--sources` path → `temper-client` → Axum → `DbBackend` → substrate → `kb_block_provenance`,
//! then reads it back through the HTTP `GET /api/resources/{id}/provenance` endpoint (the typed
//! `client.resources().provenance()` method the CLI `--provenance` view calls) and the CLI
//! `show --provenance` surface itself.
//!
//! Embed-gated: CLI create/update compute body chunks synchronously (the embed pipeline), so this
//! file only compiles under `test-embed` (the "Embed & MCP Round-Trip" CI job). Run locally with
//! `cargo make test-e2e-embed`.

mod common;

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

/// The `no-such-config.toml` sentinel (cloud mode reads config from env, not disk).
fn global_config_path(app: &common::E2eTestApp) -> String {
    app.vault_dir
        .path()
        .join("no-such-config.toml")
        .to_str()
        .unwrap()
        .to_string()
}

/// Recover the server-assigned id of a resource created earlier in this test, keyed on its
/// (unique) title. Not an addressing path — production addresses by id; the in-process CLI create
/// path does not return the minted id, so the title is the stable handle for the assertion.
async fn created_id_for_title(pool: &sqlx::PgPool, title: &str) -> uuid::Uuid {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT id FROM kb_resources WHERE title = $1 AND is_active \
         ORDER BY created DESC LIMIT 1",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("created_id_for_title({title}): {e}"))
}

/// Drive a cloud-mode CLI `resource create` on a blocking thread (the embed pipeline runs
/// synchronously; nesting runtimes would panic).
async fn cli_create(
    app: &common::E2eTestApp,
    title: &'static str,
    body: String,
    sources: Vec<String>,
) {
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config = global_config_path(app);
    let cli_config = app.cli_config.clone();
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "research",
                    title,
                    context: Some("@me/prov"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: None,
                    body_flag: Some(body),
                    from: None,
                    sources,
                    sources_as_edges: false,
                    no_source: false,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                },
            )
            .expect("cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");
}

/// The full CLI-driven round-trip: create-with-sources records provenance, update-with-a-second-
/// source accretes, and both the HTTP provenance endpoint and the CLI `show --provenance` surface
/// read it back.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sources_round_trip_through_cli_api_db(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("prov", None)
        .await
        .expect("context create");

    // Two source resources to attribute to. `--body @<path>` reads the body from a file.
    let src1_body = app.vault_dir.path().join("src1.md");
    let src2_body = app.vault_dir.path().join("src2.md");
    std::fs::write(&src1_body, "# Source One\n\nFirst source.\n").unwrap();
    std::fs::write(&src2_body, "# Source Two\n\nSecond source.\n").unwrap();
    cli_create(
        &app,
        "Source One",
        format!("@{}", src1_body.display()),
        vec![],
    )
    .await;
    cli_create(
        &app,
        "Source Two",
        format!("@{}", src2_body.display()),
        vec![],
    )
    .await;
    let source1 = created_id_for_title(&pool, "Source One").await;
    let source2 = created_id_for_title(&pool, "Source Two").await;

    // Create a distilled resource attributing its body to source1.
    let dist_body = app.vault_dir.path().join("distilled.md");
    std::fs::write(
        &dist_body,
        "# Distilled Note\n\nDistilled from source one.\n",
    )
    .unwrap();
    cli_create(
        &app,
        "Distilled Note",
        format!("@{}", dist_body.display()),
        vec![source1.to_string()],
    )
    .await;
    let distilled = created_id_for_title(&pool, "Distilled Note").await;

    // ---- Assertion 1: one provenance row, resource→source1, via the HTTP endpoint ----
    let prov = app
        .client
        .resources()
        .provenance(distilled)
        .await
        .expect("provenance read");
    assert_eq!(prov.len(), 1, "one source recorded on create, got {prov:?}");
    assert_eq!(prov[0].source_kind, "resource");
    assert_eq!(prov[0].source_id, source1);
    assert_eq!(prov[0].accretion_seq, 0);

    // ---- Update the distilled body attributing it to source2 → accretion ----
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config = global_config_path(&app);
    let cli_config = app.cli_config.clone();
    let distilled_ref = distilled.to_string();
    let source2_ref = source2.to_string();
    let upd_body = app.vault_dir.path().join("distilled-v2.md");
    std::fs::write(&upd_body, "# Distilled Note\n\nNow also from source two.\n").unwrap();
    let upd_body_flag = format!("@{}", upd_body.display());
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
            let sources = [source2_ref];
            let params = temper_cli::commands::resource::UpdateParams {
                open_meta: None,
                goal: None,
                clear_goal: false,
                r#ref: &distilled_ref,
                type_to: None,
                context_to: None,
                title: None,
                tags: &[],
                aliases: &[],
                relates_to: &[],
                references: &[],
                depends_on: &[],
                extends: &[],
                preceded_by: &[],
                derived_from: &[],
                stage: None,
                mode: None,
                effort: None,
                seq: None,
                branch: None,
                pr: None,
                status: None,
                body: Some(upd_body_flag),
                sources: &sources,
                content_block: None,
                format: temper_cli::format::OutputFormat::Json,
                act: Default::default(),
            };
            temper_cli::commands::resource::update(&cli_config, &params)
                .expect("cloud update with --sources should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 2: provenance accretes — both sources present after the revise ----
    let prov2 = app
        .client
        .resources()
        .provenance(distilled)
        .await
        .expect("provenance read after update");
    let source_ids: Vec<uuid::Uuid> = prov2.iter().map(|r| r.source_id).collect();
    assert!(
        source_ids.contains(&source1) && source_ids.contains(&source2),
        "both sources present after accretion; got {prov2:?}"
    );

    // ---- Assertion 3: the CLI `show --provenance` surface returns cleanly ----
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config = global_config_path(&app);
    let cli_config = app.cli_config.clone();
    let distilled_ref = distilled.to_string();
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
            temper_cli::commands::resource::show(
                &cli_config,
                temper_cli::commands::resource::ShowParams {
                    r#ref: &distilled_ref,
                    format: temper_cli::format::OutputFormat::Json,
                    edges: false,
                    provenance: true,
                    meta_only: false,
                    fields: &[],
                },
            )
            .expect("show --provenance should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");
}

/// A remote (URL) source round-trips end to end (T7c): `create --sources <url>` records a `'remote'`
/// provenance row and the HTTP endpoint surfaces the raw URL (`source_uri`). Proves
/// `ProvenanceSource::Remote` flows the whole CLI → client → Axum → DbBackend → substrate →
/// kb_remote_sources spine with no wire reshape (the same `Vec<ProvenanceSource>` field as T7b).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn remote_url_source_round_trips_through_cli_api_db(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("prov", None)
        .await
        .expect("context create");

    let dist_body = app.vault_dir.path().join("remote-distilled.md");
    std::fs::write(
        &dist_body,
        "# Remote Distilled\n\nDistilled from an external issue.\n",
    )
    .unwrap();
    // Mixed casing in the host is preserved raw and normalized only in the dedup key (server-side).
    let url = "https://Example.com/issue/42";
    cli_create(
        &app,
        "Remote Distilled",
        format!("@{}", dist_body.display()),
        vec![url.to_string()],
    )
    .await;
    let distilled = created_id_for_title(&pool, "Remote Distilled").await;

    let prov = app
        .client
        .resources()
        .provenance(distilled)
        .await
        .expect("provenance read");
    assert_eq!(prov.len(), 1, "one remote source recorded, got {prov:?}");
    assert_eq!(prov[0].source_kind, "remote");
    assert_eq!(
        prov[0].source_uri.as_deref(),
        Some(url),
        "the raw external URL is surfaced, not the minted uuid"
    );
}

/// The #352 default: a create that carries an external (http/https) `origin_uri` but declares NO
/// explicit `--sources` gets a Remote block-provenance row synthesized server-side, pointing at
/// the origin. Drives the real spine (`temper-client` → Axum → `DbBackend` → substrate →
/// kb_remote_sources), which is exactly the path `resource create --from <url>` takes once the CLI
/// wires the URL onto `origin_uri`. Proves "cite the block that supports this claim" works over a
/// URL import with no extra flags — the corpus-wide gap the issue reported.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn external_origin_uri_seeds_remote_provenance_by_default(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("prov", None)
        .await
        .expect("context create");

    // Create through the ingest client with an external origin and NO sources — the same shape the
    // CLI produces for `create --from <url>` (origin_uri populated, sources empty). Server-side
    // chunk+embed is the unconditional fallback (chunks_packed: None).
    use temper_core::types::ingest::IngestPayload;
    let url = "https://Example.com/import/doc-99";
    let body_text = "# Imported Doc\n\nDistilled from an external URL.\n";
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Imported From URL".to_string(),
        origin_uri: url.to_string(),
        context_ref: "@me/prov".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: body_text.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        act: Default::default(),
        sources: Vec::new(),
    };
    let created = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("create with external origin_uri");

    // Acceptance: ≥1 Remote provenance row pointing at the origin URL, with no explicit sources.
    let prov = app
        .client
        .resources()
        .provenance(created.id)
        .await
        .expect("provenance read");
    assert_eq!(
        prov.len(),
        1,
        "the external origin seeds exactly one Remote row, got {prov:?}"
    );
    assert_eq!(prov[0].source_kind, "remote");
    assert_eq!(
        prov[0].source_uri.as_deref(),
        Some(url),
        "provenance points at the origin URL"
    );

    // Acceptance: origin_uri is populated on the created resource.
    let stored_origin: String =
        sqlx::query_scalar("SELECT origin_uri FROM kb_resources WHERE id = $1")
            .bind(created.id)
            .fetch_one(&pool)
            .await
            .expect("resource row");
    assert_eq!(stored_origin, url, "origin_uri is stored on the resource");
}

/// The explicit-`--sources` override still wins even when an external `origin_uri` is present: the
/// declared sources are recorded and NO Remote row is synthesized from the origin (issue #352).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn explicit_sources_override_the_origin_uri_default(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("prov", None)
        .await
        .expect("context create");

    // A resource to attribute to (the explicit source).
    let src_body = app.vault_dir.path().join("ovr-src.md");
    std::fs::write(&src_body, "# Override Source\n\nThe cited resource.\n").unwrap();
    cli_create(
        &app,
        "Override Source",
        format!("@{}", src_body.display()),
        vec![],
    )
    .await;
    let source = created_id_for_title(&pool, "Override Source").await;

    // Create carrying BOTH an external origin_uri and an explicit resource source.
    use temper_core::types::ingest::IngestPayload;
    let url = "https://example.com/should-not-be-cited";
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: "Explicit Over Origin".to_string(),
        origin_uri: url.to_string(),
        context_ref: "@me/prov".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: "# Explicit Over Origin\n\nAttributed to a resource, not the URL.\n".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        act: Default::default(),
        sources: vec![temper_core::types::provenance::ProvenanceSource::Resource(
            source,
        )],
    };
    let created = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("create with explicit source + origin_uri");

    let prov = app
        .client
        .resources()
        .provenance(created.id)
        .await
        .expect("provenance read");
    assert_eq!(prov.len(), 1, "only the explicit source, got {prov:?}");
    assert_eq!(
        prov[0].source_kind, "resource",
        "explicit --sources wins; no Remote synthesized from origin_uri"
    );
    assert_eq!(prov[0].source_id, source);
}

/// Per-content-block addressing round-trips (T7c Task 11): `update --content-block <id> --sources`
/// applies the revise + sources to the addressed block (discovered via the provenance read), and a
/// `--content-block` that does not belong to the resource is rejected with no write. Drives the full
/// CLI → client → Axum → DbBackend → substrate spine at the production caller's level.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn content_block_addressing_round_trips_through_cli_api_db(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("prov", None)
        .await
        .expect("context create");

    // Two source resources + a distilled note attributed to source1 on create.
    let src1_body = app.vault_dir.path().join("cb-src1.md");
    let src2_body = app.vault_dir.path().join("cb-src2.md");
    std::fs::write(&src1_body, "# CB Source One\n\nFirst.\n").unwrap();
    std::fs::write(&src2_body, "# CB Source Two\n\nSecond.\n").unwrap();
    cli_create(
        &app,
        "CB Source One",
        format!("@{}", src1_body.display()),
        vec![],
    )
    .await;
    cli_create(
        &app,
        "CB Source Two",
        format!("@{}", src2_body.display()),
        vec![],
    )
    .await;
    let source1 = created_id_for_title(&pool, "CB Source One").await;
    let source2 = created_id_for_title(&pool, "CB Source Two").await;

    let dist_body = app.vault_dir.path().join("cb-distilled.md");
    std::fs::write(&dist_body, "# CB Distilled\n\nFrom source one.\n").unwrap();
    cli_create(
        &app,
        "CB Distilled",
        format!("@{}", dist_body.display()),
        vec![source1.to_string()],
    )
    .await;
    let distilled = created_id_for_title(&pool, "CB Distilled").await;

    // Discover the body block's id the way a user would — through the provenance read.
    let prov = app
        .client
        .resources()
        .provenance(distilled)
        .await
        .expect("provenance read");
    assert_eq!(prov.len(), 1, "one source on create, got {prov:?}");
    let block_id = prov[0].block_id;

    // ---- Positive: update addressing that block explicitly accretes source2 onto it ----
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config = global_config_path(&app);
    let cli_config = app.cli_config.clone();
    let distilled_ref = distilled.to_string();
    let source2_ref = source2.to_string();
    let upd_body = app.vault_dir.path().join("cb-distilled-v2.md");
    std::fs::write(&upd_body, "# CB Distilled\n\nNow also from source two.\n").unwrap();
    let upd_body_flag = format!("@{}", upd_body.display());
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
            let sources = [source2_ref];
            let params = temper_cli::commands::resource::UpdateParams {
                open_meta: None,
                goal: None,
                clear_goal: false,
                r#ref: &distilled_ref,
                type_to: None,
                context_to: None,
                title: None,
                tags: &[],
                aliases: &[],
                relates_to: &[],
                references: &[],
                depends_on: &[],
                extends: &[],
                preceded_by: &[],
                derived_from: &[],
                stage: None,
                mode: None,
                effort: None,
                seq: None,
                branch: None,
                pr: None,
                status: None,
                body: Some(upd_body_flag),
                sources: &sources,
                content_block: Some(block_id),
                format: temper_cli::format::OutputFormat::Json,
                act: Default::default(),
            };
            temper_cli::commands::resource::update(&cli_config, &params)
                .expect("update addressing the resource's own block should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    let prov2 = app
        .client
        .resources()
        .provenance(distilled)
        .await
        .expect("provenance read after addressed update");
    let ids: Vec<uuid::Uuid> = prov2.iter().map(|r| r.source_id).collect();
    assert!(
        ids.contains(&source1) && ids.contains(&source2),
        "both sources present on the addressed block after accretion; got {prov2:?}"
    );
    assert!(
        prov2.iter().all(|r| r.block_id == block_id),
        "every provenance row is on the addressed block; got {prov2:?}"
    );

    // ---- Negative: a content_block that does not belong to the resource is rejected, no write ----
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config = global_config_path(&app);
    let cli_config = app.cli_config.clone();
    let distilled_ref = distilled.to_string();
    let source2_ref = source2.to_string();
    let foreign_block = uuid::Uuid::now_v7();
    let upd_body3 = app.vault_dir.path().join("cb-distilled-v3.md");
    std::fs::write(&upd_body3, "# CB Distilled\n\nShould not be written.\n").unwrap();
    let upd_body3_flag = format!("@{}", upd_body3.display());
    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config), || {
            let sources = [source2_ref];
            let params = temper_cli::commands::resource::UpdateParams {
                open_meta: None,
                goal: None,
                clear_goal: false,
                r#ref: &distilled_ref,
                type_to: None,
                context_to: None,
                title: None,
                tags: &[],
                aliases: &[],
                relates_to: &[],
                references: &[],
                depends_on: &[],
                extends: &[],
                preceded_by: &[],
                derived_from: &[],
                stage: None,
                mode: None,
                effort: None,
                seq: None,
                branch: None,
                pr: None,
                status: None,
                body: Some(upd_body3_flag),
                sources: &sources,
                content_block: Some(foreign_block),
                format: temper_cli::format::OutputFormat::Json,
                act: Default::default(),
            };
            temper_cli::commands::resource::update(&cli_config, &params)
        })
    })
    .await
    .expect("spawn_blocking joined");
    assert!(
        result.is_err(),
        "addressing a content_block that does not belong to the resource must fail"
    );

    // Provenance is unchanged by the rejected update — still exactly the two accreted sources.
    let prov3 = app
        .client
        .resources()
        .provenance(distilled)
        .await
        .expect("provenance read after rejected update");
    assert_eq!(
        prov3.len(),
        prov2.len(),
        "the rejected update wrote nothing; got {prov3:?}"
    );
}
