#![cfg(feature = "test-db")]
//! The network door's NEW arms — the faces only the wire can produce, pinned where
//! they can be produced.
//!
//! Three sets, all driven through the production relay path (real listener, real
//! bearer, real HTTP):
//!
//! 1. **Post-edge 401 voice** — Level 1 refusals that used to die at the edge now
//!    arrive as preserved 401 bodies, and `map_post_edge_auth` splits them
//!    arm-for-arm. Expired-in-flight is the face the direct binding could not produce
//!    at all (expiry used to end the request before the tool ran); deactivated and
//!    machine-credential are the terminal voices carried over word-for-word.
//! 2. **Body-limit boundary** — the gated router's inherited ceiling is the 25 MB
//!    edge contract (ruling 3, tool-carrying); a create just under crosses, a create
//!    just over meets the bare 413 the tool maps as the fault it is.
//! 3. **Resources-protocol reads** — `temper://` browsing crosses the same door: the
//!    list, the metadata read, the raw-content read, and the contexts URI whose ref
//!    resolution is absorbed server-side.
//!
//! The trust-matrix probes (§D7's faces, driven DIRECTLY at the listener with raw
//! headers — the real relay cannot drop its own headers) close the file: positive
//! control, drop-carrier, drop-credential, and the one-trust-domain differential
//! (a valid credential cannot forge `cli` attribution). The §D7 counters are tracing
//! events, so each face asserts on the captured events.

mod common;

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use common::tracing_layer::TestTracingLayer;

const ISSUER: &str = "test-issuer";

fn signing_key() -> EncodingKey {
    EncodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.key")).expect("fixture key")
}

/// A human token with a CALLER-CHOSEN expiry — the lever the expired-in-flight face
/// needs. Same shape as the harness's fixture token otherwise (same issuer, audience,
/// key), so it verifies at the edge and only its lifetime is wrong.
#[derive(Serialize)]
struct ExpiringClaims {
    sub: String,
    email: String,
    email_verified: bool,
    iss: &'static str,
    aud: String,
    iat: i64,
    exp: i64,
}

fn mint_expired_token(sub: &str, email: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = ExpiringClaims {
        sub: sub.to_string(),
        email: email.to_string(),
        email_verified: true,
        iss: ISSUER,
        aud: common::TEST_AUDIENCE.to_string(),
        iat: now - 7200,
        exp: now - 3600,
    };
    encode(&Header::new(Algorithm::RS256), &claims, &signing_key()).expect("token signs")
}

/// A machine-shaped token in the Auth0 `client_credentials` shape — `gty` is the
/// definitive signal, the `@clients` subject derivable as the client id.
#[derive(Serialize)]
struct MachineClaims {
    sub: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    azp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gty: Option<&'static str>,
    iss: &'static str,
    aud: String,
    iat: i64,
    exp: i64,
}

/// Incoherent machine: an `@clients` subject with NO grant-type declaration. The seam
/// refuses it — the API's `machine credential refused: {why}` body (ruling 7).
fn mint_incoherent_machine_token() -> String {
    let client_id = format!("witness-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().timestamp();
    let claims = MachineClaims {
        sub: format!("{client_id}@clients"),
        azp: Some(client_id),
        gty: None,
        iss: ISSUER,
        aud: common::TEST_AUDIENCE.to_string(),
        iat: now,
        exp: now + 3600,
    };
    encode(&Header::new(Algorithm::RS256), &claims, &signing_key()).expect("token signs")
}

/// Coherent machine shape, UNREGISTERED client: verifies, reaches the registration
/// gate, and is refused by the gate's own 401 voice — the machine-principal-gate arm
/// (the catch-all's terminal framing), distinct from the machine-credential arm.
fn mint_unregistered_machine_token() -> String {
    let client_id = format!("witness-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().timestamp();
    let claims = MachineClaims {
        sub: format!("{client_id}@clients"),
        azp: Some(client_id.clone()),
        gty: Some("client-credentials"),
        iss: ISSUER,
        aud: common::TEST_AUDIENCE.to_string(),
        iat: now,
        exp: now + 3600,
    };
    encode(&Header::new(Algorithm::RS256), &claims, &signing_key()).expect("token signs")
}

/// The harness service with a caller-chosen bearer riding the parts.
fn parts_for(app: &common::E2eTestApp, token: &str) -> axum::http::request::Parts {
    app.relay_parts_for(token)
}

fn code_of(err: &rmcp::ErrorData) -> i32 {
    err.code.0
}

// ── Post-edge 401 voice ─────────────────────────────────────────────

/// Expired-in-flight: the bearer verifies at the MCP edge (no expiry check on
/// synthetic parts — parts are this test's construction), crosses the wire, and dies
/// at the API's decode. The preserved 401 body "Invalid or expired token" maps to the
/// NEW re-authenticate sentence — a face the direct binding could not produce.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn expired_in_flight_speaks_the_re_authenticate_sentence(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc: TemperMcpService = app.mcp_relay_service(app.pool.clone()).await;
    let expired = mint_expired_token("e2e-test-user", "e2e@test.example.com");
    let parts = parts_for(&app, &expired);

    let err = temper_mcp::tools::resources::get_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes"),
    )
    .await
    .expect_err("an expired-in-flight bearer does not answer");

    assert_eq!(code_of(&err), -32600, "terminal, not a caller error: {err}");
    assert_eq!(
        err.message,
        "This session's token has expired. Re-authenticate (the MCP client's OAuth \
         flow will refresh it) and retry the call."
    );
}

/// The machine-credential face (ruling 7): an INCOHERENT machine token — `@clients`
/// subject, no grant-type declaration — verifies at the edge and dies at the seam's
/// refusal, whose 401 body "machine credential refused: {why}" maps to the terminal
/// machine-gate sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn machine_credential_refusal_speaks_the_machine_gate_sentence(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = parts_for(&app, &mint_incoherent_machine_token());

    let err = temper_mcp::tools::resources::get_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes"),
    )
    .await
    .expect_err("an incoherent machine principal does not answer");

    assert_eq!(code_of(&err), -32600, "terminal, not a caller error: {err}");
    assert_eq!(
        err.message,
        "This token is machine-shaped but does not declare a valid \
         client_credentials grant. This error is terminal and should not be retried."
    );
}

/// The machine-principal registration gate (G3 Phase A): a COHERENT machine token
/// naming an unregistered client reaches the gate and is refused by the gate's own
/// 401 voice, which the catch-all frames terminal — a DIFFERENT arm from the
/// machine-credential face above, witnessed so the split stays split.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_registration_gate_speaks_its_own_voice_framed_terminal(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = parts_for(&app, &mint_unregistered_machine_token());

    let err = temper_mcp::tools::resources::get_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes"),
    )
    .await
    .expect_err("an unregistered machine principal does not answer");

    assert_eq!(code_of(&err), -32600, "terminal, not a caller error: {err}");
    assert!(
        err.message.starts_with("machine client '"),
        "the gate's own voice comes through: {err}"
    );
    assert!(
        err.message.contains("is not registered with this instance"),
        "{err}"
    );
    assert!(
        err.message
            .ends_with("This error is terminal and should not be retried."),
        "framed terminal: {err}"
    );
}

/// Deactivation is adjudicated at the API on the caller's own standing: the preserved
/// 401 body "account is deactivated" maps to the terminal deactivation sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deactivated_account_speaks_the_terminal_deactivation_sentence(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();

    // Deactivate the harness principal through the standing table — the same row the
    // API's seam consults on every request.
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'deactivated' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
    )
    .bind("e2e@test.example.com")
    .execute(&app.pool)
    .await
    .expect("deactivate the principal");

    let err = temper_mcp::tools::resources::get_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes"),
    )
    .await
    .expect_err("a deactivated principal does not answer");

    assert_eq!(code_of(&err), -32600, "terminal, not a caller error: {err}");
    assert_eq!(
        err.message,
        "This account has been deactivated. This error is terminal and should not be retried."
    );
}

/// The post-edge 401 arms speak on DELETE too, not only on the read voices: the one
/// migrated tool that skipped the mapping (found in the arc-boundary review, 2026-09-24) rendered a deactivated refusal as
/// an internal fault with a CLI login hint. Every tool in the family carries the arms.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_post_edge_401_on_delete_speaks_the_arm_not_the_fault(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();

    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'deactivated' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
    )
    .bind("e2e@test.example.com")
    .execute(&app.pool)
    .await
    .expect("deactivate the principal");

    let err = temper_mcp::tools::resources::delete_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes"),
    )
    .await
    .expect_err("a deactivated principal does not answer");

    assert_eq!(
        code_of(&err),
        -32600,
        "the arm's terminal sentence, never an internal fault: {err}"
    );
    assert_eq!(
        err.message,
        "This account has been deactivated. This error is terminal and should not be retried."
    );
}

// ── Post-edge system-access refusal (Level 2, 403) ─────────────────

/// Level 2 crosses the door too: a denied-standing caller's gated request 403s at the
/// API's `require_system_access`, temper-client surfaces the typed
/// `SystemAccessRequired`, and the tool answers with the DIRECT binding's five-field
/// fidelity (email, display_name, refusal kind, request_url, cli_command) — terminal,
/// naming the identity and the remedy. This is the first refusal a caller on an
/// invite-only deployment hits; the parity suite pins the fidelity for the direct
/// binding (`auth_seam_parity_e2e`), and until this arm it fell through to an
/// internal fault saying only "system access required".
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_denied_standing_speaks_the_system_access_arm_through_the_door(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();

    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'denied' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'denied'",
    )
    .bind("e2e@test.example.com")
    .execute(&app.pool)
    .await
    .expect("deny the principal's standing");

    let err = temper_mcp::tools::resources::get_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes"),
    )
    .await
    .expect_err("a denied principal did not answer");

    assert_eq!(
        code_of(&err),
        -32600,
        "the system-access arm is terminal, never a retryable fault: {err}"
    );
    assert!(
        err.message.starts_with(
            "Access to this temper instance requires approval for e2e@test.example.com — "
        ),
        "the sentence names the identity: {err}"
    );
    assert!(
        err.message
            .contains(temper_core::types::access_gate::REQUEST_ACCESS_URL),
        "the sentence names the self-service door: {err}"
    );
    assert!(
        err.message
            .ends_with("This error is terminal and should not be retried."),
        "framed terminal: {err}"
    );
    let data = err.data.expect("the denial carries typed details");
    for field in ["email", "display_name", "request_url", "cli_command"] {
        assert!(
            !data[field].is_null(),
            "par fidelity: `{field}` survives the hop: {data}"
        );
    }
    assert_eq!(
        data["refusal"]["kind"],
        json!("denied"),
        "the typed refusal kind rides through: {data}"
    );
}

// ── Body-limit boundary ─────────────────────────────────────────────

/// The wire's ceiling is witnessed at `/api/query` — the route whose backstop rose to
/// the 25 MB edge contract (tool-carrying, ruling 3). A body a few KiB under it is
/// ADMITTED BY THE WIRE: whatever the query planner says about the payload, the
/// answer is a caller-error refusal, never the bare 413. (Ingest's own field caps are
/// a different bound — ruling 3's "the declaration caps remain the real bound" — and
/// a megabyte pad in `open_meta` meets ingest's internals, not the wire; the ceiling
/// belongs to this witness.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_body_just_under_the_ceiling_is_admitted_by_the_wire(pool: PgPool) {
    let app = common::setup_relay(pool).await;

    // The ceiling minus a wrapper-sized margin, riding a field the query planner will
    // refuse — the wire's admission is the claim, the plan's verdict is not.
    let under = 25 * 1024 * 1024 - 4096;
    let payload = json!({
        "plan": {
            "pad": "x".repeat(under),
        },
    });
    let resp = app
        .reqwest_client
        .post(app.url("/api/query"))
        .bearer_auth(&app.token)
        .json(&payload)
        .send()
        .await
        .expect("the wire answers");

    let status = resp.status().as_u16();
    assert_ne!(status, 413, "a body under the ceiling is not a 413");
    assert!(
        (400..500).contains(&status),
        "the wire admitted the body and the planner refused it as a caller error, got {status}"
    );
}

/// Just over the ceiling the door answers a bare 413 — no refusal vocabulary, and the
/// tool names the fault as the fault it is: an internal error carrying the door's
/// status, never a silent truncation.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_create_just_over_the_ceiling_meets_the_bare_413(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    let ctx = default_context_id(&app.pool).await;

    let over = 25 * 1024 * 1024 + 4096;
    let content = "x".repeat(over);
    let body = json!({
        "context_ref": ctx.to_string(),
        "doc_type_name": "session",
        "title": "Boundary: over",
        "content": content,
    });

    let err = temper_mcp::tools::resources::create_resource(
        &svc,
        &parts,
        serde_json::from_value(body).expect("input deserializes"),
    )
    .await
    .expect_err("a create over the ceiling does not land");
    assert_eq!(code_of(&err), -32603, "the door's own fault face: {err}");
    assert!(
        err.message.starts_with("Failed to create resource:"),
        "got: {err}"
    );
}

// ── Resources-protocol reads ────────────────────────────────────────

/// The protocol browse list crosses the door: one `temper://resources/{id}` URI per
/// visible resource, titles carried.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn protocol_list_browses_through_the_door(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    let ctx = default_context_id(&app.pool).await;

    let mut ids: Vec<String> = Vec::new();
    for title in ["Browsed one", "Browsed two"] {
        let res = temper_mcp::tools::resources::create_resource(
            &svc,
            &parts,
            serde_json::from_value(json!({
                "context_ref": ctx.to_string(),
                "doc_type_name": "session",
                "title": title,
            }))
            .expect("input deserializes"),
        )
        .await
        .expect("create lands");
        let v: serde_json::Value =
            serde_json::from_str(res.content[0].as_text().expect("text part").text.as_str())
                .expect("tool json");
        ids.push(v["resource"]["id"].as_str().expect("id").to_string());
    }

    let client = svc.relay_client(&parts).expect("relay client");
    let listed = temper_mcp::resources::list_resources(&client, None)
        .await
        .expect("protocol list lands");
    let uris: Vec<String> = listed.resources.iter().map(|r| r.uri.clone()).collect();
    for id in &ids {
        assert!(
            uris.contains(&format!("temper://resources/{id}")),
            "every created resource is browsable: {uris:?}"
        );
    }
}

/// The protocol read serves metadata + markdown for the resource URI and raw markdown
/// for the /content URI; the contexts URI resolves its `@me/` ref SERVER-side (the
/// absorbed resolution the gated list handler owns) and returns the context's
/// resources as JSON.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn protocol_reads_serve_metadata_content_and_context_pages(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    let ctx = default_context_id(&app.pool).await;

    let body = "The content the protocol read serves.";
    let res = temper_mcp::tools::resources::create_resource(
        &svc,
        &parts,
        serde_json::from_value(json!({
            "context_ref": ctx.to_string(),
            "doc_type_name": "session",
            "title": "Protocol read target",
            "content": body,
        }))
        .expect("input deserializes"),
    )
    .await
    .expect("create lands");
    let v: serde_json::Value =
        serde_json::from_str(res.content[0].as_text().expect("text part").text.as_str())
            .expect("tool json");
    let id = v["resource"]["id"].as_str().expect("id").to_string();

    let client = svc.relay_client(&parts).expect("relay client");

    // Metadata + content: one read, both parts of the contract in the ResourceContents.
    let full = temper_mcp::resources::read_resource(
        &client,
        rmcp::model::ReadResourceRequestParams::new(format!("temper://resources/{id}")),
    )
    .await
    .expect("metadata read lands");
    let full_json = serde_json::to_value(&full).expect("serializable");
    let contents = full_json["contents"].as_array().expect("contents array");
    assert_eq!(contents.len(), 2, "metadata part + body part: {full_json}");
    // [0] is the resource view as JSON — identity, never the prose.
    let meta: serde_json::Value =
        serde_json::from_str(contents[0]["text"].as_str().expect("meta text"))
            .expect("the first part is the resource view");
    assert_eq!(meta["id"], json!(id), "{meta}");
    assert_eq!(meta["title"], "Protocol read target", "{meta}");
    // [1] is the markdown body.
    assert_eq!(
        contents[1]["text"].as_str().expect("body text"),
        body,
        "the second part IS the markdown body"
    );

    // Raw content: the /content URI serves ONLY the markdown.
    let raw = temper_mcp::resources::read_resource(
        &client,
        rmcp::model::ReadResourceRequestParams::new(format!("temper://resources/{id}/content")),
    )
    .await
    .expect("content read lands");
    let raw_text = serde_json::to_value(&raw).expect("serializable");
    let raw_text = raw_text["contents"][0]["text"].as_str().expect("text");
    assert_eq!(raw_text, body, "the /content URI is the raw markdown");

    // Contexts URI: the `@me/…` ref is resolved by the API's list handler against the
    // caller's own visibility — this surface resolves nothing locally. The URI is the
    // bare template expansion; the parse-shape guard takes `@me/default` whole.
    let page = temper_mcp::resources::read_resource(
        &client,
        rmcp::model::ReadResourceRequestParams::new("temper://contexts/@me/default/resources"),
    )
    .await
    .expect("the contexts URI lands");
    let page_text = serde_json::to_value(&page).expect("serializable");
    let page_text = page_text["contents"][0]["text"].as_str().expect("text");
    assert!(
        page_text.contains(&id),
        "the context page names the created resource: {page_text}"
    );
}

// ── The trust matrix, driven at the listener ────────────────────────

/// One raw POST /api/ingest with caller-chosen credential/carrier headers, the
/// harness bearer on Authorization. Returns (status, status-body-snippet).
async fn raw_ingest(
    app: &common::E2eTestApp,
    credential: Option<&str>,
    carrier: Option<&str>,
) -> (u16, String) {
    let payload = serde_json::json!({
        "title": "Probe",
        "origin_uri": "mcp://test/probe",
        "context_ref": default_context_id(&app.pool).await.to_string(),
        "doc_type_name": "session",
        "content": "",
        "act": {},
        "sources": []
    });
    let mut req = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .bearer_auth(&app.token)
        .json(&payload);
    if let Some(c) = credential {
        req = req.header(temper_workflow::operations::SERVICE_CREDENTIAL_HEADER, c);
    }
    if let Some(c) = carrier {
        req = req.header(temper_workflow::operations::RELAYED_SURFACE_HEADER, c);
    }
    let resp = req.send().await.expect("probe request answers");
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

fn degraded_events(events: &[common::tracing_layer::CapturedEvent]) -> Vec<Option<String>> {
    events
        .iter()
        .filter(|e| {
            e.fields
                .get("counter")
                .map(|c| c.contains("relayed_surface_degraded"))
                .unwrap_or(false)
        })
        .map(|e| e.fields.get("present").cloned())
        .collect()
}

fn trusted_count(events: &[common::tracing_layer::CapturedEvent]) -> usize {
    events
        .iter()
        .filter(|e| {
            e.fields
                .get("counter")
                .map(|c| c.contains("relayed_surface_trusted"))
                .unwrap_or(false)
        })
        .count()
}

/// Positive control: valid credential + the one allowed carrier → the trusted event,
/// and the act lands. Without this arm the others prove only absence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_positive_control_is_trusted_and_lands(pool: PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();
    let app = common::setup_relay(pool).await;

    let (status, _body) =
        raw_ingest(&app, Some(common::TEST_MCP_SERVICE_SECRET), Some("mcp")).await;
    assert_eq!(status, 200, "the trusted act lands: {_body}");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = captured.lock().unwrap();
    assert!(
        trusted_count(&events) >= 1,
        "the carrier is honored: {events:?}"
    );
}

/// Drop the carrier, keep the credential: the relay-shaped credential holder that
/// sends no carrier is a degrade (present=false) — the relay misbehaving mid-deploy —
/// and the act lands unattributed. No trusted event: dropping the carrier cannot keep
/// MCP attribution (the bite: honor-on-credential-alone reddens here).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dropping_the_carrier_degrades_and_never_trusts(pool: PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();
    let app = common::setup_relay(pool).await;

    let (status, _body) = raw_ingest(&app, Some(common::TEST_MCP_SERVICE_SECRET), None).await;
    assert_eq!(status, 200, "the act lands regardless: {_body}");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = captured.lock().unwrap();
    assert_eq!(
        trusted_count(&events),
        0,
        "no carrier, no trust: {events:?}"
    );
    assert_eq!(
        degraded_events(&events),
        vec![Some("false".to_string())],
        "the credential-bearing carrier-less request is the present=false degrade: {events:?}"
    );
}

/// Drop the credential, keep the carrier — every spoofing attempt's shape: the
/// degrade fires (the §D7 load-bearing signal), the act lands authenticated on the
/// bearer alone, and nothing is trusted. The bite: any honor-on-carrier-alone
/// mutation reddens the trusted_count assertion.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dropping_the_credential_degrades_and_never_trusts(pool: PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();
    let app = common::setup_relay(pool).await;

    let (status, _body) = raw_ingest(&app, None, Some("mcp")).await;
    assert_eq!(status, 200, "auth rides the bearer alone: {_body}");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = captured.lock().unwrap();
    assert_eq!(
        trusted_count(&events),
        0,
        "no credential, no trust: {events:?}"
    );
    assert_eq!(
        degraded_events(&events),
        vec![None],
        "the uncredentialed carrier is the degrade without a `present` field: {events:?}"
    );
}

/// The one-trust-domain differential: a VALID credential with carrier `cli` is
/// degraded, never trusted — the service secret's holder can claim the one value the
/// allowlist admits (`mcp`) and nothing else, so a stolen secret cannot forge CLI
/// attribution.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_valid_credential_cannot_forge_cli_attribution(pool: PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();
    let app = common::setup_relay(pool).await;

    let (status, _body) =
        raw_ingest(&app, Some(common::TEST_MCP_SERVICE_SECRET), Some("cli")).await;
    assert_eq!(status, 200, "the act still lands: {_body}");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let events = captured.lock().unwrap();
    assert_eq!(
        trusted_count(&events),
        0,
        "the allowlist admits exactly one value: {events:?}"
    );
    assert_eq!(
        degraded_events(&events),
        vec![Some("true".to_string())],
        "the off-allowlist carrier is the present=true degrade: {events:?}"
    );
}

// ── Local helpers ───────────────────────────────────────────────────

async fn default_context_id(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c \
         JOIN kb_profiles p ON p.id = c.owner_id \
         WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind("e2e@test.example.com")
    .fetch_one(pool)
    .await
    .expect("the auto-provisioned default context")
}
