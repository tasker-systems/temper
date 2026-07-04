#![cfg(feature = "test-db")]
//! Chunk B acceptance: an authored act created through the **MCP** `create_resource` tool, carrying
//! an `invocation_id` + discrete agent authorship, lands in the substrate with `kb_events.invocation_id`
//! set and the authorship serialized into `kb_events.metadata`.
//!
//! Drives the production MCP caller path (`TemperMcpService` → `require_profile` → `create_resource`)
//! against the real Axum-backed `AppState` + real Postgres, after opening a real invocation envelope
//! through the production client (`app.client.invocations()`). The act-stamp is asserted directly off
//! `kb_events` because the readback `invocation_show` does not yet surface per-act authorship — that
//! exposure is Chunk D, which adds a show-based assertion on top of this vertical.
//!
//! Invocations + create carry no embed work that this test inspects, so it runs on plain
//! `cargo make test-e2e` (the body chunk path is exercised elsewhere).

mod common;

use uuid::Uuid;

use temper_core::types::authorship::ActInput;
use temper_core::types::ids::InvocationId;
use temper_core::types::invocation_requests::OpenInvocationRequest;
use temper_core::types::ConfidenceBand;
use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`) — the invocation's
/// originating cogmap; root-team membership (via `enable_invite_only`) grants the READ that the
/// per-act correlation gate requires.
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Build an MCP service over the test pool and seed its profile cache for the `e2e-test-user` sub —
/// the same principal `app.token` authenticates as, so the invocation it opens is readable here.
async fn mcp_service(pool: &sqlx::PgPool) -> temper_mcp::service::TemperMcpService {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    let svc = temper_mcp::service::TemperMcpService::new(state);

    let req = axum::http::Request::builder()
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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_create_under_invocation_stamps_act_with_authorship(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Provision the principal and grant root-team membership so it can READ L0 (the per-act
    // correlation gate `anchor_readable_by_profile($, 'kb_cogmaps', originating_cogmap_id)`).
    let principal = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight")
        .id;
    common::enable_invite_only(&pool, principal).await;

    // A context to create into.
    app.client
        .contexts()
        .create("act-authorship-mcp", None)
        .await
        .expect("context create");

    // Open a real invocation envelope against L0.
    let ack = app
        .client
        .invocations()
        .open(&OpenInvocationRequest {
            trigger_kind: "e2e".into(),
            originating_cogmap: L0_COGMAP,
            parent_cogmap: None,
        })
        .await
        .expect("open invocation against L0");
    let invocation_id = ack.invocation_id;

    // Create a resource through the MCP tool, carrying the invocation + graded authorship.
    let svc = mcp_service(&pool).await;
    let input = temper_mcp::tools::resources::CreateResourceInput {
        context_ref: Some("@me/act-authorship-mcp".to_string()),
        cogmap: None,
        doc_type_name: "research".to_string(),
        title: "Authored via MCP".to_string(),
        content: None,
        sources: None,
        slug: None,
        origin_uri: None,
        owner: None,
        managed_meta: None,
        open_meta: None,
        act: ActInput {
            invocation_id: Some(InvocationId::from(invocation_id)),
            reasoning: Some("seeding the demo corpus".to_string()),
            confidence: Some(ConfidenceBand::Probable),
            persona: Some("steward".to_string()),
            ..Default::default()
        },
    };
    temper_mcp::tools::resources::create_resource(&svc, input)
        .await
        .expect("MCP create under invocation succeeds");

    // The authored `resource_created` act carries the invocation_id and the authorship metadata.
    // invocation_id is stamped ONLY on the authored act (other events in the txn keep default ctx),
    // so this row is unique.
    let row = sqlx::query(
        "SELECT e.invocation_id, e.metadata
           FROM kb_events e
           JOIN kb_event_types et ON et.id = e.event_type_id
          WHERE e.invocation_id = $1 AND et.name = 'resource_created'",
    )
    .bind(invocation_id)
    .fetch_one(&pool)
    .await
    .expect("the authored resource_created act is stamped with the invocation_id");

    use sqlx::Row;
    let stamped: Uuid = row.get("invocation_id");
    assert_eq!(stamped, invocation_id, "act correlates to the invocation");

    let metadata: serde_json::Value = row.get("metadata");
    assert_eq!(
        metadata["confidence"], "probable",
        "graded confidence band rides in kb_events.metadata: {metadata}"
    );
    assert_eq!(
        metadata["reasoning"], "seeding the demo corpus",
        "reasoning rides in kb_events.metadata: {metadata}"
    );
    assert_eq!(
        metadata["persona"], "steward",
        "persona rides in kb_events.metadata: {metadata}"
    );
}
