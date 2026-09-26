#![cfg(feature = "test-db")]
//! The resources family's parity suite, re-harnessed across the network door.
//!
//! [Beat G3a](temper task `01a0cb1b-42e6-7922-a9dc-b30c402d8a3f`) authored this suite
//! FIRST against the direct-service binding and proved it green there; G3a-prime moves
//! the family onto the deployed API (the network door) and this suite must stay green
//! with the wire underneath: a parity suite that never saw the old binding cannot prove
//! parity, and one that never crosses the real listener cannot prove the door
//! (`no-door-regression`'s per-family bite).
//!
//! The tools are driven the way production drives them now: a real `TemperMcpService`
//! whose relay config points at THIS process's `create_app` listener, per-request parts
//! carrying a REAL minted bearer that the API's own auth middleware resolves — the same
//! hop the deployed relay makes. Response shapes and refusal kinds asserted below are
//! the incumbent ones, carried from the direct-binding pin (including the named parity
//! deltas the door itself introduced).

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

mod parity {
    use serde::Deserialize;
    use serde_json::json;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The harness principal's email — the one `approve_app_principal` provisions and
    /// standing-approves, and the profile behind every `app.relay_parts()` bearer.
    pub const EMAIL: &str = "e2e@test.example.com";

    /// The id of the caller's auto-provisioned default context — the writable home.
    pub async fn default_context_id(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "SELECT c.id \
             FROM kb_contexts c \
             JOIN kb_profiles p ON p.id = c.owner_id \
             WHERE p.email = $1 AND c.name = 'default'",
        )
        .bind(EMAIL)
        .fetch_one(pool)
        .await
        .expect("the auto-provisioned default context")
    }

    pub async fn profile_id_by_email(pool: &PgPool, email: &str) -> Uuid {
        sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
            .bind(email)
            .fetch_one(pool)
            .await
            .expect("the profile")
    }

    /// A SECOND approved identity for the owner/other tests: warmed through the real
    /// listener (JIT provisioning with the correct handle, per-surface emitters, and
    /// its own default context) and standing-approved by its own email — the same two
    /// steps the harness runs for its principal, on a different bearer. Identity rides
    /// the token alone, so this identity shares the one MCP service.
    pub async fn second_identity(
        app: &super::common::E2eTestApp,
        pool: &PgPool,
        tag: &str,
    ) -> (axum::http::request::Parts, String) {
        let unique = Uuid::new_v4();
        let email = format!("{tag}-{unique}@example.com");
        let token = super::common::generate_test_jwt(&format!("{tag}-sub-{unique}"), &email);
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
        .execute(pool)
        .await
        .expect("approve the second identity's standing");
        (app.relay_parts_for(&token), email)
    }

    /// Build a tool input from its WIRE shape, so the deserializer — not a struct
    /// literal — pins the field names an MCP caller actually sends.
    pub fn input<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("input deserializes from its wire shape")
    }

    pub fn create_body(context_id: &Uuid, title: &str, content: Option<&str>) -> serde_json::Value {
        let mut body = json!({
            "context_ref": context_id.to_string(),
            "doc_type_name": "session",
            "title": title,
        });
        if let Some(content) = content {
            body["content"] = json!(content);
        }
        body
    }

    /// The single text part a one-part tool result carries.
    pub fn one_text(res: &rmcp::model::CallToolResult) -> serde_json::Value {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        serde_json::from_str(parts[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response")
    }

    pub fn parts_of(res: &rmcp::model::CallToolResult) -> Vec<String> {
        res.content
            .iter()
            .map(|c| c.as_text().expect("a text part").text.clone())
            .collect()
    }

    /// `rmcp::ErrorData` codes: -32602 invalid_params, -32603 internal_error.
    pub fn code_of(err: &rmcp::ErrorData) -> i32 {
        err.code.0
    }
}

use common::E2eTestApp;
use parity::{code_of, default_context_id, input, one_text, parts_of};

/// The parity harness, once per test: the relay-ready service over this app's real
/// listener, and parts carrying the harness principal's REAL bearer.
async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

/// Create through the tool under test and hand back the response value. The caller is
/// the harness principal; the home is its auto-provisioned default context.
async fn create(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    pool: &PgPool,
    title: &str,
    content: Option<&str>,
) -> serde_json::Value {
    let ctx = default_context_id(pool).await;
    let res = temper_mcp::tools::resources::create_resource(
        svc,
        parts,
        input(parity::create_body(&ctx, title, content)),
    )
    .await
    .expect("create lands");
    one_text(&res)
}

// ── create_resource ─────────────────────────────────────────────────

/// The create response is `CreateResourceResponse`: the ENRICHED view flattened at the
/// top level (ref + managed_meta + open_meta + derived embedding_status riding beside
/// the view's own keys), plus `status: "created"` (status is always `Created` on this
/// surface; the backend's dedup converges silently).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_answers_in_the_enriched_shape_with_status_created(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let value = create(
        &svc,
        &parts,
        &_app.pool,
        "Created through the network door",
        None,
    )
    .await;

    assert_eq!(value["status"], "created");
    let resource = &value["resource"];
    let id = resource["id"].as_str().expect("id present");
    assert_eq!(
        resource["ref"].as_str(),
        Some(
            temper_workflow::operations::decorated_ref(
                "Created through the network door",
                temper_core::types::ids::ResourceId(Uuid::parse_str(id).expect("id is a uuid"),),
            )
            .as_str()
        ),
        "every MCP resource response carries a decorated ref: {value}"
    );
    assert!(
        resource.get("managed_meta").is_some(),
        "the managed tier rides the response: {value}"
    );
    assert!(
        resource.get("open_meta").is_some(),
        "the open tier rides the response (enriched_view always asks for it): {value}"
    );
    assert!(
        ["ready", "pending", "failed"].contains(
            &resource["embedding_status"]
                .as_str()
                .expect("status is a string")
        ),
        "embedding_status is one of the three states, got {value}"
    );
}

/// The home-anchor refusals are caller errors shaped MCP-locally, before the wire:
/// both homes, and neither — `invalid_params` with the named sentences.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_refuses_both_and_neither_home(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&_app.pool).await;

    let err = temper_mcp::tools::resources::create_resource(
        &svc,
        &parts,
        input(json!({
            "context_ref": ctx.to_string(),
            "cogmap": format!("{ctx}"),
            "doc_type_name": "session",
            "title": "T",
        })),
    )
    .await
    .expect_err("both homes refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message,
        "context_ref and cogmap are mutually exclusive; supply exactly one home"
    );

    let err = temper_mcp::tools::resources::create_resource(
        &svc,
        &parts,
        input(json!({
            "doc_type_name": "session",
            "title": "T",
        })),
    )
    .await
    .expect_err("no home refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message,
        "no home specified — supply exactly one of context_ref or cogmap"
    );
}

/// Two distinct gate faces on the create path, both `invalid_params`, both adjudicated
/// at the API on the relayed bearer: a context that is not VISIBLE fails the door's
/// server-side resolution with the service's own sentence spoken ONCE (the direct
/// binding's doubled "context not found: " prefix is gone with `build_create_command`'s
/// wrapper — the named parity delta), while a context that is visible but not WRITABLE
/// reaches the backend's F1 gate and renders the Forbidden arm's sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_refuses_a_context_without_write_access(pool: PgPool) {
    let (app, svc, owner_parts) = harness(pool).await;
    let (other_parts, other_email) = parity::second_identity(&app, &app.pool, "create-other").await;
    let ctx = default_context_id(&app.pool).await;
    let other_profile_id: Uuid = parity::profile_id_by_email(&app.pool, &other_email).await;

    // Not visible: resolution fails before any write gate.
    let err = temper_mcp::tools::resources::create_resource(
        &svc,
        &other_parts,
        input(parity::create_body(&ctx, "T", None)),
    )
    .await
    .expect_err("an unreadable context is refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message, "context not found or not readable",
        "the service's own sentence, once, got: {err}"
    );

    // Visible (read granted) but not writable: the backend gate speaks.
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, \
              granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(ctx)
    .bind(other_profile_id)
    .bind(parity::profile_id_by_email(&app.pool, parity::EMAIL).await)
    .execute(&app.pool)
    .await
    .expect("read grant lands");

    let err = temper_mcp::tools::resources::create_resource(
        &svc,
        &other_parts,
        input(parity::create_body(&ctx, "T", None)),
    )
    .await
    .expect_err("a read-only context is refused for a write");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.starts_with(
            "Not authorized to create in this context: placing a resource requires write access,"
        ),
        "the gate's own sentence, got: {err}"
    );
    let _ = owner_parts;
}

/// An unresolvable context ref is a caller error naming the cause; the service's own
/// sentence arrives once (see the named parity delta on
/// `create_refuses_a_context_without_write_access`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_refuses_an_unknown_context(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7();

    let err = temper_mcp::tools::resources::create_resource(
        &svc,
        &parts,
        input(parity::create_body(&ghost, "T", None)),
    )
    .await
    .expect_err("an unknown context is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.starts_with("context not found"), "got: {err}");
}

/// The create-side sources guard SURVIVES the door: the ingest path carries sources
/// only inside the body update, so an empty body would silently DROP them — the tool
/// refuses first, exactly as the direct binding's `provenance_body` did.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_refuses_sources_without_content(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&_app.pool).await;
    let mut body = parity::create_body(&ctx, "Sources with no body", None);
    body["sources"] = json!(["https://example.com/origin-doc"]);

    let err = temper_mcp::tools::resources::create_resource(&svc, &parts, input(body))
        .await
        .expect_err("sources without content is refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message,
        "sources supplied without content — there is no body block to attribute"
    );
}

// ── get_resource ────────────────────────────────────────────────────

/// The incumbent get: open-meta always present, body absent until asked, and the body
/// leaves the JSON to become its OWN content part.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_returns_open_meta_and_splits_the_body_into_a_second_part(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let body = "The body that parity pins.";
    let created = create(&svc, &parts, &_app.pool, "Get body split", Some(body)).await;
    let id = created["resource"]["id"].as_str().unwrap();

    let without = one_text(
        &temper_mcp::tools::resources::get_resource(&svc, &parts, input(json!({ "id": id })))
            .await
            .expect("get lands"),
    );
    assert_eq!(without["id"], json!(id));
    assert!(
        without.get("content").is_none() || without["content"].is_null(),
        "the body is not in the metadata object: {without}"
    );

    let asked = parts_of(
        &temper_mcp::tools::resources::get_resource(
            &svc,
            &parts,
            input(json!({ "id": id, "include_content": true })),
        )
        .await
        .expect("get lands"),
    );
    assert_eq!(asked.len(), 2, "metadata part + body part");
    let meta: serde_json::Value = serde_json::from_str(&asked[0]).unwrap();
    assert!(
        meta.get("content").is_none() || meta["content"].is_null(),
        "the body has left the JSON: {meta}"
    );
    assert_eq!(asked[1], body, "the second part IS the markdown body");
}

/// The incumbent refusal kind for an unreadable/absent resource on GET is
/// `internal_error` naming the read — parity pins what IS, defects and all; improving
/// the kind is a later, deliberate change.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_unknown_resource_is_an_internal_error_naming_the_read(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = temper_core::types::ids::ResourceId(Uuid::now_v7());

    let err = temper_mcp::tools::resources::get_resource(
        &svc,
        &parts,
        input(json!({ "id": ghost.0.to_string() })),
    )
    .await
    .expect_err("an unknown resource does not answer");
    assert_eq!(code_of(&err), -32603);
    assert!(
        err.message.starts_with("Failed to get resource:"),
        "got: {err}"
    );
}

/// `fields` filters TOP-LEVEL keys, anchored on `id`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_fields_projection_keeps_the_anchor_and_drops_the_rest(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let created = create(&svc, &parts, &_app.pool, "Projected", None).await;
    let id = created["resource"]["id"].as_str().unwrap();

    let value = one_text(
        &temper_mcp::tools::resources::get_resource(
            &svc,
            &parts,
            input(json!({ "id": id, "fields": ["managed_meta", "embedding_status"] })),
        )
        .await
        .expect("get lands"),
    );
    assert!(value.get("id").is_some(), "the anchor survives: {value}");
    assert!(value.get("managed_meta").is_some(), "{value}");
    assert!(value.get("title").is_none(), "{value}");
    assert!(value.get("ref").is_none(), "{value}");
}

// ── list_resources ──────────────────────────────────────────────────

/// The list envelope carries the paging state the agent skill instructs callers to
/// read, and rows arrive enriched (embedding_status per row).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_envelope_carries_paging_state_and_enriched_rows(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&_app.pool).await;
    create(&svc, &parts, &_app.pool, "Listed one", None).await;
    create(&svc, &parts, &_app.pool, "Listed two", None).await;

    let value = one_text(
        &temper_mcp::tools::resources::list_resources(
            &svc,
            &parts,
            input(json!({ "context_ref": ctx.to_string(), "limit": 1 })),
        )
        .await
        .expect("list lands"),
    );
    assert_eq!(
        value["total"], 2,
        "the FILTERED count, not the page: {value}"
    );
    assert_eq!(value["returned"], 1);
    assert_eq!(
        value["truncated"], true,
        "one row is beyond the page: {value}"
    );
    assert_eq!(value["limit"], 1);
    assert_eq!(value["offset"], 0);
    assert!(
        value.get("facets").is_some(),
        "the doc-type histogram rides the envelope: {value}"
    );
    let rows = value["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].get("embedding_status").is_some(),
        "each row is enriched: {}",
        rows[0]
    );
    assert!(rows[0].get("ref").is_some(), "{}", rows[0]);
    // The open tier rides every row — the `sections=open-meta` param is load-bearing on
    // this envelope, so its absence must red (bite-proven: dropping it changed nothing
    // else this test asserted).
    assert!(
        rows[0].get("open_meta").is_some(),
        "the open tier rides the row, as asked by sections=open-meta: {}",
        rows[0]
    );
}

/// An unresolvable filter ref is a caller error prefixed `unknown filter:`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_refuses_an_unresolvable_context_filter(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ghost = Uuid::now_v7();

    let err = temper_mcp::tools::resources::list_resources(
        &svc,
        &parts,
        input(json!({ "context_ref": ghost.to_string() })),
    )
    .await
    .expect_err("an unresolvable filter is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(err.message.starts_with("unknown filter:"), "got: {err}");
}

/// A tag containing a comma is refused, not split.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_refuses_a_tag_containing_a_comma(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = temper_mcp::tools::resources::list_resources(
        &svc,
        &parts,
        input(json!({ "tags": ["ci,security"] })),
    )
    .await
    .expect_err("a comma tag is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message.contains("a tag may not contain a comma"),
        "got: {err}"
    );
}

// ── update_resource ─────────────────────────────────────────────────

/// The update response is the ENRICHED current state, and the gates render refusals,
/// not faults: not-found and not-modifiable are distinct `invalid_params` sentences.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_returns_the_enriched_view_and_refuses_gates(pool: PgPool) {
    let (app, svc, owner_parts) = harness(pool).await;
    let (other_parts, _other_email) = parity::second_identity(&app, &app.pool, "upd-other").await;
    let created = create(&svc, &owner_parts, &app.pool, "Original title", None).await;
    let id = created["resource"]["id"].as_str().unwrap().to_string();

    let forbidden = temper_mcp::tools::resources::update_resource(
        &svc,
        &other_parts,
        input(json!({ "id": id, "title": "Hijacked" })),
    )
    .await
    .expect_err("a non-writer is refused");
    assert_eq!(code_of(&forbidden), -32602);
    assert_eq!(forbidden.message, "Resource not found or not modifiable");

    // The incumbent ghost-resource update ALSO renders the Forbidden arm's sentence —
    // on the update path, missing and not-modifiable arrive through the same face.
    let ghost = Uuid::now_v7().to_string();
    let missing = temper_mcp::tools::resources::update_resource(
        &svc,
        &owner_parts,
        input(json!({ "id": ghost, "title": "Ghost" })),
    )
    .await
    .expect_err("an unknown resource is refused");
    assert_eq!(code_of(&missing), -32602);
    assert_eq!(missing.message, "Resource not found or not modifiable");

    let updated = one_text(
        &temper_mcp::tools::resources::update_resource(
            &svc,
            &owner_parts,
            input(json!({ "id": id, "title": "Updated title" })),
        )
        .await
        .expect("the owner's update lands"),
    );
    assert_eq!(updated["title"], "Updated title");
    assert!(
        updated.get("embedding_status").is_some(),
        "the response is the ENRICHED view: {updated}"
    );
    assert!(updated.get("ref").is_some(), "{updated}");
}

/// `sources` without a body block have nothing to attribute — the parse-don't-validate
/// guard fires at the tool.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_refuses_sources_without_content(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let created = create(&svc, &parts, &_app.pool, "No sources yet", None).await;
    let id = created["resource"]["id"].as_str().unwrap();

    let err = temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({ "id": id, "sources": ["https://example.com/a"] })),
    )
    .await
    .expect_err("sources without content is refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message,
        "sources supplied without content — there is no body block to attribute"
    );
}

// ── annotate_resource ───────────────────────────────────────────────

/// Annotate returns the itemized provenance rows it just recorded — an external URL
/// classifies as a Remote source.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn annotate_returns_the_provenance_rows_it_recorded(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let created = create(
        &svc,
        &parts,
        &_app.pool,
        "Annotated",
        Some("A body worth attributing."),
    )
    .await;
    let id = created["resource"]["id"].as_str().unwrap();

    let rows = one_text(
        &temper_mcp::tools::resources::annotate_resource(
            &svc,
            &parts,
            input(json!({
                "id": id,
                "sources": ["https://example.com/origin-doc"],
            })),
        )
        .await
        .expect("annotate lands"),
    );
    let rows = rows.as_array().expect("an array of rows");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["source_kind"], "remote", "{rows:?}");
    assert_eq!(
        rows[0]["source_uri"], "https://example.com/origin-doc",
        "{rows:?}"
    );
}

// ── update_resource_meta ────────────────────────────────────────────

/// The meta-only path answers `{updated: true, id}` and the write is visible on the
/// next read. The doc type is `task` because the managed vocabulary is doc-type-scoped
/// — `temper-stage` belongs to task/goal, and a session carrying it is refused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_meta_answers_the_ack_and_lands_the_change(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&_app.pool).await;
    let mut body = parity::create_body(&ctx, "Meta target", None);
    body["doc_type_name"] = json!("task");
    let created = one_text(
        &temper_mcp::tools::resources::create_resource(&svc, &parts, input(body))
            .await
            .expect("task lands"),
    );
    let id = created["resource"]["id"].as_str().unwrap();

    let ack = one_text(
        &temper_mcp::tools::resources::update_resource_meta(
            &svc,
            &parts,
            input(json!({
                "id": id,
                "managed_meta": { "temper-stage": "done" },
                "open_meta": {},
            })),
        )
        .await
        .expect("meta update lands"),
    );
    assert_eq!(ack, json!({ "updated": true, "id": json!(id) }));

    let after = one_text(
        &temper_mcp::tools::resources::get_resource(&svc, &parts, input(json!({ "id": id })))
            .await
            .expect("get lands"),
    );
    assert_eq!(after["managed_meta"]["temper-stage"], "done", "{after}");
}

// ── delete_resource ─────────────────────────────────────────────────

/// Delete answers the ack; the resource then reads as the incumbent get-refusal, and a
/// second delete is a `Resource not found:` refusal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delete_answers_the_ack_and_the_resource_stops_answering(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let created = create(&svc, &parts, &_app.pool, "Doomed", None).await;
    let id = created["resource"]["id"].as_str().unwrap();

    let ack = one_text(
        &temper_mcp::tools::resources::delete_resource(&svc, &parts, input(json!({ "id": id })))
            .await
            .expect("delete lands"),
    );
    assert_eq!(ack, json!({ "deleted": true, "id": json!(id) }));

    let gone = temper_mcp::tools::resources::get_resource(&svc, &parts, input(json!({ "id": id })))
        .await
        .expect_err("a deleted resource stops answering");
    assert_eq!(code_of(&gone), -32603);

    // The incumbent second delete renders the FORBIDDEN arm's sentence, not the NotFound
    // one — a tombstoned row is not-modifiable, not missing.
    let again =
        temper_mcp::tools::resources::delete_resource(&svc, &parts, input(json!({ "id": id })))
            .await
            .expect_err("a second delete is a refusal");
    assert_eq!(code_of(&again), -32602);
    assert_eq!(again.message, "Resource not found or not modifiable");

    // Act authorship rides the door's query string, and the door validates it —
    // reasoning without a confidence band is a 400 CALLER error, mapped to
    // invalid_params like the direct binding's client-side assembler did, never an
    // internal error.
    let bad_act = temper_mcp::tools::resources::delete_resource(
        &svc,
        &parts,
        input(json!({
            "id": created["resource"]["id"],
            "reasoning": "correlated but unconfident",
        })),
    )
    .await
    .expect_err("incomplete authorship is a refusal");
    assert_eq!(code_of(&bad_act), -32602);
    assert!(
        bad_act.message.contains("confidence"),
        "the door's own authorship sentence, got: {bad_act}"
    );
}

// ── block reads ─────────────────────────────────────────────────────

/// The block-read tri-state, on this surface: a live block renders as data, an absent
/// address renders as data (`state: "absent"`), and a not-visible home renders as an
/// `invalid_params` refusal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn block_read_tri_state_holds(pool: PgPool) {
    let (app, svc, owner_parts) = harness(pool).await;
    let (other_parts, _other_email) = parity::second_identity(&app, &app.pool, "block-other").await;
    let created = create(
        &svc,
        &owner_parts,
        &app.pool,
        "Block host",
        Some("Attributed body."),
    )
    .await;
    let id = created["resource"]["id"].as_str().unwrap();

    temper_mcp::tools::resources::annotate_resource(
        &svc,
        &owner_parts,
        input(json!({
            "id": id,
            "sources": ["https://example.com/src"],
        })),
    )
    .await
    .expect("annotate lands");
    let provenance = one_text(
        &temper_mcp::tools::resources::get_block_provenance(
            &svc,
            &owner_parts,
            input(json!({ "resource": id })),
        )
        .await
        .expect("provenance lands"),
    );
    let rows = provenance.as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    let block_id = rows[0]["block_id"].as_str().expect("block id on the row");

    let live = one_text(
        &temper_mcp::tools::resources::get_block(
            &svc,
            &owner_parts,
            input(json!({ "resource": id, "block_id": block_id })),
        )
        .await
        .expect("the block reads"),
    );
    assert_eq!(live["state"], "live", "{live}");

    let absent = one_text(
        &temper_mcp::tools::resources::get_block(
            &svc,
            &owner_parts,
            input(json!({
                "resource": id,
                "block_id": Uuid::now_v7().to_string(),
            })),
        )
        .await
        .expect("an absent address is DATA, not an error"),
    );
    assert_eq!(absent["state"], "absent", "{absent}");

    let invisible = temper_mcp::tools::resources::get_block(
        &svc,
        &other_parts,
        input(json!({ "resource": id, "block_id": block_id })),
    )
    .await
    .expect_err("a not-visible home is a refusal");
    assert_eq!(code_of(&invisible), -32602);
}

// ── lineage ─────────────────────────────────────────────────────────

/// The lineage walk is bidirectional over `derived_from` EDGES (migration
/// 20260712000080 — source = deriver, target = ancestor, label-keyed). The edge is
/// minted explicitly through the relationships tool as the fixture; resource-ref
/// provenance rows do NOT mint lineage edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn lineage_walks_derived_from_both_ways(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;
    let parent = create(&svc, &parts, &_app.pool, "The parent", None).await;
    let parent_id = parent["resource"]["id"].as_str().unwrap().to_string();

    let ctx = default_context_id(&_app.pool).await;
    let child_body = parity::create_body(&ctx, "The child", None);
    let child = one_text(
        &temper_mcp::tools::resources::create_resource(&svc, &parts, input(child_body))
            .await
            .expect("child lands"),
    );
    let child_id = child["resource"]["id"].as_str().unwrap();

    temper_mcp::tools::relationships::relationship(
        &svc,
        &parts,
        input(json!({
            "action": "assert",
            "source": child_id,
            "target": parent_id,
            "edge_kind": "express",
            "polarity": "forward",
            "label": "derived_from",
            "weight": 1.0,
        })),
    )
    .await
    .expect("the lineage edge asserts");

    let lineage = one_text(
        &temper_mcp::tools::resources::resource_lineage(
            &svc,
            &parts,
            input(json!({ "id": child_id })),
        )
        .await
        .expect("lineage lands"),
    );
    let ancestors = lineage["ancestors"].as_array().expect("ancestors array");
    assert!(
        ancestors
            .iter()
            .any(|a| a["resource_id"].as_str() == Some(parent_id.as_str())),
        "the parent is an ancestor: {lineage}"
    );

    let reverse = one_text(
        &temper_mcp::tools::resources::resource_lineage(
            &svc,
            &parts,
            input(json!({ "id": parent_id })),
        )
        .await
        .expect("lineage lands"),
    );
    let descendants = reverse["descendants"]
        .as_array()
        .expect("descendants array");
    assert!(
        descendants
            .iter()
            .any(|d| d["resource_id"].as_str() == Some(child_id)),
        "the child is a descendant: {reverse}"
    );
}

// ── grants ──────────────────────────────────────────────────────────

/// The grant tools' gates, on this surface: input-shape refusals (both principals, no
/// capability) are `invalid_params` at the tool; a non-administering caller is refused
/// with the mapping's sentence; the owner CAN grant.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn grant_gates_and_outcomes_hold(pool: PgPool) {
    let (app, svc, owner_parts) = harness(pool).await;
    let (other_parts, other_email) = parity::second_identity(&app, &app.pool, "grant-other").await;
    let created = create(&svc, &owner_parts, &app.pool, "Grant target", None).await;
    let id = created["resource"]["id"].as_str().unwrap();
    let other_profile_id: Uuid = parity::profile_id_by_email(&app.pool, &other_email).await;

    // Input shape: both principals.
    let err = temper_mcp::tools::resources::resource_grant(
        &svc,
        &owner_parts,
        input(json!({
            "resource": id,
            "to_profile": other_profile_id.to_string(),
            "to_team": other_profile_id.to_string(),
            "read": true,
        })),
    )
    .await
    .expect_err("both principals refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message,
        "supply exactly one principal, not both a profile and a team"
    );

    // Input shape: no capability.
    let err = temper_mcp::tools::resources::resource_grant(
        &svc,
        &owner_parts,
        input(json!({
            "resource": id,
            "to_profile": other_profile_id.to_string(),
        })),
    )
    .await
    .expect_err("no capability refused");
    assert_eq!(code_of(&err), -32602);
    assert_eq!(
        err.message,
        "no capability selected — set at least one of read/write/grant"
    );

    // Gate: a caller who may not administer grants.
    let err = temper_mcp::tools::resources::resource_grant(
        &svc,
        &other_parts,
        input(json!({
            "resource": id,
            "to_profile": other_profile_id.to_string(),
            "read": true,
        })),
    )
    .await
    .expect_err("a non-administering caller is refused");
    assert_eq!(code_of(&err), -32602);
    assert!(
        err.message
            .contains("caller may not administer grants on this resource"),
        "got: {err}"
    );

    // The owner grants read to the other profile.
    let outcome = one_text(
        &temper_mcp::tools::resources::resource_grant(
            &svc,
            &owner_parts,
            input(json!({
                "resource": id,
                "to_profile": other_profile_id.to_string(),
                "read": true,
            })),
        )
        .await
        .expect("the owner grants"),
    );
    assert_eq!(outcome["granted"], true, "{outcome}");

    // Revoke is no-op-safe and answers its outcome.
    let revoked = one_text(
        &temper_mcp::tools::resources::resource_revoke(
            &svc,
            &owner_parts,
            input(json!({
                "resource": id,
                "from_profile": other_profile_id.to_string(),
            })),
        )
        .await
        .expect("revoke lands"),
    );
    assert_eq!(revoked["revoked"], true, "{revoked}");
}
