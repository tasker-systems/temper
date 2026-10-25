#![cfg(feature = "test-db")]

//! E2E: the batch read of one artifact ask answers alike at every door (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §3.5, §7.1, §8).
//!
//! One service read (`artifact_ask_read::ask_page`) behind three doors — `POST
//! /api/data-artifacts/ask`, MCP `ask_data_artifacts` (relayed to that route, unread) and `temper
//! data-artifact ask` (a client of it) — so the three answer one page, and refuse alike: an
//! unreadable shape exactly as an absent one, and every refusal at once, `teaches` intact, as
//! fields an agent reads without parsing English (`error.details.refusals` on HTTP, the error's
//! `data` on MCP, the error payload's `details` under the CLI's JSON format).
//!
//! **Local runs need a fresh binary**: the CLI door runs the built `temper`, so run `cargo build -p
//! temper-cli --bin temper` first (see `query_cli_e2e.rs`).

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_core::types::artifact_ask::AskResponse;
use temper_core::types::data_artifact::ArtifactCommitRequest;
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_mcp::tools::data_artifacts::{ask_artifacts, AskInput};
use temper_workflow::types::resource::ResourceCreateRequest;
use uuid::Uuid;

/// The owner's world, through the real API: a context, a resource in it, the `measurement` shape
/// declared there (`unit` a string), one artifact that matches `unit` contains `"ms"` and one
/// committed with no content; and a second principal who cannot read the context.
struct World {
    app: common::E2eTestApp,
    svc: temper_mcp::service::TemperMcpService,
    other_token: String,
    shape: Uuid,
    contentless: Uuid,
    config_path: std::path::PathBuf,
}

async fn world(pool: PgPool) -> World {
    let app = common::setup_relay(pool).await;
    let context = app
        .client
        .contexts()
        .create("e2e-artifact-ask", None)
        .await
        .expect("context create");
    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/artifact-ask".to_string(),
            title: "artifact-ask subject".to_string(),
            act: Default::default(),
        })
        .await
        .expect("resource create");
    let declared = app
        .client
        .data_artifacts()
        .declare_shape(
            *context.id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": { "unit": { "type": "string" } }
                }),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .expect("declare");
    let mut committed = Vec::new();
    for content in [json!({ "unit": "ms" }), Value::Null] {
        let out = app
            .client
            .data_artifacts()
            .commit(
                resource.id.into(),
                &ArtifactCommitRequest {
                    kind: "measurement".to_string(),
                    kind_owner: None,
                    intent: "member".to_string(),
                    precedence: 0.0,
                    content,
                    supersedes: Vec::new(),
                    act: Default::default(),
                },
            )
            .await
            .expect("commit");
        committed.push(out.artifact_id.uuid());
    }
    common::provision_and_approve_second(&app).await;
    let svc = app.mcp_relay_service().await;
    let config_path = app.vault_dir.path().join("artifact-ask-config.toml");
    std::fs::write(
        &config_path,
        toml::to_string(&app.config).expect("the test config serializes"),
    )
    .expect("write the test config");
    World {
        app,
        svc,
        other_token: common::generate_second_user_jwt(),
        shape: declared.shape_id.uuid(),
        contentless: committed[1],
        config_path,
    }
}

/// The ask `unit` contains `"ms"` of `shape`.
fn unit_ms(shape: Uuid) -> Value {
    json!({ "shape": shape, "predicates": [
        { "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } }
    ] })
}

/// The HTTP door: the status and the raw body, parsed untyped so a field absent on the wire is
/// seen absent.
async fn api(w: &World, token: &str, body: &Value) -> (u16, Value) {
    let resp = w
        .app
        .reqwest_client
        .post(w.app.url("/api/data-artifacts/ask"))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .expect("POST the ask");
    let status = resp.status().as_u16();
    (status, resp.json().await.expect("a JSON body"))
}

/// The MCP door: the tool's one text block parsed, or its error.
async fn mcp(w: &World, token: &str, body: &Value) -> Result<Value, rmcp::ErrorData> {
    let result = ask_artifacts(
        &w.svc,
        &w.app.relay_parts_for(token),
        AskInput(body.clone()),
    )
    .await?;
    assert_eq!(result.content.len(), 1, "one text block");
    assert!(result.structured_content.is_none(), "sent once, as text");
    match &result.content[0] {
        rmcp::model::ContentBlock::Text(t) => {
            assert!(!t.text.contains('\n'), "compact, never pretty-printed");
            Ok(serde_json::from_str(&t.text).expect("the text is the page's JSON"))
        }
        other => panic!("a text block, got {other:?}"),
    }
}

/// The CLI door, `--ask` and `extra` flags, under the JSON format: whether it succeeded, and its
/// stdout as one JSON document (the page, or the error payload).
async fn cli(w: &World, token: &str, ask: &Value, extra: &[&str]) -> (bool, Value) {
    let ask = ask.to_string();
    let mut args = vec![
        "data-artifact",
        "ask",
        "--ask",
        ask.as_str(),
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    let out = common::run_temper_cli_with_token(&w.app.base_url(), token, &w.config_path, &args)
        .await
        .expect("spawn temper data-artifact ask");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "stdout is one JSON document ({e}): {stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.success(), parsed)
}

/// The reasons of a refusal list, in order.
fn reasons(refusals: &Value) -> Vec<String> {
    refusals
        .as_array()
        .unwrap_or_else(|| panic!("a refusal list: {refusals}"))
        .iter()
        .map(|r| r["reason"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// **A shape the asker cannot read and a shape that does not exist answer identically, at every
/// door**: the same HTTP status and, byte for byte, the same body; the same MCP message and
/// `data`; the same CLI exit and JSON payload. The second principal cannot read the owner's
/// context, so the owner's shape is unreadable to them; a fresh uuid names nothing. Each is one
/// refusal, `shape_not_askable`, naming no shape. The owner's ask, the control, is answered.
/// Bite: a door that answers an unreadable shape differently — a `404`, or another reason.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unreadable_shape_is_refused_as_an_absent_one_at_every_door(pool: PgPool) {
    let w = world(pool).await;
    let unreadable = unit_ms(w.shape);
    let absent = unit_ms(Uuid::now_v7());

    let (owner, _) = api(&w, &w.app.token, &json!({ "ask": unreadable })).await;
    assert_eq!(owner, 200, "the control: the owner may ask");

    let (u_status, u_body) = api(&w, &w.other_token, &json!({ "ask": unreadable })).await;
    let (a_status, a_body) = api(&w, &w.other_token, &json!({ "ask": absent })).await;
    assert_eq!(u_status, 400, "{u_body}");
    assert_eq!((a_status, &a_body), (u_status, &u_body), "HTTP: one answer");
    assert_eq!(
        reasons(&u_body["error"]["details"]["refusals"]),
        ["shape_not_askable"]
    );
    assert!(!u_body.to_string().contains(&w.shape.to_string()));

    let u_mcp = mcp(&w, &w.other_token, &json!({ "ask": unreadable }))
        .await
        .expect_err("refused over MCP");
    let a_mcp = mcp(&w, &w.other_token, &json!({ "ask": absent }))
        .await
        .expect_err("refused over MCP");
    assert_eq!(u_mcp.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert_eq!(
        (&a_mcp.message, &a_mcp.data),
        (&u_mcp.message, &u_mcp.data),
        "MCP: one answer"
    );
    let u_data = u_mcp.data.expect("the refusals ride data");
    assert_eq!(reasons(&u_data["refusals"]), ["shape_not_askable"]);
    assert_eq!(u_data["refusals"], u_body["error"]["details"]["refusals"]);

    let (u_ok, u_cli) = cli(&w, &w.other_token, &unreadable, &[]).await;
    let (a_ok, a_cli) = cli(&w, &w.other_token, &absent, &[]).await;
    assert!(!u_ok, "a refused ask exits non-zero: {u_cli}");
    assert_eq!((a_ok, &a_cli), (u_ok, &u_cli), "CLI: one answer");
    assert_eq!(
        u_cli["details"]["refusals"],
        u_body["error"]["details"]["refusals"]
    );
}

/// **A paging refusal and an ask refusal in one request arrive together, `teaches` intact, as
/// fields at every door.** The owner asks a path the shape does not declare (`path_outside_shape`,
/// which teaches the shape's askable paths) with `limit` 0 (`page_limit_out_of_range`): both
/// refusals, the ask's first, none with a stage — in HTTP's `error.details.refusals`, MCP's `data`
/// and the CLI's JSON `details`, the three lists equal. Neither the MCP message nor the CLI
/// message says "plan". Bite: the MCP error's `data` passed as `None`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_paging_and_an_ask_refusal_arrive_together_with_teaches_at_every_door(pool: PgPool) {
    let w = world(pool).await;
    let outside = json!({ "shape": w.shape, "predicates": [
        { "path": ["nope"], "op": { "op": "has_key" } }
    ] });
    let body = json!({ "ask": outside, "limit": 0 });
    let expected = ["path_outside_shape", "page_limit_out_of_range"];

    let (status, http) = api(&w, &w.app.token, &body).await;
    assert_eq!(status, 400, "{http}");
    let refusals = &http["error"]["details"]["refusals"];
    assert_eq!(reasons(refusals), expected);
    assert!(
        refusals[0]["teaches"]["vocabulary"].is_object(),
        "the ask's refusal teaches the shape's vocabulary: {refusals}"
    );
    for r in refusals.as_array().expect("a list") {
        assert!(r.get("stage").is_none_or(Value::is_null), "no stage: {r}");
    }

    let err = mcp(&w, &w.app.token, &body)
        .await
        .expect_err("refused over MCP");
    assert!(!err.message.contains("plan"), "{}", err.message);
    let data = err.data.expect("the refusals ride data, teaches and all");
    assert_eq!(&data["refusals"], refusals, "MCP carries the HTTP list");

    let (ok, payload) = cli(&w, &w.app.token, &outside, &["--limit", "0"]).await;
    assert!(!ok, "a refused ask exits non-zero: {payload}");
    assert_eq!(
        &payload["details"]["refusals"], refusals,
        "the CLI carries the HTTP list"
    );
    let message = payload["message"].as_str().expect("a message");
    assert!(
        message.contains("the ask will not be answered (2 refusal(s))")
            && !message.contains("plan"),
        "{message}"
    );
}

/// **The three doors answer one page, and the MCP edge relays the ask unread.** The owner's ask
/// with no predicates, over HTTP, over MCP and through the CLI: the three bodies are one JSON
/// document, and it reads back as an `AskResponse`, as does the typed client's answer. The
/// content-less artifact is served without a `content` field on every door. An ask carrying a field
/// the request does not declare is refused by the API's reader, in its words, over MCP — the edge
/// does not read it first. Bites: the tool deserializing `AskRequest` itself (the unknown field is
/// refused at the edge, in serde's words, not the API's), or a door that writes `content: null`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_doors_answer_one_page_and_the_edge_relays_the_ask_unread(pool: PgPool) {
    let w = world(pool).await;
    let everything = json!({ "shape": w.shape });
    let body = json!({ "ask": everything });

    let (status, http) = api(&w, &w.app.token, &body).await;
    assert_eq!(status, 200, "{http}");
    let over_mcp = mcp(&w, &w.app.token, &body)
        .await
        .expect("answered over MCP");
    let (ok, over_cli) = cli(&w, &w.app.token, &everything, &[]).await;
    assert!(ok, "{over_cli}");
    assert_eq!(over_mcp, http, "MCP answers the HTTP page");
    assert_eq!(over_cli, http, "the CLI answers the HTTP page");

    let typed: AskResponse = serde_json::from_value(http.clone()).expect("an AskResponse");
    assert_eq!(
        typed.artifacts.len(),
        2,
        "both artifacts, with no predicates"
    );
    let client_page = w
        .app
        .client
        .data_artifacts()
        .ask(&body)
        .await
        .expect("the typed client reads the page");
    assert_eq!(
        serde_json::to_value(&client_page).expect("serializes"),
        http
    );

    for page in [&http, &over_mcp, &over_cli] {
        let row = page["artifacts"]
            .as_array()
            .expect("artifacts")
            .iter()
            .find(|r| r["artifact_id"] == w.contentless.to_string())
            .expect("the content-less artifact is served");
        assert!(row.get("content").is_none(), "absent, not null: {row}");
    }

    let unknown = json!({ "ask": everything, "order": "desc" });
    let err = mcp(&w, &w.app.token, &unknown)
        .await
        .expect_err("the API cannot read it");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        err.message.contains("ask could not be read") && err.message.contains("order"),
        "the API's reader refused it, in its words: {}",
        err.message
    );
}
