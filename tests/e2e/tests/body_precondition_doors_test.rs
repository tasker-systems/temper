#![cfg(feature = "test-db")]
//! Conditional body saves through the MCP and CLI doors.
//!
//! The mechanism and its HTTP door are witnessed in `temper-services`' `body_precondition_test.rs`
//! and `temper-api`'s `body_token_http_test.rs`. This suite witnesses that each door above them
//! answers the token beside the markdown it emits, carries the body guard end to end, and passes a
//! write's own token through:
//!
//! - **MCP** `get_resource` (with `include_content`), the `temper://resources/{id}` resource, and
//!   `update_resource` with `expect_body`, driven through the deployed relay against this
//!   process's real listener.
//! - **The CLI** `resource show --json` and `resource update --expect-body`: the real `temper`
//!   binary. Build it first (`cargo build -p temper-cli --bin temper`).

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

use common::E2eTestApp;

const EMAIL: &str = "e2e@test.example.com";

async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

fn input<T: for<'de> serde::Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

fn one_text(res: &rmcp::model::CallToolResult) -> Value {
    serde_json::from_str(res.content[0].as_text().expect("a text part").text.as_str())
        .expect("the first part is the tool's JSON response")
}

async fn default_context_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c JOIN kb_profiles p ON p.id = c.owner_id \
          WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("the auto-provisioned default context")
}

/// A note with `body`, created through the MCP door.
async fn create_note(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    pool: &PgPool,
    body: &str,
) -> String {
    let ctx = default_context_id(pool, EMAIL).await;
    let created = one_text(
        &temper_mcp::tools::resources::create_resource(
            svc,
            parts,
            input(json!({
                "context_ref": ctx.to_string(),
                "doc_type_name": "research",
                "title": "Drafted",
                "content": body,
            })),
        )
        .await
        .expect("create lands"),
    );
    created["resource"]["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

/// `get_resource` with the body: the JSON part and the markdown part.
async fn get_with_body(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    id: &str,
) -> (Value, String) {
    let res = temper_mcp::tools::resources::get_resource(
        svc,
        parts,
        input(json!({"id": id, "include_content": true})),
    )
    .await
    .expect("get lands");
    let meta = one_text(&res);
    let markdown = res
        .content
        .last()
        .and_then(|c| c.as_text())
        .expect("a markdown part")
        .text
        .clone();
    (meta, markdown)
}

async fn stored_body(app: &E2eTestApp, id: &str) -> String {
    app.client
        .resources()
        .content(id.parse().unwrap())
        .await
        .expect("content")
        .markdown
}

/// The refusal an MCP body save carries, after asserting its shape: a tool error result whose
/// `structuredContent` is the HTTP door's `error` object, with one `body` entry and no `key`.
fn body_refusal(answer: &Result<rmcp::model::CallToolResult, rmcp::ErrorData>) -> Value {
    let result = match answer {
        Ok(r) => r,
        Err(e) => panic!("a refusal is a tool result, never a JSON-RPC error: {e:?}"),
    };
    assert_eq!(result.is_error, Some(true), "an error result: {result:?}");
    let data = result
        .structured_content
        .as_ref()
        .expect("the refusal rides structuredContent");
    assert_eq!(data["code"], "PRECONDITION_FAILED", "{data}");
    assert_eq!(data["message"], "1 precondition failed: body", "{data}");
    let entry = data["precondition"]["failed"][0].clone();
    let mut keys: Vec<&str> = entry
        .as_object()
        .expect("an entry")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["current", "expected", "tier"], "{entry}");
    assert_eq!(entry["tier"], "body");
    entry
}

/// **MCP carries the body guard and passes the write's token through.** `get_resource` with the
/// body answers the token of the markdown it emits; `update_resource` with that token as
/// `expect_body` lands and answers the token of the body it wrote (the PATCH answer's, which the
/// re-read view does not carry); the same stale token is then refused with a `body` entry whose
/// `current` is the token `get_resource` answers next. A save without `content` answers no token.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_carries_expect_body_and_passes_its_token_through(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_note(&svc, &parts, &app.pool, "# Draft\n\nfirst\n").await;
    let (meta, markdown) = get_with_body(&svc, &parts, &id).await;
    let read = meta["body_token"].as_str().expect("a token").to_string();
    assert_eq!(read, temper_core::body_token(&markdown));

    let landed = one_text(
        &temper_mcp::tools::resources::update_resource(
            &svc,
            &parts,
            input(json!({"id": id, "content": "# Draft\n\nsecond\n", "expect_body": read})),
        )
        .await
        .expect("a fresh token lands"),
    );
    let written = temper_core::body_token("# Draft\n\nsecond\n");
    assert_eq!(landed["body_token"], json!(written), "{landed}");
    assert_eq!(
        get_with_body(&svc, &parts, &id).await.0["body_token"],
        json!(written)
    );

    let stale = temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({"id": id, "content": "# Draft\n\nthird\n", "expect_body": read})),
    )
    .await;
    let entry = body_refusal(&stale);
    assert_eq!(entry["expected"], json!(read));
    assert_eq!(entry["current"], json!(written));
    assert_eq!(stored_body(&app, &id).await, "# Draft\n\nsecond\n");

    let meta_only = one_text(
        &temper_mcp::tools::resources::update_resource(
            &svc,
            &parts,
            input(json!({"id": id, "title": "Renamed"})),
        )
        .await
        .expect("a title change"),
    );
    assert!(meta_only.get("body_token").is_none(), "{meta_only}");
}

/// **Read standing first, through MCP.** A caller who may modify a resource it cannot read gets
/// the not-found error for any `expect_body`, matching or not: never a refusal, never a token.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_reader_gets_not_found_for_any_expect_body(pool: PgPool) {
    let (app, svc, _) = harness(pool).await;
    let (admin, telos, matching) = modify_but_not_read(&app).await;
    for guard in [matching.as_str(), "bt1.wrong"] {
        let err = temper_mcp::tools::resources::update_resource(
            &svc,
            &admin,
            input(json!({"id": telos, "content": "# Telos\n\nx\n", "expect_body": guard})),
        )
        .await
        .expect_err("refused as a not-found");
        assert_eq!(err.code.0, -32602, "{err:?}");
        assert!(err.message.to_lowercase().contains("not found"), "{err:?}");
        assert!(err.data.is_none(), "{err:?}");
        assert!(!err.message.contains("bt1"), "{err:?}");
    }
}

/// A map in the admin-only regime with a telos the harness principal owns, and a system admin who
/// may modify the telos but cannot read it. Returns the admin's parts, the telos, and the token the
/// owner reads for it. As in `meta_precondition_doors_test.rs`.
async fn modify_but_not_read(app: &E2eTestApp) -> (axum::http::request::Parts, String, String) {
    let unique = Uuid::new_v4();
    let email = format!("regime-admin-{unique}@example.com");
    let token = common::generate_test_jwt(&format!("regime-admin-sub-{unique}"), &email);
    let _ = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await;
    let admin: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind(&email)
        .fetch_one(&app.pool)
        .await
        .expect("admin");
    common::approved_admin(&app.pool, admin).await;
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind(EMAIL)
        .fetch_one(&app.pool)
        .await
        .expect("owner");
    let cogmap = Uuid::now_v7();
    let telos = Uuid::now_v7();
    sqlx::query(
        "SELECT cogmap_genesis($1, '{}'::jsonb, \
             (SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
               WHERE p.handle = 'system' AND e.name = 'system'))",
    )
    .bind(json!({
        "cogmap_id": cogmap,
        "name": format!("regime-{cogmap}"),
        "owner_profile_id": owner,
        "telos": {"resource_id": telos, "title": "telos",
                  "origin_uri": format!("temper://test/{cogmap}/telos"), "blocks": []},
    }))
    .execute(&app.pool)
    .await
    .expect("birth the map");
    let matching = app
        .client
        .resources()
        .content(telos)
        .await
        .expect("the owner reads the telos")
        .body_token
        .expect("a token");
    let team = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, 'regime-gate', 'regime-gate')")
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("seed the gating team");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("bind the map");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'regime-gate' WHERE id = 1")
        .execute(&app.pool)
        .await
        .expect("point the gate at it");
    let (modify, read): (bool, bool) = sqlx::query_as(
        "SELECT can_modify_resource($1, $2), \
                EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(admin)
    .bind(telos)
    .fetch_one(&app.pool)
    .await
    .expect("standing");
    assert!(modify && !read, "modify {modify}, read {read}");
    (app.relay_parts_for(&token), telos.to_string(), matching)
}

// ── the same response ────────────────────────────────────────────────────────────────────────────

/// A forwarding proxy in front of the API that lands a save of `next` directly on the API right
/// after it relays the first single-resource view read (`GET /api/resources/{id}`), before the
/// door's next request: the interleave between a door's two reads.
struct InterleavingProxy {
    upstream: String,
    http: reqwest::Client,
    token: String,
    next: String,
    views: AtomicUsize,
}

async fn forward(
    axum::extract::State(proxy): axum::extract::State<Arc<InterleavingProxy>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (head, body) = req.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("read the body");
    let path = head
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let mut upstream = proxy
        .http
        .request(head.method.clone(), format!("{}{}", proxy.upstream, path));
    for (name, value) in &head.headers {
        if !matches!(name.as_str(), "host" | "content-length" | "connection") {
            upstream = upstream.header(name, value);
        }
    }
    let answer = upstream.body(body).send().await.expect("the API answers");
    let status = answer.status();
    let headers = answer.headers().clone();
    let bytes = answer.bytes().await.expect("the API's body");
    let is_view = head.method == axum::http::Method::GET
        && head.uri.path().starts_with("/api/resources/")
        && head.uri.path().matches('/').count() == 3;
    if is_view && proxy.views.fetch_add(1, Ordering::SeqCst) == 0 {
        let id = head.uri.path().trim_start_matches("/api/resources/");
        let saved = proxy
            .http
            .patch(format!("{}/api/resources/{id}", proxy.upstream))
            .bearer_auth(&proxy.token)
            .json(&json!({"content": proxy.next}))
            .send()
            .await
            .expect("the interleaved save");
        assert!(saved.status().is_success(), "the interleaved save lands");
    }
    let mut response = (status, bytes).into_response();
    for (name, value) in &headers {
        if !matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection"
        ) {
            response.headers_mut().insert(name.clone(), value.clone());
        }
    }
    response
}

/// A tool service whose relay goes through an [`InterleavingProxy`] that saves `next`.
async fn interleaved(
    app: &E2eTestApp,
    next: &str,
) -> (TemperMcpService, tokio::task::JoinHandle<Option<()>>) {
    let proxy = Arc::new(InterleavingProxy {
        upstream: app.base_url(),
        http: reqwest::Client::new(),
        token: app.token.clone(),
        next: next.to_string(),
        views: AtomicUsize::new(0),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the proxy");
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new().fallback(forward).with_state(proxy);
    let serve = tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let svc = temper_mcp_server::tool_service(
        temper_mcp_server::config::blob_door(None, false),
        common::deployed_relay_to(&proxy_url),
    );
    (svc, serve)
}

/// **The emitted markdown and its token stay paired across an interleaved save.** A save lands
/// between a door's reads. `get_resource` emits the markdown and token of its one read, so the
/// pair is the earlier body's, and that token is then refused as stale rather than saving over
/// the interleaved body. The `temper://resources/{id}` resource reads the view and then
/// `/content`; its token is the `/content` response's, so it names the markdown it emits (the
/// interleaved body), never the view's earlier one.
///
/// FAILS IF a door answers a token from a different request than its markdown: the pair then
/// names two different bodies, and a save guarded by that token can overwrite bytes the caller
/// never read.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_emitted_markdown_and_token_come_from_one_response(pool: PgPool) {
    let (app, direct, parts) = harness(pool).await;
    let id = create_note(&direct, &parts, &app.pool, "# Draft\n\nbefore\n").await;

    let (svc, serve) = interleaved(&app, "# Draft\n\ninterleaved\n").await;
    let (meta, markdown) = get_with_body(&svc, &parts, &id).await;
    serve.abort();
    assert_eq!(markdown, "# Draft\n\nbefore\n");
    assert_eq!(
        meta["body_token"],
        json!(temper_core::body_token(&markdown))
    );
    assert_eq!(stored_body(&app, &id).await, "# Draft\n\ninterleaved\n");
    let stale = temper_mcp::tools::resources::update_resource(
        &direct,
        &parts,
        input(json!({"id": id, "content": "# Draft\n\nmine\n", "expect_body": meta["body_token"]})),
    )
    .await;
    body_refusal(&stale);
    assert_eq!(stored_body(&app, &id).await, "# Draft\n\ninterleaved\n");

    let (svc, serve) = interleaved(&app, "# Draft\n\nagain\n").await;
    let client = svc.relay_client(&parts).expect("relay client");
    let read = temper_mcp::resources::read_resource(
        &client,
        rmcp::model::ReadResourceRequestParams::new(format!("temper://resources/{id}")),
    )
    .await
    .expect("the resource read");
    serve.abort();
    let read = serde_json::to_value(&read).unwrap();
    let parts_json = read["contents"].as_array().expect("contents");
    let meta: Value = serde_json::from_str(parts_json[0]["text"].as_str().unwrap()).unwrap();
    let markdown = parts_json[1]["text"].as_str().unwrap();
    assert_eq!(
        markdown, "# Draft\n\nagain\n",
        "the /content read came after the save"
    );
    assert_eq!(meta["body_token"], json!(temper_core::body_token(markdown)));
}

// ── CLI ──────────────────────────────────────────────────────────────────────────────────────────

async fn cli(app: &E2eTestApp, args: &[&str]) -> std::process::Output {
    common::run_temper_cli(app, args).await.expect("cli run")
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is one JSON document ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// **The CLI: `show --json` answers the token beside the body it prints, and `update
/// --expect-body` carries it.** A save guarded by the shown token lands and prints the token of
/// the body it wrote; the same stale token is then refused (exit 1, code `PRECONDITION_FAILED`, a
/// `body` entry in `details.failed`), and nothing is written.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_show_answers_the_token_and_update_carries_expect_body(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_note(&svc, &parts, &app.pool, "# Draft\n\nfirst\n").await;

    let shown = cli(&app, &["--format", "json", "resource", "show", &id]).await;
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown = stdout_json(&shown);
    let body = shown["content"]
        .as_str()
        .expect("show --json prints the body as content");
    let token = shown["body_token"]
        .as_str()
        .expect("show --json answers a token")
        .to_string();
    assert_eq!(token, temper_core::body_token(body), "{shown}");

    let path = app.vault_dir.path().join("next.md");
    std::fs::write(&path, "# Draft\n\nsecond\n").unwrap();
    let path = format!("@{}", path.to_str().unwrap());
    let path = path.as_str();
    let landed = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--body",
            path,
            "--expect-body",
            &token,
        ],
    )
    .await;
    assert!(
        landed.status.success(),
        "{}",
        String::from_utf8_lossy(&landed.stderr)
    );
    assert_eq!(stored_body(&app, &id).await, "# Draft\n\nsecond\n");

    std::fs::write(app.vault_dir.path().join("next.md"), "# Draft\n\nthird\n").unwrap();
    let refused = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--body",
            path,
            "--expect-body",
            &token,
        ],
    )
    .await;
    assert_eq!(refused.status.code(), Some(1), "a refusal exits 1");
    let payload = stdout_json(&refused);
    assert_eq!(payload["code"], "PRECONDITION_FAILED", "{payload}");
    assert_eq!(payload["message"], "1 precondition failed: body");
    assert_eq!(
        payload["details"]["failed"],
        json!([{
            "tier": "body",
            "expected": token,
            "current": temper_core::body_token("# Draft\n\nsecond\n"),
        }])
    );
    assert_eq!(stored_body(&app, &id).await, "# Draft\n\nsecond\n");
}
