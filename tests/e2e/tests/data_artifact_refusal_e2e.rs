#![cfg(feature = "test-db")]

//! E2E: the data-artifact refusal surface — both defects of the refusal-surface pair,
//! exercised over BOTH doors (typed HTTP client and the in-process MCP service).
//!
//! Two defects, kept separable in the witnesses:
//!
//! 1. **The dropped `kind_owner`.** Naming `kind_owner` explicitly is accepted by every
//!    surface and dropped before the SQL layer, so an empty context is undeclarable. The
//!    witnesses assert a declare with an explicit owner SUCCEEDS on an empty context and
//!    that the stored shape carries the named namespace.
//! 2. **The 500 mapping.** SQL refusals (declare) and enforcing-commit refusals surface as
//!    `server error (500)` with the generic internal body, so the refusal teaches nothing.
//!    The witnesses assert the refusal reaches the caller carrying its own vocabulary —
//!    the SQL wrapper's words, or the per-violation detail — and never as a 500.
//!
//! The enforcing witnesses seed a context WITH a resource so the defaulting arm works
//! regardless of defect 1 — defect 2 is isolated. The empty-context witnesses isolate
//! defect 1. Both doors means both: a fix that only reaches one door leaves the identical
//! refusal live on the other.

mod common;

// The counting loopback listener shared with temper-substrate's tests, so every suite agrees on
// what a connection is.
#[path = "../../../crates/temper-substrate/tests/common/fetch_probe.rs"]
mod fetch_probe;

use fetch_probe::FetchProbe;
use serde_json::json;
use sqlx::PgPool;
use temper_client::error::ClientError;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactListParams, KindOwnerInput,
};
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_workflow::types::resource::ResourceCreateRequest;

/// A minimal enforcing-able schema: an object with a required string `value`.
fn string_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "value": { "type": "string" } },
        "required": ["value"]
    })
}

/// The refusal the SQL declare wrapper raises on an empty context with no owner named —
/// its vocabulary is the deliverable (the designed-in refusal is not the defect).
const EMPTY_CONTEXT_VOCAB: &str = "name kind_owner explicitly";

/// Extract the text of the first content part of a tool result.
fn text_of(result: rmcp::model::CallToolResult) -> String {
    match result.content.first() {
        Some(rmcp::model::ContentBlock::Text(t)) => t.text.clone(),
        other => panic!("tool returned no text content part: {other:?}"),
    }
}

/// The beat-G4 relay: a service pointed at the app's real listener plus the
/// caller's relay parts. Every declare call forwards to the door — the direct
/// binding's pair (`mcp_service`) is retired.
async fn relay_service_and_parts(
    app: &common::E2eTestApp,
) -> (
    temper_mcp::service::TemperMcpService,
    axum::http::request::Parts,
) {
    let svc = app.mcp_relay_service().await;
    (svc, app.relay_parts())
}

/// A context holding one resource — the minimal world where the defaulting arm resolves.
async fn context_with_resource(app: &common::E2eTestApp, slug: &str) -> (uuid::Uuid, uuid::Uuid) {
    let context = app
        .client
        .contexts()
        .create(slug, None)
        .await
        .expect("context create failed");
    let resource = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            idempotency_key: None,
            doc_type: "research".to_string(),
            origin_uri: format!("test://e2e/{slug}"),
            title: format!("{slug} subject"),
            act: Default::default(),
        })
        .await
        .expect("resource create failed");
    (*context.id, *resource.id)
}

// ── Defect 1: the dropped kind_owner ─────────────────────────────────────────

/// An explicit `kind_owner` on an EMPTY context must reach the SQL layer: the declare
/// succeeds and the stored shape carries the named namespace. The drop sat between every
/// surface and `data_artifact_shape_declare`'s `kind_owner_id` defaulting arm.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn explicit_kind_owner_declares_on_an_empty_context_over_http(pool: PgPool) {
    let app = common::setup(pool).await;
    let profile = app.client.profile().get().await.expect("profile").id;

    let context = app
        .client
        .contexts()
        .create("e2e-refusal-empty-http", None)
        .await
        .expect("context create failed");

    let shape = app
        .client
        .data_artifacts()
        .declare_shape(
            *context.id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: Some(KindOwnerInput::Profile(profile)),
                schema: string_schema(),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .expect("an explicitly-owned declare must succeed on an empty context");

    assert_eq!(shape.kind_owner_table, "kb_profiles");
    assert_eq!(shape.kind_owner_id, profile);
}

/// Same assertion through the MCP door — the parameter the tool accepts must not be
/// dropped between the tool input and the SQL wrapper.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn explicit_kind_owner_declares_on_an_empty_context_over_mcp(pool: PgPool) {
    let app = common::setup_relay(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("profile").id;
    let context = app
        .client
        .contexts()
        .create("e2e-refusal-empty-mcp", None)
        .await
        .expect("context create failed");
    let (svc, parts) = relay_service_and_parts(&app).await;

    let result = temper_mcp::tools::data_artifact_shapes::declare_shape(
        &svc,
        &parts,
        temper_mcp::tools::data_artifact_shapes::DeclareShapeInput {
            home_type: "context".to_string(),
            home_id: context.id.to_string(),
            kind: "measurement".to_string(),
            kind_owner: Some(KindOwnerInput::Profile(profile)),
            schema: string_schema(),
            enforcement: EnforcementMode::Advisory,
            invocation_id: None,
            correlation_id: None,
            confidence: None,
            reasoning: None,
            rationale: None,
            persona: None,
            model: None,
        },
    )
    .await
    .expect("an explicitly-owned declare must succeed on an empty context over MCP");

    let view: serde_json::Value =
        serde_json::from_str(&text_of(result)).expect("tool result is ShapeView JSON");
    assert_eq!(view["kind_owner_table"], "kb_profiles");
    assert_eq!(view["kind_owner_id"], profile.to_string());
}

// ── Defect 2: refusals surface as 500s ───────────────────────────────────────

/// The designed-in empty-context refusal must reach the HTTP caller as the refusal text —
/// the wrapper's own words — never as a 500 with the generic internal body.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sql_refusal_carries_its_vocabulary_over_http(pool: PgPool) {
    let app = common::setup(pool).await;

    let context = app
        .client
        .contexts()
        .create("e2e-refusal-vocab-http", None)
        .await
        .expect("context create failed");

    let err = app
        .client
        .data_artifacts()
        .declare_shape(
            *context.id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: string_schema(),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .expect_err("an empty context with no kind_owner must refuse");

    // TYPED, not just "not a 500": the client discriminates the refusal by wire code and
    // carries the wrapper's words verbatim.
    let refusal = match err {
        ClientError::DataArtifactRefusal { message } => message,
        other => panic!("the SQL refusal must arrive typed, not as {other}"),
    };
    assert!(
        refusal.contains(EMPTY_CONTEXT_VOCAB),
        "the refusal must teach its vocabulary, got: {refusal}"
    );
}

/// The same refusal through the MCP door: the tool must surface the wrapper's words,
/// not an internal-error envelope around them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sql_refusal_carries_its_vocabulary_over_mcp(pool: PgPool) {
    let app = common::setup_relay(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let context = app
        .client
        .contexts()
        .create("e2e-refusal-vocab-mcp", None)
        .await
        .expect("context create failed");
    let (svc, parts) = relay_service_and_parts(&app).await;

    let err = temper_mcp::tools::data_artifact_shapes::declare_shape(
        &svc,
        &parts,
        temper_mcp::tools::data_artifact_shapes::DeclareShapeInput {
            home_type: "context".to_string(),
            home_id: context.id.to_string(),
            kind: "measurement".to_string(),
            kind_owner: None,
            schema: string_schema(),
            enforcement: EnforcementMode::Advisory,
            invocation_id: None,
            correlation_id: None,
            confidence: None,
            reasoning: None,
            rationale: None,
            persona: None,
            model: None,
        },
    )
    .await
    .expect_err("an empty context with no kind_owner must refuse over MCP");

    // INTERNAL_ERROR (-32603) is the leak the old mapping produced: the generic internal
    // envelope with the raw database text spliced into it. The refusal must travel as a
    // caller-side error (INVALID_PARAMS = -32602), carrying the wrapper's words.
    assert!(
        err.code.0 != -32603,
        "the refusal must not arrive as an internal error — that is the leak, not the fix: {err:?}"
    );
    assert!(
        err.message.contains(EMPTY_CONTEXT_VOCAB),
        "the refusal must teach its vocabulary, got: {}",
        err.message
    );
}

/// A schema-violating commit under an ENFORCING shape is refused atomically and
/// correctly — the caller must see the refusal naming the violations, never a 500.
/// The context holds a resource, so the defaulting arm resolves regardless of the
/// dropped-parameter defect; this witness isolates the error mapping.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enforcing_refusal_names_the_violations_over_http(pool: PgPool) {
    let app = common::setup(pool).await;
    let (context_id, resource_id) = context_with_resource(&app, "e2e-enforce-http").await;

    app.client
        .data_artifacts()
        .declare_shape(
            context_id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: string_schema(),
                enforcement: EnforcementMode::Enforcing,
                act: Default::default(),
            },
        )
        .await
        .expect("declaring an enforcing shape in a context with a resource must succeed");

    let err = app
        .client
        .data_artifacts()
        .commit(
            resource_id,
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: json!({ "value": 42 }),
                supersedes: Vec::new(),
                act: Default::default(),
            },
        )
        .await
        .expect_err("a non-conforming commit under an enforcing shape must refuse");

    // TYPED, not just "not a 500": the refusal names the conformance failure and the
    // violating location, carried through the code-discriminated client variant.
    let refusal = match err {
        ClientError::DataArtifactRefusal { message } => message,
        other => panic!("the enforcing refusal must arrive typed, not as {other}"),
    };
    assert!(
        refusal.contains("does not conform"),
        "the refusal must name the conformance failure, got: {refusal}"
    );
    assert!(
        refusal.contains("value"),
        "the refusal must name the violating location, got: {refusal}"
    );

    // The refusal is atomic: nothing was recorded.
    let list = app
        .client
        .data_artifacts()
        .list_all(
            resource_id,
            &ArtifactListParams {
                kind: Some("measurement".to_string()),
                include_folded: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("list after refusal");
    assert!(
        list.is_empty(),
        "a refused commit must record nothing, got: {list:?}"
    );
}

/// A commit into an enforcing family whose stored shape is not a field tree (declared before
/// declare compiled, and stored here through the substrate write, which does not compile) is
/// refused naming what the compile refused first — the committer owns the context, so it can read
/// the shape (shape ruling R4). The detail rides beside the substrate's fixed message and only the
/// API door, which knows the reader, names it: bites a door that drops it, or names it unasked.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_uncompilable_enforcing_shape_refusal_names_what_failed_to_its_reader_over_http(
    pool: PgPool,
) {
    let app = common::setup(pool).await;
    let (context_id, resource_id) = context_with_resource(&app, "e2e-uncompilable-commit").await;
    let owner: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the harness profile");
    let emitter: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e2e-shape-emitter') RETURNING id",
    )
    .bind(owner)
    .fetch_one(&app.pool)
    .await
    .expect("an emitter for the substrate write");
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "value": { "anyOf": [{ "type": "string" }] } }
    });
    temper_substrate::writes::declare_shape(
        &app.pool,
        temper_substrate::writes::DeclareShapeParams {
            home: temper_substrate::payloads::AnchorRef::context(context_id.into()),
            kind: "measurement",
            kind_owner: None,
            schema: &schema,
            enforcement: temper_substrate::payloads::EnforcementMode::Enforcing,
            emitter: emitter.into(),
        },
    )
    .await
    .expect("the substrate write stores a schema declare would refuse");

    let err = app
        .client
        .data_artifacts()
        .commit(
            resource_id,
            &ArtifactCommitRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                intent: "current".to_string(),
                precedence: 0.0,
                content: json!({ "value": "x" }),
                supersedes: Vec::new(),
                act: Default::default(),
            },
        )
        .await
        .expect_err("an enforcing family whose shape cannot compile refuses the commit");
    let refusal = match err {
        ClientError::DataArtifactRefusal { message } => message,
        other => panic!("the refusal must arrive typed, not as {other}"),
    };
    assert_eq!(
        refusal,
        temper_substrate::writes::uncompilable_shape_message(Some(
            "keyword anyOf at /properties/value"
        )),
    );
}

/// The enforcing refusal through the MCP commit door — `map_api_err` must not wrap it
/// in an internal-error envelope either.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enforcing_refusal_names_the_violations_over_mcp(pool: PgPool) {
    let app = common::setup_relay(pool.clone()).await;
    let (context_id, resource_id) = context_with_resource(&app, "e2e-enforce-mcp").await;

    app.client
        .data_artifacts()
        .declare_shape(
            context_id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: string_schema(),
                enforcement: EnforcementMode::Enforcing,
                act: Default::default(),
            },
        )
        .await
        .expect("declaring an enforcing shape in a context with a resource must succeed");
    let (svc, parts) = relay_service_and_parts(&app).await;

    let err = temper_mcp::tools::data_artifacts::commit_artifact(
        &svc,
        &parts,
        temper_mcp::tools::data_artifacts::CommitArtifactInput {
            resource_id: resource_id.to_string(),
            kind: "measurement".to_string(),
            kind_owner: None,
            intent: "current".to_string(),
            precedence: 0.0,
            content: json!({ "value": 42 }),
            supersedes: vec![],
            invocation_id: None,
            correlation_id: None,
            confidence: None,
            reasoning: None,
            rationale: None,
            persona: None,
            model: None,
        },
    )
    .await
    .expect_err("a non-conforming commit under an enforcing shape must refuse over MCP");

    // Same leak-pinning as the declare witness: INTERNAL_ERROR is the old mapping's
    // envelope, not the refusal.
    assert!(
        err.code.0 != -32603,
        "the refusal must not arrive as an internal error — that is the leak, not the fix: {err:?}"
    );
    assert!(
        err.message.contains("does not conform"),
        "the refusal must name the conformance failure, got: {}",
        err.message
    );
}

/// The CLI door: a bare declare on an empty context refuses with the wrapper's vocabulary
/// in the error output, and `--kind-owner kb_profiles:<uuid>` declares on the same empty
/// context — the flag is what makes an empty context declarable from the CLI. The spawned
/// process runs with non-TTY stdout, so JSON output is the default on both runs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_declare_refuses_with_vocabulary_then_succeeds_with_the_flag(pool: PgPool) {
    let app = common::setup(pool).await;
    let profile = app.client.profile().get().await.expect("profile").id;
    let context = app
        .client
        .contexts()
        .create("e2e-refusal-cli", None)
        .await
        .expect("context create failed");
    let schema = string_schema().to_string();
    let context_ref = context.id.to_string();

    let output = common::run_temper_cli_with_stdin(
        &app,
        &schema,
        &[
            "data-artifact",
            "schema",
            "declare",
            &context_ref,
            "--kind",
            "measurement",
            "--content",
            "-",
        ],
    )
    .await
    .expect("cli run");
    assert!(
        !output.status.success(),
        "a bare declare on an empty context must fail: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains(EMPTY_CONTEXT_VOCAB),
        "the CLI must surface the refusal's vocabulary, got: {combined}"
    );

    let output = common::run_temper_cli_with_stdin(
        &app,
        &schema,
        &[
            "data-artifact",
            "schema",
            "declare",
            &context_ref,
            "--kind",
            "measurement",
            "--kind-owner",
            &format!("kb_profiles:{profile}"),
            "--content",
            "-",
        ],
    )
    .await
    .expect("cli run");
    assert!(
        output.status.success(),
        "the explicit-owner declare must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("kb_profiles"),
        "the declared shape must carry the named namespace, got: {stdout}"
    );
}

// ── Declare-time compile: a schema that needs a fetch is refused at every door ──────────────
//
// A shape's schema is compiled when it is declared, with a compiler that never fetches. A schema
// whose `$ref` names a host is refused with one fixed sentence that names neither the host nor the
// compiler's text, no `kb_data_artifact_shapes` row is written, and the listener the `$ref` names
// sees no connection. The connection count guards against a regression and is not the fetch
// witness: the server runs in-process on an async runtime, where a fetching compiler's blocking
// fetch panics before it connects. The witnesses are the synchronous `shape_schema_no_fetch` tests
// in temper-substrate and the `audit-shape-schema-compile.sh` tripwire; these tests pin that each
// door refuses with the fixed sentence and writes no row.

/// The fixed sentence's distinctive words — what a declarer is told to change.
const UNCOMPILABLE_VOCAB: &str = "field tree";

/// The schema under test: valid JSON Schema whose `$ref` leaves the document for the probe's host.
fn remote_ref_schema(probe: &FetchProbe) -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": probe.url("shape.json"),
    })
}

/// Rows in the shape registry, read directly: the witness that a refused declare wrote nothing.
async fn shape_row_count(app: &common::E2eTestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_shapes")
        .fetch_one(&app.pool)
        .await
        .expect("count shape rows")
}

/// The refusal text teaches the sentence and echoes nothing of the URI.
fn assert_fixed_sentence(text: &str, probe: &FetchProbe) {
    assert!(
        text.contains(UNCOMPILABLE_VOCAB),
        "the refusal must carry the fixed sentence, got: {text}"
    );
    // The probe's host AND port: the CLI's output may legitimately name the API's own address.
    assert!(
        !text.contains(&format!("127.0.0.1:{}", probe.port())),
        "the refusal must not echo the URI the schema names, got: {text}"
    );
}

/// The context declare route (`POST /api/contexts/{id}/shapes`): typed refusal, nothing written.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_uncompilable_schema_is_refused_at_declare_over_http(pool: PgPool) {
    let app = common::setup(pool).await;
    let probe = FetchProbe::bind();
    let (context_id, _resource) = context_with_resource(&app, "e2e-uncompilable-http").await;
    let rows_before = shape_row_count(&app).await;

    let err = app
        .client
        .data_artifacts()
        .declare_shape(
            context_id,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: remote_ref_schema(&probe),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .expect_err("a schema that needs a fetch must be refused at declare");

    let message = match err {
        ClientError::DataArtifactRefusal { message } => message,
        other => panic!("the refusal must arrive typed (a 400), not as {other}"),
    };
    assert_fixed_sentence(&message, &probe);
    assert!(
        message.contains("(keyword $ref at the root)"),
        "the declarer is told the first keyword refused and where, got: {message}"
    );
    assert_eq!(
        shape_row_count(&app).await,
        rows_before,
        "a refused declare writes no shape row"
    );
    assert_eq!(probe.connections(), 0, "the declare must not fetch");
}

/// The cogmap declare route (`POST /api/cognitive-maps/{id}/shapes`) is the twin of the context
/// route and refuses the same schema the same way.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_uncompilable_schema_is_refused_at_declare_over_the_cogmap_route(pool: PgPool) {
    /// The L0 kernel cognitive map reserved id; the harness profile is made a system admin to author it.
    const L0_COGMAP: uuid::Uuid = uuid::Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

    let app = common::setup(pool).await;
    let owner_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("the harness profile");
    // L0 is in the admin-only regime: only a system admin authors it, and a write grant there
    // confers nothing (20261113000010).
    common::make_system_admin(&app.pool, owner_id).await;
    let probe = FetchProbe::bind();
    let rows_before = shape_row_count(&app).await;

    let err = app
        .client
        .data_artifacts()
        .declare_cogmap_shape(
            L0_COGMAP,
            &ShapeDeclareRequest {
                kind: "measurement".to_string(),
                kind_owner: None,
                schema: remote_ref_schema(&probe),
                enforcement: EnforcementMode::Advisory,
                act: Default::default(),
            },
        )
        .await
        .expect_err("a schema that needs a fetch must be refused on the cogmap route too");

    let message = match err {
        ClientError::DataArtifactRefusal { message } => message,
        other => panic!("the refusal must arrive typed (a 400), not as {other}"),
    };
    assert_fixed_sentence(&message, &probe);
    assert_eq!(
        shape_row_count(&app).await,
        rows_before,
        "a refused declare writes no shape row"
    );
    assert_eq!(probe.connections(), 0, "the declare must not fetch");
}

/// The MCP door: the refusal is a caller-side `invalid_params` carrying the fixed sentence, never
/// the internal-error envelope.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_uncompilable_schema_is_refused_at_declare_over_mcp(pool: PgPool) {
    let app = common::setup_relay(pool.clone()).await;
    let probe = FetchProbe::bind();
    let (context_id, _resource) = context_with_resource(&app, "e2e-uncompilable-mcp").await;
    let (svc, parts) = relay_service_and_parts(&app).await;
    let rows_before = shape_row_count(&app).await;

    let err = temper_mcp::tools::data_artifact_shapes::declare_shape(
        &svc,
        &parts,
        temper_mcp::tools::data_artifact_shapes::DeclareShapeInput {
            home_type: "context".to_string(),
            home_id: context_id.to_string(),
            kind: "measurement".to_string(),
            kind_owner: None,
            schema: remote_ref_schema(&probe),
            enforcement: EnforcementMode::Advisory,
            invocation_id: None,
            correlation_id: None,
            confidence: None,
            reasoning: None,
            rationale: None,
            persona: None,
            model: None,
        },
    )
    .await
    .expect_err("a schema that needs a fetch must be refused over MCP");

    assert_eq!(
        err.code.0, -32602,
        "the refusal must arrive as invalid_params, not an internal error: {err:?}"
    );
    assert_fixed_sentence(&err.message, &probe);
    assert_eq!(
        shape_row_count(&app).await,
        rows_before,
        "a refused declare writes no shape row"
    );
    assert_eq!(probe.connections(), 0, "the declare must not fetch");
}

/// The CLI door: the spawned process fails and its output carries the fixed sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_uncompilable_schema_is_refused_at_declare_over_the_cli(pool: PgPool) {
    let app = common::setup(pool).await;
    let probe = FetchProbe::bind();
    let (context_id, _resource) = context_with_resource(&app, "e2e-uncompilable-cli").await;
    let rows_before = shape_row_count(&app).await;

    let output = common::run_temper_cli_with_stdin(
        &app,
        &remote_ref_schema(&probe).to_string(),
        &[
            "data-artifact",
            "schema",
            "declare",
            &context_id.to_string(),
            "--kind",
            "measurement",
            "--content",
            "-",
        ],
    )
    .await
    .expect("cli run");

    assert!(
        !output.status.success(),
        "declaring a schema that needs a fetch must fail: stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_fixed_sentence(&combined, &probe);
    assert_eq!(
        shape_row_count(&app).await,
        rows_before,
        "a refused declare writes no shape row"
    );
    assert_eq!(probe.connections(), 0, "the declare must not fetch");
}

// ── U+0000: refused by name at the doors that would store it ────────────────────────────────
//
// Postgres stores no U+0000: a `jsonb` bind refuses it in a string or a key, and a `text` bind
// refuses it outright. A commit or a declare carrying one reached the database and answered 500.
// Both doors now refuse it first, as a typed refusal naming the field. The MCP tools and the CLI
// relay the request to these routes, so the API door is where it is witnessed. Each value hides
// the word `secret` beside its NUL, so a refusal that quoted the value would show it.

/// Every artifact on `resource`, folded ones included: the witness that a refused commit stored
/// nothing.
async fn artifacts_on(app: &common::E2eTestApp, resource: uuid::Uuid) -> usize {
    app.client
        .data_artifacts()
        .list_all(
            resource,
            &ArtifactListParams {
                include_folded: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("list the resource's artifacts")
        .len()
}

/// **A commit whose content or family name holds U+0000 is refused by name, as a 400 under
/// `DATA_ARTIFACT_REFUSAL_CODE`, and stores nothing; a commit without it is stored.** U+0000 in a
/// content string, in a content key inside an array, and in the family name: each arrives as
/// `ClientError::DataArtifactRefusal` (the client's variant for exactly that status and code),
/// naming `content` or `kind` and quoting none of the value. The control holds U+0001, which
/// Postgres stores.
///
/// Bite: in `DbBackend::commit_data_artifact`, the `unstorable_nul` check removed — each commit
/// answers 500 again, and the typed match panics on the first.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_nul_in_a_commit_is_refused_by_name_and_stores_nothing(pool: PgPool) {
    let app = common::setup(pool).await;
    let (_context, resource_id) = context_with_resource(&app, "e2e-nul-commit").await;
    let request = |kind: &str, content: serde_json::Value| ArtifactCommitRequest {
        kind: kind.to_string(),
        kind_owner: None,
        intent: "member".to_string(),
        precedence: 0.0,
        content,
        supersedes: Vec::new(),
        act: Default::default(),
    };

    for (field, kind, content) in [
        ("content", "measurement", json!({ "value": "a\u{0}secret" })),
        (
            "content",
            "measurement",
            json!({ "rows": [{ "k\u{0}secret": 1 }] }),
        ),
        ("kind", "measure\u{0}secret", json!({ "value": "fine" })),
    ] {
        let err = app
            .client
            .data_artifacts()
            .commit(resource_id, &request(kind, content))
            .await
            .expect_err("a commit holding U+0000 must refuse");
        let refusal = match err {
            ClientError::DataArtifactRefusal { message } => message,
            other => panic!("U+0000 in `{field}` must arrive as a typed 400, not as {other}"),
        };
        assert!(
            refusal.starts_with(&format!("`{field}` holds the character U+0000")),
            "the refusal names `{field}`, got: {refusal}"
        );
        assert!(
            !refusal.contains("secret"),
            "the refusal quotes nothing of the value, got: {refusal}"
        );
    }
    assert_eq!(
        artifacts_on(&app, resource_id).await,
        0,
        "a refused commit stores nothing"
    );

    app.client
        .data_artifacts()
        .commit(
            resource_id,
            &request("measurement", json!({ "value": "a\u{1}b" })),
        )
        .await
        .expect("the same commit without U+0000 is stored");
    assert_eq!(artifacts_on(&app, resource_id).await, 1);
}

/// **A shape whose schema or family name holds U+0000 is refused by name, as a 400 under
/// `DATA_ARTIFACT_REFUSAL_CODE`, and writes no shape row; a declare without it is stored.**
/// U+0000 in a schema key, in a schema string, and in the family name, each refused naming
/// `schema` or `kind` and quoting none of the value; the control declares the same family with a
/// clean schema.
///
/// Bite: in `shape_service::declare_shape`, the `unstorable_nul` check removed — each declare
/// answers 500 again, and the typed match panics on the first.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_nul_in_a_shape_is_refused_by_name_and_writes_nothing(pool: PgPool) {
    let app = common::setup(pool).await;
    let (context_id, _resource) = context_with_resource(&app, "e2e-nul-shape").await;
    let rows_before = shape_row_count(&app).await;
    let request = |kind: &str, schema: serde_json::Value| ShapeDeclareRequest {
        kind: kind.to_string(),
        kind_owner: None,
        schema,
        enforcement: EnforcementMode::Advisory,
        act: Default::default(),
    };

    for (field, kind, schema) in [
        (
            "schema",
            "measurement",
            json!({ "type": "object", "properties": { "k\u{0}secret": {} } }),
        ),
        ("schema", "measurement", json!({ "const": "a\u{0}secret" })),
        ("kind", "measure\u{0}secret", string_schema()),
    ] {
        let err = app
            .client
            .data_artifacts()
            .declare_shape(context_id, &request(kind, schema))
            .await
            .expect_err("a declare holding U+0000 must refuse");
        let refusal = match err {
            ClientError::DataArtifactRefusal { message } => message,
            other => panic!("U+0000 in `{field}` must arrive as a typed 400, not as {other}"),
        };
        assert!(
            refusal.starts_with(&format!("`{field}` holds the character U+0000")),
            "the refusal names `{field}`, got: {refusal}"
        );
        assert!(
            !refusal.contains("secret"),
            "the refusal quotes nothing of the value, got: {refusal}"
        );
    }
    assert_eq!(
        shape_row_count(&app).await,
        rows_before,
        "a refused declare writes no shape row"
    );

    app.client
        .data_artifacts()
        .declare_shape(context_id, &request("measurement", string_schema()))
        .await
        .expect("the same declare without U+0000 is stored");
    assert_eq!(shape_row_count(&app).await, rows_before + 1);
}

// ── Inputs the store cannot index or hold: refused by name ahead of the database ─────────────
//
// A family name is a key in three btree indexes (the live-shape unique index, and
// `idx_kb_data_artifacts_kind` / `idx_kb_data_artifacts_family`), and a btree row holds at most
// 2704 bytes after compression. A family name past that failed in the database: 500 at the shape
// door, and at the commit door the bare `54000` sentence, which names no field. An act's prose
// fields ride `kb_events.metadata` (`jsonb`), which cannot hold U+0000.

/// A family name far past every index's row limit, even compressed (300,000 characters answered
/// 500 at the shape door when this was found).
fn unindexable_family() -> String {
    "k".repeat(300_000)
}

/// **A shape whose family name is longer than `MAX_ARTIFACT_KIND_BYTES` is refused by name, as a
/// 400 under `DATA_ARTIFACT_REFUSAL_CODE` naming `kind` and the limit, and writes no shape row; a
/// family name at the limit is declared.**
///
/// Bite: in `shape_service::declare_shape`, the `overlong_kind` check removed — the 300,000-character
/// declare answers 500 again, and the typed match panics.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_overlong_family_name_is_refused_by_name_at_the_shape_door(pool: PgPool) {
    use temper_core::types::data_artifact::MAX_ARTIFACT_KIND_BYTES;
    let app = common::setup(pool).await;
    let (context_id, _resource) = context_with_resource(&app, "e2e-long-kind-shape").await;
    let rows_before = shape_row_count(&app).await;
    let request = |kind: String| ShapeDeclareRequest {
        kind,
        kind_owner: None,
        schema: string_schema(),
        enforcement: EnforcementMode::Advisory,
        act: Default::default(),
    };

    for kind in [
        unindexable_family(),
        "k".repeat(MAX_ARTIFACT_KIND_BYTES + 1),
    ] {
        let err = app
            .client
            .data_artifacts()
            .declare_shape(context_id, &request(kind))
            .await
            .expect_err("an over-long family name must refuse");
        let refusal = match err {
            ClientError::DataArtifactRefusal { message } => message,
            other => panic!("an over-long `kind` must arrive as a typed 400, not as {other}"),
        };
        assert!(
            refusal.starts_with("`kind` is ")
                && refusal.contains(&MAX_ARTIFACT_KIND_BYTES.to_string()),
            "the refusal names `kind` and its limit, got: {refusal}"
        );
    }
    assert_eq!(
        shape_row_count(&app).await,
        rows_before,
        "a refused declare writes no shape row"
    );

    app.client
        .data_artifacts()
        .declare_shape(context_id, &request("k".repeat(MAX_ARTIFACT_KIND_BYTES)))
        .await
        .expect("a family name at the limit is declared");
    assert_eq!(shape_row_count(&app).await, rows_before + 1);
}

/// **A commit whose family name is longer than `MAX_ARTIFACT_KIND_BYTES` is refused by name, as a
/// 400 under `DATA_ARTIFACT_REFUSAL_CODE` naming `kind` and the limit, and stores nothing; a
/// family name at the limit is stored.** No shape is declared, so nothing but the commit's own
/// indexes reads the name.
///
/// Bite: in `DbBackend::commit_data_artifact`, the `overlong_kind` check removed — the
/// 300,000-character commit answers the generic `54000` 400 (or a 500 before #1264), which is not
/// the typed refusal, and the match panics.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_overlong_family_name_is_refused_by_name_at_the_commit_door(pool: PgPool) {
    use temper_core::types::data_artifact::MAX_ARTIFACT_KIND_BYTES;
    let app = common::setup(pool).await;
    let (_context, resource_id) = context_with_resource(&app, "e2e-long-kind-commit").await;
    let request = |kind: String| ArtifactCommitRequest {
        kind,
        kind_owner: None,
        intent: "member".to_string(),
        precedence: 0.0,
        content: json!({ "value": "fine" }),
        supersedes: Vec::new(),
        act: Default::default(),
    };

    for kind in [
        unindexable_family(),
        "k".repeat(MAX_ARTIFACT_KIND_BYTES + 1),
    ] {
        let err = app
            .client
            .data_artifacts()
            .commit(resource_id, &request(kind))
            .await
            .expect_err("an over-long family name must refuse");
        let refusal = match err {
            ClientError::DataArtifactRefusal { message } => message,
            other => panic!("an over-long `kind` must arrive as a typed 400, not as {other}"),
        };
        assert!(
            refusal.starts_with("`kind` is ")
                && refusal.contains(&MAX_ARTIFACT_KIND_BYTES.to_string()),
            "the refusal names `kind` and its limit, got: {refusal}"
        );
    }
    assert_eq!(
        artifacts_on(&app, resource_id).await,
        0,
        "a refused commit stores nothing"
    );

    app.client
        .data_artifacts()
        .commit(resource_id, &request("k".repeat(MAX_ARTIFACT_KIND_BYTES)))
        .await
        .expect("a family name at the limit is stored");
    assert_eq!(artifacts_on(&app, resource_id).await, 1);
}

/// **U+0000 in any of an act's prose fields is refused as a 400 naming the field, and the write
/// stores nothing; the same act with U+0001 is stored.** Witnessed at the commit door; every door
/// that carries an act assembles it through `ActInput::into_act_context`, where the check sits.
///
/// Bite: in `ActInput::into_act_context`, the `storable` checks removed — each commit answers 500
/// when `kb_events.metadata` refuses the bind.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_nul_in_an_act_prose_field_is_refused_by_name(pool: PgPool) {
    use temper_core::types::authorship::{ActInput, ConfidenceBand};
    let app = common::setup(pool).await;
    let (_context, resource_id) = context_with_resource(&app, "e2e-nul-act").await;
    let request = |field: &str, value: &str| {
        let mut act = ActInput {
            confidence: Some(ConfidenceBand::Probable),
            ..Default::default()
        };
        let value = Some(value.to_string());
        match field {
            "reasoning" => act.reasoning = value,
            "rationale" => act.rationale = value,
            "persona" => act.persona = value,
            "model" => act.model = value,
            other => unreachable!("not an act prose field: {other}"),
        }
        ArtifactCommitRequest {
            kind: "measurement".to_string(),
            kind_owner: None,
            intent: "member".to_string(),
            precedence: 0.0,
            content: json!({ "value": "fine" }),
            supersedes: Vec::new(),
            act,
        }
    };

    for field in ["reasoning", "rationale", "persona", "model"] {
        let err = app
            .client
            .data_artifacts()
            .commit(resource_id, &request(field, "a\u{0}secret"))
            .await
            .expect_err("an act holding U+0000 must refuse");
        let message = match err {
            ClientError::Server {
                status: 400,
                message,
            } => message,
            other => panic!("U+0000 in `{field}` must arrive as a 400, not as {other}"),
        };
        assert!(
            message.contains(&format!("`{field}` holds the character U+0000")),
            "the refusal names `{field}`, got: {message}"
        );
        assert!(
            !message.contains("secret"),
            "the refusal quotes nothing of the value, got: {message}"
        );
    }
    assert_eq!(
        artifacts_on(&app, resource_id).await,
        0,
        "a refused act stores nothing"
    );

    app.client
        .data_artifacts()
        .commit(resource_id, &request("reasoning", "a\u{1}b"))
        .await
        .expect("the same act without U+0000 is stored");
    assert_eq!(artifacts_on(&app, resource_id).await, 1);
}
