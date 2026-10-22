//! `list_data_artifacts` with and without `limit`, driven through the tool's own door
//! (`tools::data_artifacts::list_artifacts`) against a stand-in API on a loopback listener.
//!
//! The stand-in answers `GET /api/resources/{id}/artifacts` the way the route does: with no
//! `limit`, every artifact as a bare array; with `limit`, one page in `(created, id)` order in the
//! asked direction, the bare array again, and the next position in a `Link: rel="next"` header
//! when more remain. It records every query it is asked, so what the door forwarded can be read
//! back. (A router is assembled here, outside `src/`, where `audit-mcp-route-auth.sh` forbids one.)
//!
//! What is under test is the door: that `limit` is the demarcation between the unchanged bare
//! array and the `{artifacts, next}` envelope, that the paging refusals are the route's own words
//! and are spoken before anything is relayed, and that `next` passed back as `after` walks every
//! artifact exactly once in both orders, across a page boundary that falls between two artifacts
//! sharing one `created`.

use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use rmcp::model::{CallToolResult, ErrorCode};
use serde_json::{json, Value};
use temper_core::types::data_artifact::{
    ArtifactListParams, ArtifactPageRefusal, ArtifactView, ARTIFACT_DRAIN_PAGE_ROWS,
    MAX_ARTIFACT_PAGE_ROWS,
};
use temper_core::types::ids::{DataArtifactId, ResourceId};
use temper_core::types::resource::SortOrder;
use temper_mcp::tools::data_artifacts::{list_artifacts, ListArtifactsInput};
use temper_mcp::{BlobDoor, IdentitySeam, OutgoingIdentity, RelayConfig, TemperMcpService};
use temper_workflow::operations::Surface;
use uuid::Uuid;

const RESOURCE: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0001);

/// The stand-in API: a fixed set of artifacts, and every query it was asked.
struct StandIn {
    rows: Vec<ArtifactView>,
    asked: Mutex<Vec<ArtifactListParams>>,
}

/// The `(created, id)` position of a row: the keyset the route orders and continues by.
fn position(row: &ArtifactView) -> (DateTime<Utc>, Uuid) {
    (row.created, row.artifact_id.uuid())
}

async fn answer(
    State(stand_in): State<Arc<StandIn>>,
    Query(q): Query<ArtifactListParams>,
) -> Response {
    stand_in.asked.lock().expect("lock").push(q.clone());
    let mut rows = stand_in.rows.clone();
    rows.sort_by_key(position);
    let request = match q.page_request() {
        Ok(Some(request)) => request,
        Ok(None) => return axum::Json(rows).into_response(),
        Err(refusal) => {
            return (axum::http::StatusCode::BAD_REQUEST, refusal.to_string()).into_response()
        }
    };
    if request.order == SortOrder::Desc {
        rows.reverse();
    }
    let after = request.after.map(|c| (c.created, c.artifact_id.uuid()));
    let beyond: Vec<ArtifactView> = rows
        .into_iter()
        .filter(|row| match (after, request.order) {
            (None, _) => true,
            (Some(after), SortOrder::Asc) => position(row) > after,
            (Some(after), SortOrder::Desc) => position(row) < after,
        })
        .collect();
    let limit = request.limit as usize;
    let page: Vec<ArtifactView> = beyond.iter().take(limit).cloned().collect();
    if beyond.len() <= limit {
        return axum::Json(page).into_response();
    }
    let last = page.last().expect("a full page is not empty");
    let link = format!(
        "</api/resources/{RESOURCE}/artifacts?limit={limit}&after_created={}&after_id={}&order={}>; rel=\"next\"",
        last.created.to_rfc3339_opts(SecondsFormat::AutoSi, true),
        last.artifact_id.uuid(),
        match request.order {
            SortOrder::Asc => "asc",
            SortOrder::Desc => "desc",
        },
    );
    ([(axum::http::header::LINK, link)], axum::Json(page)).into_response()
}

/// Serve `rows` on a loopback listener; answers the stand-in (to read back what it was asked) and
/// its base URL.
async fn stand_in(rows: Vec<ArtifactView>) -> (Arc<StandIn>, String) {
    let stand_in = Arc::new(StandIn {
        rows,
        asked: Mutex::new(Vec::new()),
    });
    let app = axum::Router::new()
        .route("/api/resources/{id}/artifacts", axum::routing::get(answer))
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

/// One artifact at `micros` past a fixed second, with an id that orders by `n`.
fn artifact(n: u128, micros: i64) -> ArtifactView {
    let second = DateTime::parse_from_rfc3339("2026-10-19T12:30:45Z")
        .expect("literal parses")
        .with_timezone(&Utc);
    ArtifactView {
        artifact_id: DataArtifactId::from(Uuid::from_u128(
            0x0199_0000_0000_7000_8000_0000_0000_1000 + n,
        )),
        resource_id: ResourceId::from(RESOURCE),
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: Uuid::from_u128(1),
        artifact_kind: "measurement".to_string(),
        intent: "member".to_string(),
        precedence: 0.0,
        content_hash: format!("hash-{n}"),
        content_bytes: 1,
        shape_state: "never_declared".to_string(),
        is_folded: false,
        created: second + chrono::Duration::microseconds(micros),
        content: Some(json!({ "n": n as u64 })),
    }
}

/// Six artifacts inside one second, two of them sharing a `created`. With a page of two, the
/// ascending walk's first boundary falls between the tied pair (rows 1|2), and so does the
/// descending walk's second (rows 2|1). All six share a second, so a position that lost its
/// sub-second digits would land every continuation at the same place.
fn six_with_a_tie() -> Vec<ArtifactView> {
    vec![
        artifact(0, 100),
        artifact(1, 300),
        artifact(2, 300),
        artifact(3, 400),
        artifact(4, 500),
        artifact(5, 600),
    ]
}

fn input(args: Value) -> ListArtifactsInput {
    serde_json::from_value(args).expect("input deserializes")
}

/// The one text block a result carries, parsed. Fails if there is not exactly one.
fn only_block(result: &CallToolResult) -> Value {
    assert_eq!(result.content.len(), 1, "exactly one content block");
    serde_json::from_str(&result.content[0].as_text().expect("a text block").text)
        .expect("the block is JSON")
}

fn ids(artifacts: &Value) -> Vec<String> {
    artifacts
        .as_array()
        .expect("an array of artifacts")
        .iter()
        .map(|a| a["artifact_id"].as_str().expect("an id").to_string())
        .collect()
}

/// Walk the list a page of two at a time, passing each `next` back as `after` exactly as an agent
/// would. Answers the ids in the order visited and the number of pages.
async fn walk(svc: &TemperMcpService, order: Option<&str>) -> (Vec<String>, usize) {
    let mut visited = Vec::new();
    let mut after: Option<Value> = None;
    for pages in 1..=10 {
        let mut args = json!({ "resource_id": RESOURCE.to_string(), "limit": 2 });
        if let Some(order) = order {
            args["order"] = json!(order);
        }
        if let Some(after) = &after {
            args["after"] = after.clone();
        }
        let envelope = only_block(
            &list_artifacts(svc, &empty_parts(), input(args))
                .await
                .expect("a page answers"),
        );
        visited.extend(ids(&envelope["artifacts"]));
        if envelope["next"].is_null() {
            return (visited, pages);
        }
        after = Some(envelope["next"].clone());
    }
    panic!("the walk did not end within ten pages: {visited:?}");
}

fn expected_ids(order: SortOrder) -> Vec<String> {
    let mut rows = six_with_a_tie();
    rows.sort_by_key(position);
    if order == SortOrder::Desc {
        rows.reverse();
    }
    rows.iter()
        .map(|r| r.artifact_id.uuid().to_string())
        .collect()
}

/// Without `limit` the answer is unchanged: one block, the bare array of every artifact, across
/// more than one drain page. FAILS IF the no-`limit` path starts paging: an envelope is not an
/// array, and a single default page would stop at `ARTIFACT_DRAIN_PAGE_ROWS`.
#[tokio::test]
async fn without_limit_the_answer_is_every_artifact_as_a_bare_array() {
    let total = ARTIFACT_DRAIN_PAGE_ROWS as u128 + 10;
    let rows: Vec<ArtifactView> = (0..total).map(|n| artifact(n, n as i64)).collect();
    let (_, base) = stand_in(rows).await;
    let svc = relaying_to(&base);

    let answer = only_block(
        &list_artifacts(
            &svc,
            &empty_parts(),
            input(json!({ "resource_id": RESOURCE.to_string() })),
        )
        .await
        .expect("the list answers"),
    );
    let every = answer.as_array().expect("a bare array, not an envelope");
    assert_eq!(every.len() as u128, total, "every artifact, not one page");
}

/// With `limit`, one block holding the envelope: `next` names the position after the page's last
/// artifact, and on the last page `next` is present and `null`. FAILS IF the page is answered as
/// a bare array, if `next` is dropped, or if the last page omits `next` instead of nulling it.
#[tokio::test]
async fn with_limit_one_block_holds_the_envelope_and_next_is_null_on_the_last_page() {
    let (_, base) = stand_in(vec![artifact(0, 100), artifact(1, 200), artifact(2, 300)]).await;
    let svc = relaying_to(&base);

    let first = only_block(
        &list_artifacts(
            &svc,
            &empty_parts(),
            input(json!({ "resource_id": RESOURCE.to_string(), "limit": 2 })),
        )
        .await
        .expect("the first page answers"),
    );
    let keys: Vec<&str> = first
        .as_object()
        .expect("an envelope object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["artifacts", "next"]);
    assert_eq!(first["artifacts"].as_array().expect("artifacts").len(), 2);
    let second_row = artifact(1, 200);
    assert_eq!(
        first["next"],
        json!({
            "created": second_row.created,
            "artifact_id": second_row.artifact_id,
        }),
        "next is the last artifact's position"
    );

    let last = only_block(
        &list_artifacts(
            &svc,
            &empty_parts(),
            input(json!({
                "resource_id": RESOURCE.to_string(),
                "limit": 2,
                "after": first["next"],
            })),
        )
        .await
        .expect("the last page answers"),
    );
    assert_eq!(
        ids(&last["artifacts"]),
        [artifact(2, 300).artifact_id.uuid().to_string()]
    );
    assert!(
        last.as_object()
            .expect("an envelope object")
            .contains_key("next"),
        "next is present on the last page: {last}"
    );
    assert!(last["next"].is_null(), "and null: {last}");
}

/// The door forwards `limit`, `order`, the whole `after` position and the filters exactly.
/// FAILS IF any of them is dropped, defaulted or loses precision on the way to the route.
#[tokio::test]
async fn limit_order_after_and_the_filters_reach_the_route_exactly() {
    let (stand_in, base) = stand_in(Vec::new()).await;
    let svc = relaying_to(&base);
    let after = artifact(9, 123_456);

    list_artifacts(
        &svc,
        &empty_parts(),
        input(json!({
            "resource_id": RESOURCE.to_string(),
            "kind": "measurement",
            "intent": "member",
            "include_folded": true,
            "limit": 7,
            "order": "desc",
            "after": { "created": after.created, "artifact_id": after.artifact_id },
        })),
    )
    .await
    .expect("the page answers");

    let asked = stand_in.asked.lock().expect("lock").clone();
    assert_eq!(asked.len(), 1, "one page is one request");
    let q = &asked[0];
    assert_eq!(q.limit, Some(7));
    assert_eq!(q.order, Some(SortOrder::Desc));
    assert_eq!(q.after_created, Some(after.created));
    assert_eq!(q.after_id, Some(after.artifact_id.uuid()));
    assert_eq!(q.kind.as_deref(), Some("measurement"));
    assert_eq!(q.intent.as_deref(), Some("member"));
    assert_eq!(q.include_folded, Some(true));
    assert_eq!(q.counts, None);
}

/// The paging refusals are the route's own words, as `invalid_params`, spoken before anything is
/// relayed: the service here cannot relay at all, so a call that reached the relay would answer
/// the host's sentence as an internal error. FAILS IF a combination is ignored, forwarded, or
/// refused in other words.
#[tokio::test]
async fn the_paging_refusals_are_the_routes_own_words_before_any_relay() {
    let svc = TemperMcpService::unavailable(
        BlobDoor::Closed {
            refusal: "unused".to_string(),
        },
        "this host relays nowhere",
    );
    let position = json!({
        "created": "2026-10-19T12:30:45.123456Z",
        "artifact_id": Uuid::from_u128(9).to_string(),
    });
    let over = MAX_ARTIFACT_PAGE_ROWS + 1;
    let cases = [
        (json!({ "limit": 0 }), ArtifactPageRefusal::LimitZero),
        (
            json!({ "limit": over }),
            ArtifactPageRefusal::LimitAboveCeiling { limit: over },
        ),
        (
            json!({ "after": position }),
            ArtifactPageRefusal::AfterWithoutLimit,
        ),
        (
            json!({ "order": "desc" }),
            ArtifactPageRefusal::OrderWithoutLimit,
        ),
    ];
    for (mut args, refusal) in cases {
        args["resource_id"] = json!(RESOURCE.to_string());
        let err = list_artifacts(&svc, &empty_parts(), input(args.clone()))
            .await
            .expect_err("refused");
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS, "{args}");
        assert_eq!(err.message.as_ref(), refusal.to_string(), "{args}");
    }
}

/// Following `next` visits every artifact exactly once, oldest first, over three pages, with a
/// page boundary between two artifacts that share a `created`. FAILS IF the continuation loses
/// its `artifact_id` tie-break (the tie partner is skipped) or its sub-second digits (pages
/// repeat).
#[tokio::test]
async fn walking_next_ascending_visits_every_artifact_once() {
    let (_, base) = stand_in(six_with_a_tie()).await;
    let svc = relaying_to(&base);
    let (visited, pages) = walk(&svc, None).await;
    assert_eq!(visited, expected_ids(SortOrder::Asc));
    assert_eq!(pages, 3);
}

/// The same walk newest first, keeping `order: desc` on every page. FAILS IF the direction is not
/// carried on the continuation, or the tie is mishandled at the descending boundary.
#[tokio::test]
async fn walking_next_descending_visits_every_artifact_once() {
    let (_, base) = stand_in(six_with_a_tie()).await;
    let svc = relaying_to(&base);
    let (visited, pages) = walk(&svc, Some("desc")).await;
    assert_eq!(visited, expected_ids(SortOrder::Desc));
    assert_eq!(pages, 3);
}
