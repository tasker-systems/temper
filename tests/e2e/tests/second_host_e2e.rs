#![cfg(feature = "test-db")]
//! **A second host, against the real API.** The deployed door's tool service and a test host
//! built only on the tool layer's public API (`TemperMcpService`, `RelayConfig`, `IdentitySeam`)
//! relay to the same listener; this suite holds them to the goal's two-host clauses:
//!
//! - **`refusals-render-once`** — one case per post-edge refusal arm the wire can produce
//!   (expired-in-flight, machine credential, registration gate, system access, deactivation)
//!   renders byte-identical from both hosts.
//! - **`no-host-written-authorship`** — a relayed write with a full agent envelope reads back
//!   byte-identical on the trail, and one without an envelope reads back with none, through
//!   both hosts.
//! - **The seam is the outgoing credential** — the deployed door's acts attribute to
//!   `<handle>@mcp` (its service credential and carrier); the test host's, relaying the
//!   person's bearer at the plain client surface with no extra headers, to `<handle>@cli`.
//! - A host `correlation_id` threads an act the agent left uncorrelated, and never overrides
//!   the agent's own.

mod common;

use std::sync::Arc;

use serde_json::json;
use sqlx::{PgPool, Row};
use temper_core::types::authorship::ActInput;
use temper_core::types::ConfidenceBand;
use temper_mcp::{IdentitySeam, OutgoingIdentity, RelayConfig, TemperMcpService};
use temper_workflow::operations::Surface;

/// The test host's seam: a credential store holding the person's bearer, at the plain client
/// surface, no extra headers — and, when the host threads its conversations, a correlation id.
struct PersonSeam {
    bearer: Option<String>,
    correlation_id: Option<uuid::Uuid>,
}

impl IdentitySeam for PersonSeam {
    fn outgoing_identity(&self, _parts: &http::request::Parts) -> Option<OutgoingIdentity> {
        let identity = OutgoingIdentity::new(self.bearer.clone()?, Surface::CliCloud);
        Some(match self.correlation_id {
            Some(id) => identity.with_correlation_id(id),
            None => identity,
        })
    }
}

fn test_host(
    app: &common::E2eTestApp,
    bearer: Option<&str>,
    correlation_id: Option<uuid::Uuid>,
) -> TemperMcpService {
    TemperMcpService::new(
        temper_mcp_server::config::blob_door(None, false),
        RelayConfig::new(app.base_url(), std::time::Duration::from_secs(30)),
        Arc::new(PersonSeam {
            bearer: bearer.map(str::to_string),
            correlation_id,
        }),
    )
}

/// Request parts with nothing in them — the test host's credential does not ride the request.
fn empty_parts() -> http::request::Parts {
    http::Request::new(()).into_parts().0
}

/// The same read through both hosts on `token`, each answering a refusal; returns both, rendered.
async fn refusal_from_both(app: &common::E2eTestApp, token: &str) -> (Vec<u8>, Vec<u8>) {
    let input = || {
        serde_json::from_value(json!({ "id": uuid::Uuid::now_v7().to_string() }))
            .expect("input deserializes")
    };
    let deployed = temper_mcp::tools::resources::get_resource(
        &app.mcp_relay_service().await,
        &app.relay_parts_for(token),
        input(),
    )
    .await
    .expect_err("the deployed door refuses");
    let second = temper_mcp::tools::resources::get_resource(
        &test_host(app, Some(token), None),
        &empty_parts(),
        input(),
    )
    .await
    .expect_err("the second host refuses");
    (
        serde_json::to_vec(&deployed).unwrap(),
        serde_json::to_vec(&second).unwrap(),
    )
}

/// An `@clients` subject with no grant-type declaration — the API refuses it as an incoherent
/// machine credential (`machine credential refused: …`).
fn mint_incoherent_machine_token() -> String {
    #[derive(serde::Serialize)]
    struct Claims {
        sub: String,
        azp: String,
        iss: &'static str,
        aud: &'static str,
        iat: i64,
        exp: i64,
    }
    let client_id = format!("second-host-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().timestamp();
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &Claims {
            sub: format!("{client_id}@clients"),
            azp: client_id,
            iss: "test-issuer",
            aud: common::TEST_AUDIENCE,
            iat: now,
            exp: now + 3600,
        },
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.key"))
            .expect("fixture key"),
    )
    .expect("token signs")
}

async fn set_standing(pool: &PgPool, state: &'static str) {
    let sql = match state {
        "denied" => {
            "INSERT INTO kb_principal_standing (profile_id, state)
             SELECT id, 'denied' FROM kb_profiles WHERE email = $1
             ON CONFLICT (profile_id) DO UPDATE SET state = 'denied'"
        }
        "deactivated" => {
            "INSERT INTO kb_principal_standing (profile_id, state)
             SELECT id, 'deactivated' FROM kb_profiles WHERE email = $1
             ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'"
        }
        other => panic!("no standing case for {other}"),
    };
    sqlx::query(sql)
        .bind("e2e@test.example.com")
        .execute(pool)
        .await
        .expect("set the principal's standing");
}

/// **`refusals-render-once`:** each arm, byte-identical from both hosts. Each case also checks
/// it hit the arm it names, so a case that silently degraded to another arm cannot pass.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_refusal_arm_renders_byte_identical_from_both_hosts(pool: PgPool) {
    let app = common::setup_relay(pool).await;

    let token_cases = [
        (
            "expired-in-flight",
            common::generate_expired_jwt("e2e-test-user", "e2e@test.example.com"),
            "This session's token has expired.",
        ),
        (
            "machine credential",
            mint_incoherent_machine_token(),
            temper_mcp::service::TERMINAL_MACHINE_GATE_SENTENCE,
        ),
        (
            "registration gate",
            common::generate_machine_jwt(&format!("unregistered-{}", uuid::Uuid::new_v4())),
            "machine client '",
        ),
    ];
    for (arm, token, prefix) in &token_cases {
        let (deployed, second) = refusal_from_both(&app, token).await;
        assert_eq!(
            String::from_utf8_lossy(&deployed),
            String::from_utf8_lossy(&second),
            "the {arm} arm renders differently from the two hosts"
        );
        let rendered: serde_json::Value = serde_json::from_slice(&deployed).unwrap();
        assert!(
            rendered["message"].as_str().unwrap().starts_with(prefix),
            "the {arm} case did not reach its arm: {rendered}"
        );
    }

    // The standing arms act on the harness principal itself: denied, then deactivated.
    for (arm, state, prefix) in [
        (
            "system access",
            "denied",
            "Access to this temper instance requires approval for e2e@test.example.com",
        ),
        (
            "deactivation",
            "deactivated",
            temper_mcp::service::TERMINAL_DEACTIVATION_SENTENCE,
        ),
    ] {
        set_standing(&app.pool, state).await;
        let (deployed, second) = refusal_from_both(&app, &app.token).await;
        assert_eq!(
            String::from_utf8_lossy(&deployed),
            String::from_utf8_lossy(&second),
            "the {arm} arm renders differently from the two hosts"
        );
        let rendered: serde_json::Value = serde_json::from_slice(&deployed).unwrap();
        assert!(
            rendered["message"].as_str().unwrap().starts_with(prefix),
            "the {arm} case did not reach its arm: {rendered}"
        );
    }

    // And the crate's own: a host that supplies no identity.
    let deployed = app
        .mcp_relay_service()
        .await
        .relay_client(&empty_parts())
        .unwrap_err();
    let second = test_host(&app, None, None)
        .relay_client(&empty_parts())
        .unwrap_err();
    assert_eq!(
        serde_json::to_vec(&deployed).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(deployed.message, temper_mcp::seam::NOT_CONNECTED_SENTENCE);
}

/// The five agent-authorship keys as `kb_events.metadata` holds them for one event.
const AUTHORSHIP_KEYS: [&str; 5] = ["confidence", "reasoning", "rationale", "persona", "model"];

struct Landed {
    actor_name: String,
    trail_confidence: Option<String>,
    authorship: serde_json::Map<String, serde_json::Value>,
}

/// Create one resource through `svc` and read its `resource_created` event back off the trail
/// (through the same host's `element_trail` tool), plus the event's stored authorship.
async fn create_and_read_back(
    app: &common::E2eTestApp,
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    title: &str,
    act: ActInput,
) -> Landed {
    let input = temper_mcp::tools::resources::CreateResourceInput {
        goal: None,
        context_ref: Some("@me/second-host".to_string()),
        cogmap: None,
        idempotency_key: None,
        doc_type_name: "research".to_string(),
        title: title.to_string(),
        content: None,
        sources: None,
        origin_uri: None,
        owner: None,
        managed_meta: None,
        open_meta: None,
        act,
    };
    let created = temper_mcp::tools::resources::create_resource(svc, parts, input)
        .await
        .expect("the relayed create lands");
    let created: serde_json::Value = serde_json::from_str(
        created.content[0]
            .as_text()
            .expect("the create answers text")
            .text
            .as_str(),
    )
    .expect("the create answers JSON");
    let id = created["resource"]["id"]
        .as_str()
        .expect("the created resource has an id");

    let trail = temper_mcp::tools::trail::element_trail(
        svc,
        parts,
        serde_json::from_value(json!({ "kind": "node", "element": id })).unwrap(),
    )
    .await
    .expect("the trail reads through the same host");
    let trail: temper_core::types::element_trail::EventTrail =
        serde_json::from_str(trail.content[0].as_text().unwrap().text.as_str()).unwrap();
    let event = trail
        .events
        .iter()
        .find(|e| e.kind == "resource_created")
        .expect("the trail carries the create");

    let metadata: serde_json::Value = sqlx::query("SELECT metadata FROM kb_events WHERE id = $1")
        .bind(event.event_id)
        .fetch_one(&app.pool)
        .await
        .expect("the event row")
        .get("metadata");
    let authorship = AUTHORSHIP_KEYS
        .iter()
        .filter_map(|k| metadata.get(*k).map(|v| ((*k).to_string(), v.clone())))
        .collect();

    Landed {
        actor_name: event.actor_name.clone(),
        trail_confidence: event.confidence.clone(),
        authorship,
    }
}

fn full_envelope() -> ActInput {
    ActInput {
        reasoning: Some("the agent's own reasoning — verbatim, «unicode» and all".to_string()),
        confidence: Some(ConfidenceBand::Tentative),
        rationale: Some("{\"why\": [\"structured\", 1]}".to_string()),
        persona: Some("auditor".to_string()),
        model: Some("an-agent-model-id".to_string()),
        ..Default::default()
    }
}

/// **`no-host-written-authorship`, through both hosts — and each host's attribution.**
///
/// FAILS IF: either host fills, defaults, rewrites or drops an authorship field; or the deployed
/// door stops attributing `@mcp` (its credential and carrier); or the test host's act attributes
/// anywhere but the person at the plain client surface.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn authorship_crosses_both_hosts_exactly_as_the_agent_sent_it(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    app.client
        .contexts()
        .create("second-host", None)
        .await
        .expect("context create");
    let handle = app.client.profile().get().await.expect("profile").slug;

    let deployed = app.mcp_relay_service().await;
    let deployed_parts = app.relay_parts();
    let second = test_host(&app, Some(&app.token), None);
    let second_parts = empty_parts();

    let sent = full_envelope();
    let expected: serde_json::Map<String, serde_json::Value> = [
        ("confidence", json!("tentative")),
        ("reasoning", json!(sent.reasoning)),
        ("rationale", json!(sent.rationale)),
        ("persona", json!(sent.persona)),
        ("model", json!(sent.model)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();

    for (host, svc, parts, surface) in [
        ("deployed", &deployed, &deployed_parts, "mcp"),
        ("second", &second, &second_parts, "cli"),
    ] {
        let with = create_and_read_back(
            &app,
            svc,
            parts,
            &format!("{host}: with an envelope"),
            full_envelope(),
        )
        .await;
        assert_eq!(
            with.authorship, expected,
            "{host}: the envelope must land exactly as the agent sent it"
        );
        assert_eq!(
            with.trail_confidence.as_deref(),
            Some("tentative"),
            "{host}"
        );
        assert_eq!(
            with.actor_name,
            format!("{handle}@{surface}"),
            "{host}: the act attributes to the identity this host's seam supplies"
        );

        let without = create_and_read_back(
            &app,
            svc,
            parts,
            &format!("{host}: without an envelope"),
            ActInput::default(),
        )
        .await;
        assert!(
            without.authorship.is_empty(),
            "{host}: an act sent without authorship crosses without it: {:?}",
            without.authorship
        );
        assert_eq!(without.trail_confidence, None, "{host}");
        assert_eq!(without.actor_name, format!("{handle}@{surface}"), "{host}");
    }
}

/// A host's `correlation_id` threads an act the agent left uncorrelated, through the dispatch
/// path a real call takes (`call_tool`); the agent's own correlation id always wins.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_host_correlation_id_threads_only_what_the_agent_left_unthreaded(pool: PgPool) {
    let app = common::setup_relay(pool).await;
    app.client
        .contexts()
        .create("second-host", None)
        .await
        .expect("context create");
    let host_thread = uuid::Uuid::now_v7();
    let agent_thread = uuid::Uuid::now_v7();
    let svc = test_host(&app, Some(&app.token), Some(host_thread));

    // Drive the real MCP dispatch (where the fill happens) over rmcp's in-memory transport.
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let server = svc.clone();
    tokio::spawn(async move {
        let running = rmcp::serve_server(server, server_io).await.expect("serve");
        let _ = running.waiting().await;
    });
    let client = rmcp::serve_client((), client_io).await.expect("client");

    let mut created_ids = Vec::new();
    for agent_correlation in [None, Some(agent_thread)] {
        let mut args = json!({
            "context_ref": "@me/second-host",
            "doc_type_name": "research",
            "title": format!("threaded by {agent_correlation:?}"),
        });
        if let Some(id) = agent_correlation {
            args["correlation_id"] = json!(id.to_string());
        }
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("create_resource")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .expect("the create dispatches");
        let created: serde_json::Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        created_ids.push(created["resource"]["id"].as_str().unwrap().to_string());
    }

    for (id, want) in created_ids.iter().zip([host_thread, agent_thread]) {
        let trail = app
            .client
            .events()
            .element_trail(
                temper_core::types::element_trail::ElementKind::Node,
                id.parse().unwrap(),
            )
            .await
            .expect("trail");
        let created = trail
            .events
            .iter()
            .find(|e| e.kind == "resource_created")
            .expect("the create is on the trail");
        assert_eq!(created.correlation_id, Some(want));
    }
    client.cancel().await.ok();
}
