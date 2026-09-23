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

use serde_json::json;
use sqlx::PgPool;
use temper_client::error::ClientError;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactListParams, KindOwnerInput,
};
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_services::auth_config::{AuthConfig, AuthMode};
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};
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
    match result.content.first().map(|c| &c.raw) {
        Some(rmcp::model::RawContent::Text(t)) => t.text.clone(),
        other => panic!("tool returned no text content part: {other:?}"),
    }
}

/// Build an MCP service over the test pool and seed its profile cache for the
/// `e2e-test-user` sub — the same pattern `act_authorship_mcp_e2e.rs` uses.
async fn mcp_service(pool: &sqlx::PgPool) -> temper_mcp::service::TemperMcpService {
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
            mcp_audience: common::TEST_AUDIENCE.to_string(),
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
        rate_limit: None,
        blob: None,
        blob_disabled_by_policy: false,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let svc = temper_mcp::service::TemperMcpService::new(state);

    let req = axum::http::Request::builder()
        .extension(temper_mcp::middleware::BearerToken("synthetic".to_string()))
        .extension(temper_services::auth::RawJwtClaims {
            sub: "e2e-test-user".to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            iat: 0,
        })
        .body(())
        .expect("build request");
    let (req_parts, ()) = req.into_parts();
    svc.ensure_profile_from_parts(&req_parts)
        .await
        .expect("seed profile cache");
    svc
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
    let app = common::setup(pool.clone()).await;
    let profile = app.client.profile().get().await.expect("profile").id;
    let context = app
        .client
        .contexts()
        .create("e2e-refusal-empty-mcp", None)
        .await
        .expect("context create failed");
    let svc = mcp_service(&pool).await;

    let result = temper_mcp::tools::data_artifact_shapes::declare_shape(
        &svc,
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
    let app = common::setup(pool.clone()).await;
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
    let svc = mcp_service(&pool).await;

    let err = temper_mcp::tools::data_artifact_shapes::declare_shape(
        &svc,
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
        .list(
            resource_id,
            &ArtifactListParams {
                kind: Some("measurement".to_string()),
                intent: None,
                include_folded: Some(true),
                counts: None,
            },
        )
        .await
        .expect("list after refusal");
    assert!(
        list.as_array().map(|a| a.is_empty()).unwrap_or(false),
        "a refused commit must record nothing, got: {list}"
    );
}

/// The enforcing refusal through the MCP commit door — `map_api_err` must not wrap it
/// in an internal-error envelope either.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enforcing_refusal_names_the_violations_over_mcp(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
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
    let svc = mcp_service(&pool).await;

    let err = temper_mcp::tools::data_artifacts::commit_artifact(
        &svc,
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
