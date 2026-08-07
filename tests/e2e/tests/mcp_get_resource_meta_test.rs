#![cfg(feature = "test-db")]

//! What an MCP caller receives back for a resource, driven through the production tool functions
//! (`TemperMcpService` → `require_profile` → `tools::resources::*`) rather than through the
//! enrichment helper underneath them.
//!
//! Two contracts live here. The older one is Gap 2 from `mcp-frontmatter-roundtrip-gaps`: both
//! metadata tiers must reach the caller, on `get_resource` and on every row of `list_resources`, or
//! frontmatter written through this surface is write-blind from it.
//!
//! The newer one is the defect the `ResourceView` convergence closes. Three affordances the shipped
//! agent skill instructs every agent to use — the decorated `ref`, and the `returned` / `truncated`
//! paging state — were injected render-time by the CLI and by the CLI alone. MCP emitted a bare
//! JSON array with no `ref` on any row and no way to tell a page from a whole set.
//! `mcp_list_emits_ref_for_every_row` is that defect's named witness.
//!
//! These drive the tools rather than a helper on purpose: the enrichment helper no longer decides
//! what a resource response contains — the read that produces the `ResourceView` does — so a test
//! against the helper could pass while the tool that calls it asks for the wrong sections.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_mcp::service::TemperMcpService;
use temper_mcp::tools::resources::{GetResourceInput, ListResourcesInput};

fn sha2_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Build an MCP service whose profile cache is seeded from synthetic JWT claims — the production
/// caller path, mirroring `mcp_round_trip_test` and `mcp_segmented_ingest_test`.
async fn mcp_service(pool: &sqlx::PgPool) -> TemperMcpService {
    use temper_services::auth_config::{AuthConfig, AuthMode};
    use temper_services::config::ApiConfig;
    use temper_services::state::{AppState, JwksKeyStore};

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
    let svc = TemperMcpService::new(AppState::new(pool.clone(), jwks_store, api_config));

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

/// The tool functions return a `CallToolResult`; the payload is the JSON text of the typed
/// response. Taken as `impl Serialize` rather than naming `rmcp::model::CallToolResult`, so this
/// crate need not link rmcp (the convention `mcp_segmented_ingest_test` set).
fn tool_json(result: impl serde::Serialize) -> serde_json::Value {
    let value = serde_json::to_value(&result).expect("tool result serializes");
    let text = value["content"][0]["text"]
        .as_str()
        .expect("first content part is text");
    serde_json::from_str(text).expect("content part parses as JSON")
}

/// Seed a resource with managed + open meta through the production ingest path (POST /api/ingest)
/// and return its id. The substrate's collapsed write path is the only resource-creation surface.
async fn seed_resource(
    app: &common::E2eTestApp,
    context_name: &str,
    slug: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> uuid::Uuid {
    app.client
        .contexts()
        .create(context_name, None)
        .await
        .expect("context create");

    let view = app
        .client
        .ingest()
        .create(&IngestPayload {
            idempotency_key: None,
            segmented: None,
            goal: None,
            title: slug.to_string(),
            origin_uri: format!("mcp://test/{slug}"),
            context_ref: format!("@me/{context_name}"),
            home_cogmap_id: None,
            doc_type_name: "research".to_string(),
            content_hash: Some(format!("sha256:{}", sha2_hex(slug))),
            // EMPTY body: client-ingested prose rides in `chunks_packed`, so a non-empty `content`
            // would engage `create_resource`'s body-dedup, which collapses these empty-bodied batch
            // rows onto one (empty) hash. An empty body skips dedup → each distinct row persists.
            content: String::new(),
            metadata: None,
            managed_meta: Some(managed_meta.clone()),
            open_meta: Some(open_meta.clone()),
            chunks_packed: Some(pack_chunks(&[]).expect("pack empty chunks")),
            act: Default::default(),
            sources: Vec::new(),
        })
        .await
        .expect("ingest create");
    view.id.into()
}

/// `get_resource` returns both meta tiers, with the typed managed fields populated under their
/// canonical `temper-*` names and `open_meta` preserved verbatim.
///
/// The predecessor of this test also asserted a top-level `slug`, and was `#[ignore]`d against a
/// deferral (F1, receive-side identity-key injection) for as long as it existed. **That premise is
/// dead, not deferred:** `temper-slug` is `KeyFate::Die`, and `ResourceView` — the one shape every
/// surface now answers in — has no `slug` field at all. A resource's address is its decorated
/// `ref`, asserted below and in `mcp_list_emits_ref_for_every_row`. So the assertion is deleted
/// rather than carried forward ignored.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_get_resource_carries_both_meta_tiers(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let svc = mcp_service(&pool).await;

    let seeded_open = serde_json::json!({"tags": ["alpha", "mcp"], "weight": 3});
    let id = seed_resource(
        &app,
        "mcp-get-meta",
        "mcp-get-meta",
        &serde_json::json!({ "temper-stage": "in-progress" }),
        &seeded_open,
    )
    .await;

    let v = tool_json(
        temper_mcp::tools::resources::get_resource(
            &svc,
            GetResourceInput {
                id: id.to_string(),
                include_content: None,
                fields: None,
            },
        )
        .await
        .expect("get_resource"),
    );

    // doc_type lives on the typed top-level field (the substrate's `doc_type` property), not in the
    // managed bag — `temper-type` is `KeyFate::ReconcileToDocType`.
    assert_eq!(v["doc_type_name"], "research");
    // Workflow keys survive §7 as `kb_properties`, and reach the caller under their canonical
    // `temper-*` names rather than hoisted flat onto the response.
    assert_eq!(v["managed_meta"]["temper-stage"], "in-progress");
    assert!(
        v.as_object().expect("object").get("stage").is_none(),
        "no workflow field is hoisted onto the MCP response: {v}"
    );
    assert_eq!(v["open_meta"], seeded_open);
    assert_eq!(
        v["ref"].as_str(),
        Some(format!("mcp-get-meta-{id}").as_str()),
        "`show` carries the decorated ref too, not only `list`: {v}"
    );
}

/// A resource whose open_meta is an empty object must still surface a present-but-empty
/// `open_meta` block — readable, not silently dropped.
///
/// This is the `Some({})` vs `None` distinction the section vocabulary rests on: absent means *the
/// section was not requested*, never "empty". `get_resource` always requests it, so `None` here
/// would be a lie about the resource rather than about the request.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_get_resource_surfaces_empty_open_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let svc = mcp_service(&pool).await;

    let id = seed_resource(
        &app,
        "mcp-empty-open",
        "empty-open",
        &serde_json::json!({}),
        &serde_json::json!({}),
    )
    .await;

    let v = tool_json(
        temper_mcp::tools::resources::get_resource(
            &svc,
            GetResourceInput {
                id: id.to_string(),
                include_content: None,
                fields: None,
            },
        )
        .await
        .expect("get_resource"),
    );

    assert!(
        v["managed_meta"].is_object(),
        "managed_meta is always present, even all-empty: {v}"
    );
    assert_eq!(
        v["open_meta"],
        serde_json::json!({}),
        "an empty open tier must still surface (not be dropped to absent): {v}"
    );
}

/// **The named witness for the defect this task closes.** Every row of an MCP list carries a
/// decorated, self-resolving `ref`.
///
/// `ref` is `sluggify(title)-<uuid>` — the address the shipped agent skill instructs every agent to
/// copy and paste back. It was injected at render time by `temper-cli` and by nothing else, so an
/// agent on the MCP surface was told to use a field it was never sent. It is a field of
/// `ResourceView` now, derived once by `with_derived_refs` in the read that assembles the view, so
/// no surface can emit a row without one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_list_emits_ref_for_every_row(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let svc = mcp_service(&pool).await;

    let id_a = seed_resource(
        &app,
        "mcp-ref-witness",
        "ref-witness-a",
        &serde_json::json!({}),
        &serde_json::json!({}),
    )
    .await;
    let id_b = seed_resource(
        &app,
        "mcp-ref-witness-2",
        "ref-witness-b",
        &serde_json::json!({}),
        &serde_json::json!({}),
    )
    .await;

    let v = tool_json(
        temper_mcp::tools::resources::list_resources(
            &svc,
            ListResourcesInput {
                goal: None,
                cogmap: None,
                stage: None,
                status: None,
                tags: None,
                context_ref: None,
                doc_type_name: Some("research".to_string()),
                limit: None,
                offset: None,
                fields: None,
            },
        )
        .await
        .expect("list_resources"),
    );

    let rows = v["rows"].as_array().expect("rows array");
    assert!(rows.len() >= 2, "both seeded rows are visible: {v}");
    for row in rows {
        let r#ref = row["ref"]
            .as_str()
            .unwrap_or_else(|| panic!("every row carries a `ref`: {row}"));
        let id = row["id"].as_str().expect("every row carries an id");
        assert!(
            r#ref.ends_with(id),
            "`ref` is decorated and self-resolving — it ends in the resource's own uuid: {row}"
        );
    }

    let by_id = |id: uuid::Uuid| -> String {
        rows.iter()
            .find(|row| row["id"].as_str() == Some(id.to_string().as_str()))
            .and_then(|row| row["ref"].as_str())
            .unwrap_or_else(|| panic!("row {id} present"))
            .to_string()
    };
    assert_eq!(by_id(id_a), format!("ref-witness-a-{id_a}"));
    assert_eq!(by_id(id_b), format!("ref-witness-b-{id_b}"));
}

/// The list envelope carries `total`, `returned` and `truncated` — the three quantities the shipped
/// skill tells every agent to read before concluding a resource is absent.
///
/// MCP answered with a bare JSON array until this task, so none of them existed on this surface.
/// `truncated` is the load-bearing one: it is what tells a caller that what it sees is not all
/// there is.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_list_envelope_carries_paging_state(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let svc = mcp_service(&pool).await;

    for n in 0..3 {
        seed_resource(
            &app,
            &format!("mcp-paging-{n}"),
            &format!("paging-{n}"),
            &serde_json::json!({}),
            &serde_json::json!({}),
        )
        .await;
    }

    let page = |limit: Option<i64>| {
        let svc = &svc;
        async move {
            tool_json(
                temper_mcp::tools::resources::list_resources(
                    svc,
                    ListResourcesInput {
                        goal: None,
                        cogmap: None,
                        stage: None,
                        status: None,
                        tags: None,
                        context_ref: None,
                        doc_type_name: Some("research".to_string()),
                        limit,
                        offset: None,
                        fields: None,
                    },
                )
                .await
                .expect("list_resources"),
            )
        }
    };

    let first = page(Some(1)).await;
    assert_eq!(first["returned"], 1, "one row asked for, one returned");
    assert_eq!(first["rows"].as_array().expect("rows").len(), 1);
    assert_eq!(first["limit"], 1);
    assert_eq!(first["offset"], 0);
    assert!(
        first["total"].as_i64().expect("total") >= 3,
        "`total` is the FILTERED match count, not the page's: {first}"
    );
    assert_eq!(
        first["truncated"], true,
        "rows beyond this page exist and the caller must be told: {first}"
    );

    let whole = page(None).await;
    assert_eq!(
        whole["returned"], whole["total"],
        "the whole filtered set came back: {whole}"
    );
    assert_eq!(
        whole["truncated"], false,
        "nothing is hidden, so nothing is claimed to be: {whole}"
    );
}

/// Every row of an MCP list carries both metadata tiers — the list surface is not meta-blind.
///
/// The tiers no longer arrive from a per-row meta fetch: the managed one is joined by
/// `readback::hit_identities` and the open one is asked for as the `open-meta` SECTION, both once
/// for the whole page. What this asserts is the contract, not the mechanism; the statement count is
/// measured separately in `temper-services`' `list_page_open_meta_query_count_test`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn mcp_list_carries_both_meta_tiers_for_every_row(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    let svc = mcp_service(&pool).await;

    let id_a = seed_resource(
        &app,
        "mcp-batch",
        "batch-a",
        &serde_json::json!({"temper-stage": "backlog"}),
        &serde_json::json!({"tags": ["a"]}),
    )
    .await;
    let id_b = seed_resource(
        &app,
        "mcp-batch-2",
        "batch-b",
        &serde_json::json!({"temper-stage": "done"}),
        &serde_json::json!({"tags": ["b"]}),
    )
    .await;

    let v = tool_json(
        temper_mcp::tools::resources::list_resources(
            &svc,
            ListResourcesInput {
                goal: None,
                cogmap: None,
                stage: None,
                status: None,
                tags: None,
                context_ref: None,
                doc_type_name: Some("research".to_string()),
                limit: None,
                offset: None,
                fields: None,
            },
        )
        .await
        .expect("list_resources"),
    );
    let rows = v["rows"].as_array().expect("rows array");

    let row_of = |id: uuid::Uuid| -> &serde_json::Value {
        rows.iter()
            .find(|row| row["id"].as_str() == Some(id.to_string().as_str()))
            .unwrap_or_else(|| panic!("row {id} present in {v}"))
    };
    assert_eq!(row_of(id_a)["managed_meta"]["temper-stage"], "backlog");
    assert_eq!(row_of(id_b)["managed_meta"]["temper-stage"], "done");
    assert_eq!(row_of(id_a)["open_meta"]["tags"][0], "a");
    assert_eq!(row_of(id_b)["open_meta"]["tags"][0], "b");

    for row in rows {
        assert!(
            row["open_meta"].is_object(),
            "every listed row carries the open tier: {row}"
        );
    }
}
