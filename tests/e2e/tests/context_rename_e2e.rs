#![cfg(feature = "test-db")]
//! Context-rename e2e: **surface parity for a two-dialect refusal**, and the re-addressing the
//! rename performs.
//!
//! `POST /api/contexts/{id}/rename` is gated by `ContextAdminAuthority`, which answers in *two*
//! dialects: `403` to a principal who reads the context but does not administer it, `404` to one
//! who cannot see it at all. The register's closure class 3 claims the **surface is immaterial** —
//! the same two callers must get the same two, still-distinguishable, refusals over HTTP, over the
//! CLI, and over MCP. That claim is not free: the gate lives in `temper-services` so the *decision*
//! cannot diverge, but the *rendering* still could, and MCP renders through `map_api_error`, which
//! the gate never sees. This file is the only level that can show all three agree.
//!
//! Modeled on `context_transfer_e2e.rs` (its `provision` / `root_bootstrap_first_admin` helpers and
//! the `common::approve` / `common::approved_admin` idiom), with an MCP harness copied from
//! `resource_facet_e2e_test.rs` — the suite's per-file convention is a local copy so each file's
//! harness can evolve independently.
//!
//! **The gate itself is exercised at the unit tier**, in `temper-services`'
//! `authz::context_admin` test module: all four mechanisms that produce a reader (two
//! `contexts_readable_by` arms, a `kb_team_contexts` share, a `kb_access_grants` grant) get one
//! test each there. What is owed *here* is one representative reader against every surface, which
//! is a different claim and needs a different tier.
//!
//! ⚠️ **The CLI cases pass a bare UUID, deliberately.** `resolve_context_id_for_read` lists
//! contexts client-side and matches locally, so a decorated `@owner/slug` naming a context the
//! caller cannot see fails inside the CLI with *"not found among the contexts you can see"* and
//! **never reaches the server** — testing the local resolver instead of the gate. `Uuid::parse_str`
//! short-circuits that path, which is what puts the refusal on the wire.
//!
//! No embedding work is inspected here, so this runs on plain `cargo make test-e2e`.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::context::RenameContextRequest;
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door and the
    // ENDPOINT authz (the rename gate) is what the test exercises.
    let pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, pid).await;
    pid
}

/// The irreducible operator root step: configure gating + mint first admin.
async fn root_bootstrap_first_admin(pool: &sqlx::PgPool, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .expect("gating");
    common::approved_admin(pool, admin_id).await;
}

/// Seed the representative reader: an explicit profile-anchored read grant on the context.
///
/// One of the four routes the gate tests enumerate, chosen here because it is the one that needs no
/// second act to set up — the context stays personal and nothing but this row changes the answer.
/// `granted_by_profile_id` is **NOT NULL** on `kb_access_grants`.
async fn grant_context_read(pool: &sqlx::PgPool, context: Uuid, principal: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(context)
    .bind(principal)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("seed context read-grant");
}

/// Wire-level status of `POST /api/contexts/{context}/rename` as `token`.
async fn rename_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    name: &str,
) -> StatusCode {
    app.reqwest_client
        .post(app.url(&format!("/api/contexts/{context_id}/rename")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&RenameContextRequest {
            name: name.to_owned(),
        })
        .send()
        .await
        .expect("rename request")
        .status()
}

/// `GET /api/contexts/{id}` status — the read oracle the two dialects are defined against.
async fn context_get_status(app: &common::E2eTestApp, token: &str, context_id: Uuid) -> StatusCode {
    app.reqwest_client
        .get(app.url(&format!("/api/contexts/{context_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("context get")
        .status()
}

/// `POST /api/ingest` homed at `context_ref`, as `token`. The server-side **ref resolution**
/// oracle: `resolve_context_ref` is what a stale `@owner/slug` meets.
async fn ingest_status(
    app: &common::E2eTestApp,
    token: &str,
    context_ref: &str,
    slug: &str,
) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": format!("ctx-rename {slug}"),
            "origin_uri": format!("test://context-rename-e2e/{}", Uuid::new_v4()),
            "context_ref": context_ref,
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource authored into a context.",
        }))
        .send()
        .await
        .expect("ingest request");
    let status = resp.status();
    let body = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// Run the real `temper` binary as an arbitrary token, and return `(success, stderr)`.
///
/// `run_temper_cli` uses `app.token`; the readers here are *other* principals, so this materializes
/// the config once and goes through `run_temper_cli_with_token`.
async fn cli_rename(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    name: &str,
) -> (bool, String) {
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig");
    let config_path = app.vault_dir.path().join("test-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config");

    // A BARE UUID — see the module doc. A decorated ref would never reach the server.
    let bare_uuid = context_id.to_string();
    let out = common::run_temper_cli_with_token(
        &app.base_url(),
        token,
        &config_path,
        &["context", "rename", &bare_uuid, "--name", name],
    )
    .await
    .expect("spawn temper");

    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// An MCP service over the same pool, with its profile cache seeded for `sub`.
///
/// One service per principal: the cache is per-service, so the reader and the stranger cannot
/// share one. Local to this file, matching `resource_facet_e2e_test.rs`'s own copy.
async fn mcp_service_for(pool: &sqlx::PgPool, sub: &str) -> temper_mcp::service::TemperMcpService {
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
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let svc = temper_mcp::service::TemperMcpService::new(state);

    let (parts, ()) = axum::http::Request::builder()
        .extension(temper_mcp::middleware::BearerToken("synthetic".to_string()))
        .extension(temper_services::auth::RawJwtClaims {
            sub: sub.to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            iat: 0,
        })
        .body(())
        .expect("build request")
        .into_parts();
    svc.ensure_profile_from_parts(&parts)
        .await
        .expect("seed profile cache");
    svc
}

/// The refusal message the MCP `rename_context` tool renders for a caller.
async fn mcp_rename_refusal(
    svc: &temper_mcp::service::TemperMcpService,
    context_id: Uuid,
    name: &str,
) -> String {
    let err = temper_mcp::tools::contexts::rename_context(
        svc,
        temper_mcp::tools::contexts::RenameContextInput {
            context: context_id,
            name: name.to_owned(),
        },
    )
    .await
    .expect_err("the caller may not rename this context");
    err.message.to_string()
}

/// Class 3, over every door: the reader's `403` and the stranger's `404` stay themselves, and stay
/// distinguishable from each other, on HTTP, on the CLI and on MCP.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_two_dialects_survive_every_surface(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    let reader_token = common::generate_second_user_jwt();
    let reader_id = provision(&app, &reader_token).await;
    let stranger_token = common::generate_third_user_jwt();
    provision(&app, &stranger_token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    // The admin's personal context; the reader gets an explicit read grant, the stranger nothing.
    let context = app
        .client
        .contexts()
        .create("Proj", None)
        .await
        .expect("admin creates context");
    grant_context_read(&pool, *context.id, reader_id, admin_id).await;

    // ── the precondition the whole split rests on ────────────────────────────────────────────
    // Without this the two refusals below could both be correct for the wrong reason. The reader
    // demonstrably READS the context, which is exactly why a `404` to them would be a lie about a
    // row they can `GET`; the stranger demonstrably cannot.
    assert_eq!(
        context_get_status(&app, &reader_token, *context.id).await,
        StatusCode::OK,
        "fixture precondition: the reader can read this context"
    );
    assert_eq!(
        context_get_status(&app, &stranger_token, *context.id).await,
        StatusCode::NOT_FOUND,
        "fixture precondition: the stranger cannot see this context at all"
    );

    // ── HTTP ─────────────────────────────────────────────────────────────────────────────────
    assert_eq!(
        rename_status(&app, &reader_token, *context.id, "Reader Rename").await,
        StatusCode::FORBIDDEN,
        "a reader who does not administer the context is refused with 403, not the \
         existence-hiding 404"
    );
    assert_eq!(
        rename_status(&app, &stranger_token, *context.id, "Stranger Rename").await,
        StatusCode::NOT_FOUND,
        "a non-reader gets the existence-hiding 404 — the rename endpoint is not an oracle"
    );

    // ── CLI ──────────────────────────────────────────────────────────────────────────────────
    let (reader_ok, reader_err) = cli_rename(&app, &reader_token, *context.id, "Reader CLI").await;
    assert!(!reader_ok, "the CLI must fail: {reader_err}");
    assert!(
        reader_err.contains("administer the context"),
        "the CLI renders the 403 as the administration requirement: {reader_err}"
    );

    let (stranger_ok, stranger_err) =
        cli_rename(&app, &stranger_token, *context.id, "Stranger CLI").await;
    assert!(!stranger_ok, "the CLI must fail: {stranger_err}");
    assert!(
        stranger_err.contains("context not found or not readable"),
        "the CLI carries the server's own withholding 404 message: {stranger_err}"
    );
    assert_ne!(
        reader_err, stranger_err,
        "403 and 404 must stay distinguishable at the CLI surface"
    );

    // ── MCP ──────────────────────────────────────────────────────────────────────────────────
    // The surface with its own mapper (`map_api_error`), which the shared gate never sees — so
    // this is the door where the rendering could diverge even though the decision cannot.
    let reader_svc = mcp_service_for(&pool, "e2e-second-user").await;
    let reader_mcp = mcp_rename_refusal(&reader_svc, *context.id, "Reader MCP").await;
    assert!(
        reader_mcp.contains("administer the context"),
        "MCP renders the 403 as the (one-sided) administration requirement: {reader_mcp}"
    );
    assert!(
        !reader_mcp.contains("target team"),
        "rename has no target team — that clause belongs to share/unshare/transfer: {reader_mcp}"
    );

    let stranger_svc = mcp_service_for(&pool, "e2e-third-user").await;
    let stranger_mcp = mcp_rename_refusal(&stranger_svc, *context.id, "Stranger MCP").await;
    assert!(
        stranger_mcp.contains("context not found or not readable"),
        "MCP carries the service's own withholding message: {stranger_mcp}"
    );
    assert_ne!(
        reader_mcp, stranger_mcp,
        "403 and 404 must stay distinguishable at the MCP surface too — a mapper that collapsed \
         them would discard the disclosure distinction the gate deliberately draws"
    );

    // Six refusals, and not one of them wrote anything.
    let stored: (String, String) =
        sqlx::query_as("SELECT name, slug FROM kb_contexts WHERE id = $1")
            .bind(*context.id)
            .fetch_one(&pool)
            .await
            .expect("context row");
    assert_eq!(
        stored,
        ("Proj".to_string(), "proj".to_string()),
        "a refused rename changes nothing"
    );
}

/// The happy path, and the re-addressing it performs: the old `@owner/slug` stops resolving, the
/// new one starts, and the context keeps its contents through the move.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_administrator_renames_and_the_context_is_re_addressed(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let context = app
        .client
        .contexts()
        .create("Project Alpha", None)
        .await
        .expect("admin creates context");
    assert_eq!(context.slug, "project-alpha");
    let old_ref = format!("{}/{}", context.owner_ref, context.slug);

    // A resource homed in the context BEFORE the rename — the thing that must survive it.
    let (ing, body) = ingest_status(&app, &app.token, &old_ref, "before-rename").await;
    assert_eq!(ing, StatusCode::OK, "admin authors into their own context");
    let resource_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    // ── the rename, through the client (the wire `temper context rename` uses) ────────────────
    let outcome = app
        .client
        .contexts()
        .rename(
            *context.id,
            &RenameContextRequest {
                name: "Renamed Project".to_owned(),
            },
        )
        .await
        .expect("the owner administers their own context");

    assert!(outcome.renamed, "the name moved, so this is a rename");
    assert_eq!(outcome.name, "Renamed Project");
    assert_eq!(outcome.slug, "renamed-project", "the slug is DERIVED");
    assert_eq!(
        outcome.owner_ref, context.owner_ref,
        "a rename leaves the owner untouched"
    );
    // The composed ref is the point of the field: the caller has just had their address changed
    // out from under them and must not have to reassemble the new one from parts.
    assert_eq!(
        outcome.context_ref,
        format!("{}/renamed-project", context.owner_ref),
        "the outcome carries the composed new ref"
    );

    // ── the address really moved ─────────────────────────────────────────────────────────────
    let (stale, _) = ingest_status(&app, &app.token, &old_ref, "via-stale-ref").await;
    assert_eq!(
        stale,
        StatusCode::NOT_FOUND,
        "the old @owner/slug no longer resolves — this is the re-addressing, stated not mitigated"
    );
    let (fresh, _) = ingest_status(&app, &app.token, &outcome.context_ref, "via-new-ref").await;
    assert_eq!(
        fresh,
        StatusCode::OK,
        "the new @owner/slug resolves to the same context"
    );

    // ── and the context did not lose its contents to the rename ──────────────────────────────
    let show = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("show request")
        .status();
    assert_eq!(show, StatusCode::OK, "the homed resource is still readable");

    // `fetch_all`, not `fetch_one`: sqlx's `fetch_one` silently takes the first of several rows, so
    // a rename that APPENDED a second home would slip past it. The row count is part of the claim.
    let homes: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("resource homes");
    assert_eq!(
        homes,
        vec![("kb_contexts".to_string(), *context.id)],
        "still homed in the SAME context, exactly once — the identity pair moved, the container \
         did not"
    );
}
