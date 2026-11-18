#![cfg(feature = "test-db")]
//! MCP `update_resource` moves and converts a resource, as the CLI does — task
//! `01a1ec86-0d90-767a-b7a6-6d97c916bfc4`.
//!
//! The HTTP wire always carried `context_to` and `type_to`; the CLI sent them and the MCP door
//! hard-coded both to `None`, so an MCP-only agent could not re-home or convert a resource. These
//! witnesses drive the tool through the production relay (real listener, real bearer) and the CLI
//! through the real binary:
//!
//! 1. **Re-home** — `context_to` moves the resource; id, body and both meta tiers are kept.
//! 2. **Door parity** — the same move through the CLI and through MCP leaves the same stored state
//!    and emits the same events (the same backend command, seen from the ledger).
//! 3. **Refusals** — a destination the caller can read but not write is refused with the kind the
//!    tool gives every other `403`, HTTP answers it `403`, and the sentence names the destination;
//!    a bare context name is refused by name.
//! 4. **Type conversion** — `type_to` converts on MCP as `--type-to` does on the CLI, and an
//!    unknown type is refused by name on both doors.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

/// The harness principal's email (see `resources_parity_test.rs`).
const EMAIL: &str = "e2e@test.example.com";

fn input<T: for<'de> serde::Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

fn one_text(res: &rmcp::model::CallToolResult) -> Value {
    serde_json::from_str(res.content[0].as_text().expect("a text part").text.as_str())
        .expect("the part is the tool's JSON response")
}

async fn harness(
    pool: PgPool,
) -> (
    common::E2eTestApp,
    TemperMcpService,
    axum::http::request::Parts,
) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

async fn profile_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("the profile")
}

async fn default_context(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c JOIN kb_profiles p ON p.id = c.owner_id \
         WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("the default context")
}

/// A second context owned by `email`'s profile — a destination its owner can write.
async fn second_context(pool: &PgPool, email: &str, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, $3, $3)",
    )
    .bind(id)
    .bind(profile_id(pool, email).await)
    .bind(slug)
    .execute(pool)
    .await
    .expect("insert the destination context");
    id
}

/// A second approved identity with its own default context (as `resources_parity_test.rs`).
async fn second_identity(
    app: &common::E2eTestApp,
    tag: &str,
) -> (axum::http::request::Parts, String, String) {
    let unique = Uuid::new_v4();
    let email = format!("{tag}-{unique}@example.com");
    let token = common::generate_test_jwt(&format!("{tag}-sub-{unique}"), &email);
    let _ = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await;
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'approved' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
    )
    .bind(&email)
    .execute(&app.pool)
    .await
    .expect("approve the second identity");
    (app.relay_parts_for(&token), email, token)
}

/// Create a research resource with a body and both meta tiers set, through MCP.
async fn create(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    context: Uuid,
    title: &str,
) -> String {
    let res = temper_mcp::tools::resources::create_resource(
        svc,
        parts,
        input(json!({
            "context_ref": context.to_string(),
            "doc_type_name": "research",
            "title": title,
            "content": "A body the move must not touch.",
            "open_meta": { "tags": ["move-witness"], "claim_id": "c-1" },
        })),
    )
    .await
    .expect("create lands");
    one_text(&res)["resource"]["id"]
        .as_str()
        .expect("id")
        .to_string()
}

/// The resource as `get_resource` shows it, with its body.
async fn get(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    id: &str,
) -> (Value, String) {
    let res = temper_mcp::tools::resources::get_resource(
        svc,
        parts,
        input(json!({ "id": id, "include_content": true })),
    )
    .await
    .expect("get lands");
    let meta: Value =
        serde_json::from_str(&res.content[0].as_text().expect("text").text).expect("json");
    let body = res
        .content
        .get(1)
        .map(|c| c.as_text().expect("text").text.clone())
        .unwrap_or_default();
    (meta, body)
}

/// What a move must leave as it was, and what it changes, side by side.
fn stored_state(view: &Value, body: &str) -> Value {
    json!({
        "kb_context_id": view["kb_context_id"],
        "doc_type_name": view["doc_type_name"],
        "title": view["title"],
        "managed_meta": view["managed_meta"],
        "open_meta": view["open_meta"],
        "body": body,
    })
}

/// The event-type names emitted under one correlation id, sorted.
async fn events_under(pool: &PgPool, correlation: Uuid) -> Vec<String> {
    let mut names: Vec<String> = sqlx::query_scalar(
        "SELECT et.name FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE e.correlation_id = $1",
    )
    .bind(correlation)
    .fetch_all(pool)
    .await
    .expect("events");
    names.sort();
    names
}

async fn cli_ok(app: &common::E2eTestApp, args: &[&str]) {
    let out = common::run_temper_cli(app, args).await.expect("cli run");
    assert!(
        out.status.success(),
        "cli {args:?} failed: stderr={} stdout={}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

// ── 1. Re-home ──────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_context_to_rehomes_and_keeps_id_body_and_meta(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let home = default_context(&app.pool, EMAIL).await;
    let dest = second_context(&app.pool, EMAIL, "move-dest").await;
    let id = create(&svc, &parts, home, "Moved through MCP").await;
    let (before, body_before) = get(&svc, &parts, &id).await;
    assert_eq!(before["kb_context_id"], json!(home.to_string()));

    let moved = one_text(
        &temper_mcp::tools::resources::update_resource(
            &svc,
            &parts,
            input(json!({ "id": id, "context_to": "@me/move-dest" })),
        )
        .await
        .expect("the move lands"),
    );
    assert_eq!(moved["kb_context_id"], json!(dest.to_string()), "{moved}");

    let (after, body_after) = get(&svc, &parts, &id).await;
    assert_eq!(after["id"], json!(id), "the id is kept");
    assert_eq!(after["kb_context_id"], json!(dest.to_string()));
    assert!(
        after["context_ref"]
            .as_str()
            .is_some_and(|r| r.ends_with("/move-dest")),
        "get_resource shows the new context_ref: {after}"
    );
    assert_eq!(body_after, body_before, "the body is kept");
    assert_eq!(
        after["managed_meta"], before["managed_meta"],
        "managed tier kept"
    );
    assert_eq!(after["open_meta"], before["open_meta"], "open tier kept");
    assert_eq!(after["doc_type_name"], before["doc_type_name"]);
}

// ── 2. Door parity ──────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_same_move_through_cli_and_mcp_stores_the_same_thing(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let home = default_context(&app.pool, EMAIL).await;
    let dest = second_context(&app.pool, EMAIL, "parity-dest").await;
    let via_cli = create(&svc, &parts, home, "Parity subject").await;
    let via_mcp = create(&svc, &parts, home, "Parity subject").await;
    let cli_corr = Uuid::now_v7();
    let mcp_corr = Uuid::now_v7();

    let dest_ref = dest.to_string();
    let cli_corr_s = cli_corr.to_string();
    cli_ok(
        &app,
        &[
            "resource",
            "update",
            &via_cli,
            "--context-to",
            &dest_ref,
            "--correlation",
            &cli_corr_s,
        ],
    )
    .await;
    temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({ "id": via_mcp, "context_to": dest_ref, "correlation_id": mcp_corr })),
    )
    .await
    .expect("the MCP move lands");

    let (cli_view, cli_body) = get(&svc, &parts, &via_cli).await;
    let (mcp_view, mcp_body) = get(&svc, &parts, &via_mcp).await;
    assert_eq!(cli_view["kb_context_id"], json!(dest.to_string()));
    assert_eq!(
        stored_state(&cli_view, &cli_body),
        stored_state(&mcp_view, &mcp_body),
        "the two doors left the same stored state"
    );

    let cli_events = events_under(&app.pool, cli_corr).await;
    assert!(
        cli_events.iter().any(|n| n == "resource_rehomed"),
        "the CLI move re-homed: {cli_events:?}"
    );
    assert_eq!(
        cli_events,
        events_under(&app.pool, mcp_corr).await,
        "the two doors reached the same backend command"
    );
}

// ── 3. Refusals ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_readable_but_unwritable_destination_is_refused_naming_it(pool: PgPool) {
    let (app, svc, owner_parts) = harness(pool).await;
    let owner_ctx = default_context(&app.pool, EMAIL).await;
    let (other_parts, other_email, other_token) = second_identity(&app, "move-ro").await;
    let other_home = default_context(&app.pool, &other_email).await;
    let other_id = profile_id(&app.pool, &other_email).await;

    // The other identity may READ the owner's context, never write it.
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(owner_ctx)
    .bind(other_id)
    .bind(profile_id(&app.pool, EMAIL).await)
    .execute(&app.pool)
    .await
    .expect("read grant lands");
    let id = create(&svc, &other_parts, other_home, "Mine to move").await;

    // HTTP's answer to the same act: a 403.
    let http = app
        .reqwest_client
        .patch(app.url(&format!("/api/resources/{id}")))
        .bearer_auth(&other_token)
        .json(&json!({ "context_to": owner_ctx.to_string() }))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(http.status().as_u16(), 403, "HTTP refuses the destination");

    // MCP's: the kind this tool gives every 403 (the non-move face, for comparison) ...
    let not_modifiable = temper_mcp::tools::resources::update_resource(
        &svc,
        &owner_parts,
        input(json!({ "id": id, "title": "Not yours" })),
    )
    .await
    .expect_err("a non-writer is refused");
    let refused = temper_mcp::tools::resources::update_resource(
        &svc,
        &other_parts,
        input(json!({ "id": id, "context_to": owner_ctx.to_string() })),
    )
    .await
    .expect_err("an unwritable destination is refused");
    assert_eq!(
        refused.code, not_modifiable.code,
        "the same kind as any 403"
    );
    assert_eq!(refused.code.0, -32602);
    // ... and a sentence that names the destination as a possible cause.
    assert!(
        refused.message.contains("destination") && refused.message.contains(&owner_ctx.to_string()),
        "the refusal names the destination: {}",
        refused.message
    );
    assert!(
        refused.message.contains("not modifiable"),
        "and still names the resource: {}",
        refused.message
    );

    let (after, _) = get(&svc, &other_parts, &id).await;
    assert_eq!(
        after["kb_context_id"],
        json!(other_home.to_string()),
        "nothing moved"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_bare_context_name_is_refused_by_name(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let home = default_context(&app.pool, EMAIL).await;
    let id = create(&svc, &parts, home, "Bare name").await;

    let err = temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({ "id": id, "context_to": "knowledge" })),
    )
    .await
    .expect_err("a bare name is refused");
    assert_eq!(err.code.0, -32602);
    assert!(
        err.message.contains("bare names") && err.message.contains("\"knowledge\""),
        "the refusal names the input: {}",
        err.message
    );
}

// ── 4. Type conversion ──────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn type_to_converts_on_mcp_as_on_the_cli(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let home = default_context(&app.pool, EMAIL).await;
    let via_cli = create(&svc, &parts, home, "Convert me").await;
    let via_mcp = create(&svc, &parts, home, "Convert me").await;

    cli_ok(
        &app,
        &["resource", "update", &via_cli, "--type-to", "concept"],
    )
    .await;
    temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({ "id": via_mcp, "type_to": "concept" })),
    )
    .await
    .expect("the MCP conversion lands");

    let (cli_view, cli_body) = get(&svc, &parts, &via_cli).await;
    let (mcp_view, mcp_body) = get(&svc, &parts, &via_mcp).await;
    assert_eq!(mcp_view["doc_type_name"], "concept", "{mcp_view}");
    assert_eq!(
        stored_state(&cli_view, &cli_body),
        stored_state(&mcp_view, &mcp_body),
        "the two doors converted to the same stored state"
    );

    // An unknown type is refused by name on both doors, before anything is written.
    let err = temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({ "id": via_mcp, "type_to": "frobnicate" })),
    )
    .await
    .expect_err("an unknown type is refused");
    assert_eq!(err.code.0, -32602);
    assert!(
        err.message
            .starts_with("type_to: unknown doctype 'frobnicate'"),
        "{}",
        err.message
    );
    let out = common::run_temper_cli(
        &app,
        &["resource", "update", &via_cli, "--type-to", "frobnicate"],
    )
    .await
    .expect("cli run");
    assert!(!out.status.success(), "the CLI refuses it too");
    let (still, _) = get(&svc, &parts, &via_mcp).await;
    assert_eq!(still["doc_type_name"], "concept", "nothing was written");
}
