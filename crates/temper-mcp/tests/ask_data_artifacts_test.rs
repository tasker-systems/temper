//! `ask_data_artifacts`, driven through the tool's own door (`tools::data_artifacts::ask_artifacts`)
//! against a stand-in API on a loopback listener.
//!
//! The stand-in answers `POST /api/data-artifacts/ask` with a canned answer — one page, or the
//! `PLAN_REFUSED` body the route renders — and records the bytes it was sent, so what the door
//! forwarded can be read back. (A router is assembled here, outside `src/`, where
//! `audit-mcp-route-auth.sh` forbids one.)
//!
//! What is under test is the door: that it relays the request unread, answers the page once, as
//! compact text, and carries a refusal's list, `teaches` included, in the error's `data`. The
//! three doors against the real API are `tests/e2e/tests/data_artifact_ask_doors_e2e.rs`.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use rmcp::model::ErrorCode;
use serde_json::{json, Value};
use temper_core::types::artifact_ask::{
    AskOrder, AskResponse, AskedShape, OversizedArtifact, RefusalTeaching,
};
use temper_core::types::data_artifact::{ArtifactCursor, ArtifactView};
use temper_core::types::error_details::{ErrorBody, ErrorDetail, ErrorDetails, PlanRefusalDetails};
use temper_core::types::ids::{DataArtifactId, ResourceId, ShapeId};
use temper_core::types::query::{ArtifactAskPopulation, PlanRefusal, RefusalReason};
use temper_mcp::tools::data_artifacts::{ask_artifacts, AskInput};
use temper_mcp::{BlobDoor, IdentitySeam, OutgoingIdentity, RelayConfig, TemperMcpService};
use temper_workflow::operations::Surface;
use uuid::Uuid;

/// What the stand-in answers, and every body it was sent.
struct StandIn {
    status: axum::http::StatusCode,
    canned: Value,
    received: Mutex<Vec<Vec<u8>>>,
}

async fn answer(State(stand_in): State<Arc<StandIn>>, body: axum::body::Bytes) -> Response {
    stand_in.received.lock().expect("lock").push(body.to_vec());
    (stand_in.status, axum::Json(stand_in.canned.clone())).into_response()
}

async fn stand_in(status: axum::http::StatusCode, canned: Value) -> (Arc<StandIn>, String) {
    let stand_in = Arc::new(StandIn {
        status,
        canned,
        received: Mutex::new(Vec::new()),
    });
    let app = axum::Router::new()
        .route("/api/data-artifacts/ask", axum::routing::post(answer))
        .with_state(stand_in.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback binds");
    let addr = listener.local_addr().expect("bound address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("stand-in serves") });
    (stand_in, format!("http://{addr}"))
}

struct Bearer;

impl IdentitySeam for Bearer {
    fn outgoing_identity(&self, _parts: &http::request::Parts) -> Option<OutgoingIdentity> {
        Some(OutgoingIdentity::new("tok", Surface::CliCloud))
    }
}

fn relaying_to(base_url: &str) -> TemperMcpService {
    TemperMcpService::new(
        BlobDoor::Closed {
            refusal: "unused".to_string(),
        },
        RelayConfig::new(base_url, std::time::Duration::from_secs(10)),
        Arc::new(Bearer),
    )
}

fn empty_parts() -> http::request::Parts {
    http::Request::new(()).into_parts().0
}

fn created() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-24T09:15:30.123456Z")
        .expect("literal parses")
        .with_timezone(&Utc)
}

/// One page: an artifact whose content holds a quote, a backslash and a newline, an oversized
/// entry, the population and a continuation.
fn page() -> AskResponse {
    AskResponse {
        shape: AskedShape {
            shape_id: ShapeId::from(Uuid::from_u128(7)),
            shape_version: 1,
            home_anchor_table: "kb_contexts".to_string(),
            home_anchor_id: Uuid::from_u128(1),
        },
        artifacts: vec![ArtifactView {
            artifact_id: DataArtifactId::from(Uuid::from_u128(10)),
            resource_id: ResourceId::from(Uuid::from_u128(11)),
            kind_owner_table: "kb_profiles".to_string(),
            kind_owner_id: Uuid::from_u128(2),
            artifact_kind: "measurement".to_string(),
            intent: "member".to_string(),
            precedence: 0.0,
            content_hash: "0".repeat(64),
            content_bytes: 36,
            shape_state: "declared_satisfied".to_string(),
            is_folded: false,
            created: created(),
            content: Some(json!({ "unit": "ms", "note": "a \"b\" \\ c\nd" })),
        }],
        oversized: vec![OversizedArtifact {
            artifact_id: DataArtifactId::from(Uuid::from_u128(12)),
            resource_id: ResourceId::from(Uuid::from_u128(11)),
            content_bytes: 3_000_000,
        }],
        population: Some(ArtifactAskPopulation {
            declared_satisfied: 2,
            declared_not_satisfied: 0,
            declared_not_yet_checked: 0,
            matched: 2,
        }),
        ordered_by: AskOrder::Created,
        next: Some(ArtifactCursor {
            created: created(),
            artifact_id: DataArtifactId::from(Uuid::from_u128(12)),
        }),
        amended_since: None,
    }
}

/// **The tool relays the request unread and answers the page once, as compact text.** The
/// arguments carry a field no `AskRequest` declares; the stand-in receives exactly the arguments,
/// that field included, and answers one page. The result is one text block, no structured content,
/// with no newline in it (the content's own newline travels escaped), and it reads back as the
/// page. Bites: the tool deserializing `AskRequest` itself (the field is dropped before the relay,
/// or the call refused at the edge), `to_string_pretty` in `ask_result`, or the page also sent as
/// structured content.
#[tokio::test]
async fn the_tool_relays_the_ask_unread_and_answers_the_page_once_compactly() {
    let page = page();
    let (stand_in, base) = stand_in(
        axum::http::StatusCode::OK,
        serde_json::to_value(&page).expect("the page serializes"),
    )
    .await;
    let args = json!({
        "ask": { "shape": Uuid::from_u128(7), "predicates": [
            { "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } }
        ] },
        "limit": 2,
        "not_a_field": [1, 2]
    });

    let result = ask_artifacts(&relaying_to(&base), &empty_parts(), AskInput(args.clone()))
        .await
        .expect("answered");

    let received = stand_in.received.lock().expect("lock").clone();
    assert_eq!(received.len(), 1, "one relay");
    let sent: Value = serde_json::from_slice(&received[0]).expect("the relay sent JSON");
    assert_eq!(
        sent, args,
        "the arguments, unread, the undeclared field included"
    );

    assert_eq!(result.content.len(), 1, "one text block");
    assert!(result.structured_content.is_none(), "sent once");
    let text = &result.content[0].as_text().expect("a text block").text;
    assert!(!text.contains('\n'), "compact: {text}");
    assert_eq!(
        text,
        &serde_json::to_string(&page).expect("serializes"),
        "the page's compact JSON, exactly"
    );
    let back: AskResponse = serde_json::from_str(text).expect("reads back as an AskResponse");
    assert_eq!(
        serde_json::to_value(back).expect("serializes"),
        serde_json::to_value(&page).expect("serializes")
    );
}

/// The `PLAN_REFUSED` body the route renders for `refusals`.
fn refused_body(refusals: Vec<PlanRefusal>) -> Value {
    serde_json::to_value(ErrorBody {
        error: ErrorDetail {
            code: temper_core::error::PLAN_REFUSED_CODE.to_string(),
            message: format!("Plan refused: {} refusal(s)", refusals.len()),
            details: Some(
                serde_json::to_value(ErrorDetails::PlanRefusals(PlanRefusalDetails { refusals }))
                    .expect("details serialize"),
            ),
        },
    })
    .expect("the body serializes")
}

/// **A refused ask is `invalid_params` naming the ask, with every refusal as `data`, `teaches`
/// intact** — the rendering `run_query` shares. An ask refusal that teaches and a paging refusal
/// arrive together, in order. Bites: `data` passed as `None`, or the prose naming a plan.
#[tokio::test]
async fn a_refused_ask_carries_every_refusal_with_its_teaching_as_data() {
    let refusals = vec![
        PlanRefusal {
            stage: None,
            reason: RefusalReason::ValueOutsideShape,
            detail: "a value lies outside the shape".to_string(),
            teaches: Some(Box::new(RefusalTeaching {
                non_closure: Some("stored content is not closed by its shape".to_string()),
                ..RefusalTeaching::default()
            })),
        },
        PlanRefusal {
            stage: None,
            reason: RefusalReason::PageLimitOutOfRange,
            detail: "`limit` must be at least 1".to_string(),
            teaches: None,
        },
    ];
    let (_, base) = stand_in(
        axum::http::StatusCode::BAD_REQUEST,
        refused_body(refusals.clone()),
    )
    .await;

    let err = ask_artifacts(
        &relaying_to(&base),
        &empty_parts(),
        AskInput(json!({ "ask": { "shape": Uuid::from_u128(7) }, "limit": 0 })),
    )
    .await
    .expect_err("refused");

    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("ask refused"), "{}", err.message);
    assert!(!err.message.contains("plan"), "{}", err.message);
    let data: PlanRefusalDetails =
        serde_json::from_value(err.data.expect("the refusals ride data")).expect("typed");
    assert_eq!(
        data.refusals, refusals,
        "every refusal, teaches intact, in order"
    );
}
