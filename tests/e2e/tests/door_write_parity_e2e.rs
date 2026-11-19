#![cfg(feature = "test-db")]
//! The same resource write stores the same thing through the CLI and through MCP.
//!
//! Both doors shape their requests with `temper_workflow::shaping`, which owns every default a
//! write applies, and the server judges the result. So one set of inputs, sent once through each
//! door, must leave two resources that differ only in their ids. Each case below writes through
//! both doors against this process's real server and compares what was stored:
//!
//! - a create with no body stores no body, through either door (the CLI used to store
//!   `# {title}`);
//! - a create with no origin stores an empty one (MCP used to store `mcp://agent/<uuid>`);
//! - a create's managed and open tiers land the same;
//! - an update converting to a doc type no schema knows lands the same (both doors used to
//!   refuse it before sending; the server always took it).
//!
//! The CLI half runs the real `temper` binary: build it first (`cargo build -p temper-cli --bin
//! temper`). The MCP half drives the tool functions through the deployed relay.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;

use common::E2eTestApp;

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

/// Run the CLI, assert it succeeded, and parse its one JSON document.
async fn cli_json(app: &E2eTestApp, args: &[&str]) -> Value {
    let output = common::run_temper_cli(app, args).await.expect("cli run");
    assert!(
        output.status.success(),
        "cli {args:?} failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON document")
}

/// The resource id in a CLI or MCP write answer.
fn written_id(answer: &Value) -> String {
    answer["resource"]["id"]
        .as_str()
        .or_else(|| answer["id"].as_str())
        .unwrap_or_else(|| panic!("no resource id in {answer}"))
        .to_string()
}

/// What a write stored: everything but the resource's identity and timestamps.
async fn stored(app: &E2eTestApp, id: &str) -> Value {
    let uuid: uuid::Uuid = id.parse().expect("a uuid");
    let sections: temper_core::types::resource_view::SectionSet =
        [temper_core::types::resource_view::ResourceSection::OpenMeta]
            .into_iter()
            .collect();
    let view = app
        .client
        .resources()
        .get(uuid, Some(&sections))
        .await
        .expect("read back");
    let body = app
        .client
        .resources()
        .content(uuid)
        .await
        .expect("content")
        .markdown;
    json!({
        "title": view.title,
        "doc_type_name": view.doc_type_name,
        "origin_uri": view.origin_uri,
        "context_ref": view.context_ref,
        "managed_meta": view.managed_meta,
        "open_meta": view.open_meta,
        "body": body,
    })
}

async fn mcp_create(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    args: Value,
) -> String {
    written_id(&one_text(
        &temper_mcp::tools::resources::create_resource(svc, parts, input(args))
            .await
            .expect("mcp create lands"),
    ))
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_bodyless_create_stores_the_same_through_both_doors(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;

    let via_cli = written_id(
        &cli_json(
            &app,
            &[
                "resource",
                "create",
                "--type",
                "research",
                "--title",
                "Door parity, no body",
                "--context",
                "@me/default",
                "--format",
                "json",
            ],
        )
        .await,
    );
    let via_mcp = mcp_create(
        &svc,
        &parts,
        json!({
            "context_ref": "@me/default",
            "doc_type_name": "research",
            "title": "Door parity, no body",
        }),
    )
    .await;

    let (cli, mcp) = (stored(&app, &via_cli).await, stored(&app, &via_mcp).await);
    assert_eq!(cli, mcp, "the same create stored differently by door");
    assert_eq!(cli["body"], "", "a create with no body stores no body");
    assert_eq!(cli["origin_uri"], "", "a create with no origin stores none");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_create_with_both_meta_tiers_stores_the_same_through_both_doors(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;

    let via_cli = written_id(
        &cli_json(
            &app,
            &[
                "resource",
                "create",
                "--type",
                "task",
                "--title",
                "Door parity, meta",
                "--context",
                "@me/default",
                "--mode",
                "plan",
                "--effort",
                "small",
                "--open-meta",
                r#"{"tags":["parity"],"descriptor":"both doors"}"#,
                "--format",
                "json",
            ],
        )
        .await,
    );
    let via_mcp = mcp_create(
        &svc,
        &parts,
        json!({
            "context_ref": "@me/default",
            "doc_type_name": "task",
            "title": "Door parity, meta",
            "managed_meta": {"temper-mode": "plan", "temper-effort": "small"},
            "open_meta": {"tags": ["parity"], "descriptor": "both doors"},
        }),
    )
    .await;

    let (cli, mcp) = (stored(&app, &via_cli).await, stored(&app, &via_mcp).await);
    assert_eq!(cli, mcp, "the same create stored differently by door");
    assert_eq!(cli["managed_meta"]["temper-mode"], "plan");
    assert_eq!(cli["open_meta"]["tags"], json!(["parity"]));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_update_to_an_unrecognized_doc_type_stores_the_same_through_both_doors(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let seed = json!({
        "context_ref": "@me/default",
        "doc_type_name": "research",
        "title": "Door parity, update",
    });
    let a = mcp_create(&svc, &parts, seed.clone()).await;
    let b = mcp_create(&svc, &parts, seed).await;

    cli_json(
        &app,
        &[
            "resource",
            "update",
            &a,
            "--title",
            "Door parity, converted",
            "--type-to",
            "field-note",
            "--format",
            "json",
        ],
    )
    .await;
    temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({
            "id": b,
            "title": "Door parity, converted",
            "type_to": "field-note",
        })),
    )
    .await
    .expect("mcp update lands");

    let (cli, mcp) = (stored(&app, &a).await, stored(&app, &b).await);
    assert_eq!(cli, mcp, "the same update stored differently by door");
    assert_eq!(cli["doc_type_name"], "field-note");
}
