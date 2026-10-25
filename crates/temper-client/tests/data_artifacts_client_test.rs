//! `DataArtifactsClient::list_page` / `list_all` and the `Link: rel="next"` continuation.
//!
//! Uses `wiremock`, the pattern `reblock_client_test.rs` establishes in this crate. The body is
//! always the bare array; the continuation rides the `Link` header. A server without the bounded
//! list ignores `limit` and answers every artifact with no `Link`, which the client reads as the
//! final page. A `Link` it cannot read is an error, never a silent last page.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::data_artifacts::parse_next_link;
use temper_client::error::ClientError;
use temper_client::TemperClient;
use temper_core::types::data_artifact::{
    ArtifactListParams, ArtifactPageRequest, ArtifactView, ARTIFACT_DRAIN_PAGE_ROWS,
};
use temper_core::types::ids::{DataArtifactId, ResourceId};
use temper_core::types::resource::SortOrder;

fn test_client(base_url: &str) -> TemperClient {
    TemperClient::with_token(
        base_url,
        None,
        temper_workflow::operations::Surface::CliCloud,
        "test-token".to_string(),
        Arc::new(MemoryTokenStore::empty()),
    )
    .expect("test server URL (loopback) validates")
}

fn artifact(resource: Uuid, n: i64) -> ArtifactView {
    ArtifactView {
        artifact_id: DataArtifactId::new(),
        resource_id: ResourceId::from(resource),
        kind_owner_table: "kb_profiles".to_string(),
        kind_owner_id: Uuid::now_v7(),
        artifact_kind: "measurement".to_string(),
        intent: "member".to_string(),
        precedence: 0.0,
        content_hash: format!("hash-{n}"),
        content_bytes: 1,
        shape_state: "never_declared".to_string(),
        is_folded: false,
        created: chrono::Utc::now(),
        content: Some(serde_json::json!({ "n": n })),
    }
}

/// A new client against an old server: the old route ignores `limit` and answers every artifact
/// as a bare array with no `Link`. `list_all` returns all of them and asks exactly once, because
/// no `Link` means the final page: neither truncated nor followed.
#[tokio::test]
async fn list_all_reads_an_older_servers_answer_with_no_link_as_the_final_page() {
    let resource = Uuid::now_v7();
    // More than one drain page: an old server returns everything regardless of `limit`.
    let everything: Vec<ArtifactView> = (0..i64::from(ARTIFACT_DRAIN_PAGE_ROWS) + 3)
        .map(|n| artifact(resource, n))
        .collect();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/resources/{resource}/artifacts")))
        .and(query_param("limit", ARTIFACT_DRAIN_PAGE_ROWS.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(&everything))
        .expect(1)
        .mount(&server)
        .await;

    let drained = test_client(&server.uri())
        .data_artifacts()
        .list_all(resource, &ArtifactListParams::default())
        .await
        .expect("a bare array is a readable answer");

    assert_eq!(
        drained.iter().map(|a| a.artifact_id).collect::<Vec<_>>(),
        everything.iter().map(|a| a.artifact_id).collect::<Vec<_>>(),
        "every element, in order, none dropped"
    );
    // wiremock verifies `.expect(1)` on drop: exactly one request was made.
}

const AFTER_ID: &str = "00000000-0000-0000-0000-0000000000aa";
const AFTER_CREATED: &str = "2026-10-19T12:30:45.123456Z";

fn link_to_next(resource: Uuid) -> String {
    format!(
        "</api/resources/{resource}/artifacts?limit={ARTIFACT_DRAIN_PAGE_ROWS}\
         &after_created=2026-10-19T12%3A30%3A45.123456Z&after_id={AFTER_ID}>; rel=\"next\""
    )
}

/// `list_all` follows `Link: rel="next"`: the first page names a position, the second request
/// carries exactly that position, and the drain ends on the page with no `Link`.
#[tokio::test]
async fn list_all_follows_the_link_header_to_the_last_page() {
    let resource = Uuid::now_v7();
    let first = vec![artifact(resource, 0), artifact(resource, 1)];
    let second = vec![artifact(resource, 2)];

    let server = MockServer::start().await;
    let route = format!("/api/resources/{resource}/artifacts");
    // The more specific mock is mounted first: wiremock answers with the first that matches.
    Mock::given(method("GET"))
        .and(path(&route))
        .and(query_param("after_id", AFTER_ID))
        .and(query_param("after_created", AFTER_CREATED))
        .respond_with(ResponseTemplate::new(200).set_body_json(&second))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(&route))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Link", link_to_next(resource).as_str())
                .set_body_json(&first),
        )
        .expect(1)
        .mount(&server)
        .await;

    let drained = test_client(&server.uri())
        .data_artifacts()
        .list_all(resource, &ArtifactListParams::default())
        .await
        .expect("the drain follows the link");

    assert_eq!(
        drained.iter().map(|a| a.artifact_id).collect::<Vec<_>>(),
        first
            .iter()
            .chain(&second)
            .map(|a| a.artifact_id)
            .collect::<Vec<_>>(),
    );
}

/// A `Link` that is present but unreadable is an error: reading it as "no next page" would end a
/// drain early and hand back a short list as if it were whole.
#[tokio::test]
async fn a_malformed_link_is_an_error_not_a_last_page() {
    let resource = Uuid::now_v7();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/resources/{resource}/artifacts")))
        .respond_with(
            ResponseTemplate::new(200)
                // rel="next" with a target that carries no `after_id`.
                .insert_header(
                    "Link",
                    "</api/resources/x/artifacts?after_created=2026-10-19T12%3A30%3A45Z>; rel=\"next\"",
                )
                .set_body_json(vec![artifact(resource, 0)]),
        )
        .expect(1)
        .mount(&server)
        .await;

    let err = test_client(&server.uri())
        .data_artifacts()
        .list_page(
            resource,
            &ArtifactListParams::default(),
            ArtifactPageRequest {
                limit: 2,
                after: None,
                order: SortOrder::Asc,
            },
        )
        .await
        .expect_err("an unreadable Link must not read as the last page");

    assert!(
        matches!(&err, ClientError::Other(m) if m.contains("malformed `Link`")),
        "got: {err}"
    );
}

#[test]
fn parse_next_link_reads_the_servers_form_and_refuses_anything_else() {
    let resource = Uuid::now_v7();
    let cursor = parse_next_link(&link_to_next(resource))
        .expect("the server's form parses")
        .expect("it names a next page");
    assert_eq!(
        cursor.created,
        AFTER_CREATED
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
    assert_eq!(cursor.artifact_id.uuid().to_string(), AFTER_ID);

    // A well-formed link that is not the next page names no next page.
    assert_eq!(
        parse_next_link("</api/resources/x/artifacts>; rel=\"prev\"").unwrap(),
        None
    );
    // Anything that is not `<target>`-shaped is malformed, and so is a next link that lost its
    // position.
    for bad in [
        "",
        "not a link",
        "<unterminated; rel=\"next\"",
        "</a?after_id=not-a-uuid&after_created=2026-10-19T12%3A30%3A45Z>; rel=\"next\"",
        "</a?after_id=00000000-0000-0000-0000-0000000000aa&after_created=yesterday>; rel=\"next\"",
        "</a>; rel=\"next\"",
    ] {
        assert!(
            parse_next_link(bad).is_err(),
            "should be malformed: {bad:?}"
        );
    }
}

/// RFC 8288 spellings of the same relation: the `rel` name and the relation type compare
/// case-insensitively, the value may be bare or quoted, and it may list several relation types.
/// Each must be read as the next page, or a drain against a server that spells it differently
/// stops after one page and returns a short list as if it were whole.
#[test]
fn parse_next_link_reads_rel_as_rfc_8288_allows() {
    let target = format!("</a?after_created=2026-10-19T12%3A30%3A45.123456Z&after_id={AFTER_ID}>");
    for params in [
        "; rel=next",
        "; REL=\"next\"",
        "; Rel=NEXT",
        "; rel=\"next last\"",
        "; rel=\"prev  next\"",
        "; title=\"page\"; rel=\"next\"",
        " ;rel = \"next\" ",
    ] {
        let cursor = parse_next_link(&format!("{target}{params}"))
            .unwrap_or_else(|e| panic!("{params:?} should parse: {e}"))
            .unwrap_or_else(|| panic!("{params:?} names the next page"));
        assert_eq!(cursor.artifact_id.uuid().to_string(), AFTER_ID);
    }
    // Not the next page: another relation, a relation that only starts with `next`, no `rel`,
    // and a second `rel` (only the first counts).
    for params in [
        "; rel=last",
        "; rel=\"nextish\"",
        "; title=\"next\"",
        "; rel=\"prev\"; rel=\"next\"",
    ] {
        assert_eq!(
            parse_next_link(&format!("{target}{params}")).unwrap(),
            None,
            "{params:?} names no next page"
        );
    }
}

/// A server whose `Link` repeats the position the client just sent would lap forever. The drain
/// refuses on the first repeat: two requests, then an error naming the stall.
#[tokio::test]
async fn list_all_refuses_a_link_that_does_not_advance() {
    let resource = Uuid::now_v7();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/resources/{resource}/artifacts")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Link", link_to_next(resource).as_str())
                .set_body_json(vec![artifact(resource, 0)]),
        )
        .expect(2)
        .mount(&server)
        .await;

    let err = test_client(&server.uri())
        .data_artifacts()
        .list_all(resource, &ArtifactListParams::default())
        .await
        .expect_err("a Link that names the position just asked for must not be followed again");

    assert!(
        matches!(&err, ClientError::Other(m) if m.contains("did not advance")),
        "got: {err}"
    );
    // wiremock verifies `.expect(2)` on drop: the first page, then the one repeat.
}

const POS_EARLY: (&str, &str) = (
    "2026-10-19T12%3A30%3A45.123456Z",
    "00000000-0000-0000-0000-0000000000aa",
);
const POS_LATE: (&str, &str) = (
    "2026-10-19T12%3A31%3A45.123456Z",
    "00000000-0000-0000-0000-0000000000bb",
);
/// Same `created` as `POS_LATE`, a lower id: behind it only on the tiebreak.
const POS_LATE_LOWER_ID: (&str, &str) = (
    "2026-10-19T12%3A31%3A45.123456Z",
    "00000000-0000-0000-0000-0000000000ab",
);

fn link_at(resource: Uuid, (created, id): (&str, &str)) -> String {
    format!(
        "</api/resources/{resource}/artifacts?limit={ARTIFACT_DRAIN_PAGE_ROWS}\
         &after_created={created}&after_id={id}>; rel=\"next\""
    )
}

/// Mount `route` so that a request asking to continue after `asked_after` (`None`: the first
/// page) is answered once with a `Link` to `names`. Each mount answers exactly once and then
/// stops matching, so a drain that goes on past the walk the test scripts gets a 404 and fails
/// on that error instead of looping.
async fn mount_hop(
    server: &MockServer,
    resource: Uuid,
    asked_after: Option<(&str, &str)>,
    names: (&str, &str),
) {
    let mock = Mock::given(method("GET")).and(path(format!("/api/resources/{resource}/artifacts")));
    let mock = match asked_after {
        Some((_, id)) => mock.and(query_param("after_id", id)),
        None => mock,
    };
    mock.respond_with(
        ResponseTemplate::new(200)
            .insert_header("Link", link_at(resource, names).as_str())
            .set_body_json(vec![artifact(resource, 0)]),
    )
    .up_to_n_times(1)
    .expect(1)
    .mount(server)
    .await;
}

async fn drain_error(server: &MockServer, resource: Uuid) -> ClientError {
    test_client(&server.uri())
        .data_artifacts()
        .list_all(resource, &ArtifactListParams::default())
        .await
        .expect_err("a Link that does not strictly advance must end the drain with an error")
}

fn assert_did_not_advance(err: &ClientError) {
    assert!(
        matches!(err, ClientError::Other(m) if m.contains("did not advance")),
        "got: {err}"
    );
}

/// A server whose `Link` moves backward: the first page names LATE, the continuation after LATE
/// names EARLY. The drain refuses on that second answer, after exactly two requests. Bites the
/// guard reverting to exact-repeat only: EARLY is not equal to LATE, so it would be followed, and
/// the scripted walk has no mount for a third request, which then answers 404 and fails the
/// message assertion (and `.expect(1)` on each hop).
#[tokio::test]
async fn list_all_refuses_a_link_that_moves_backward() {
    let resource = Uuid::now_v7();
    let server = MockServer::start().await;
    // The position-specific mount goes first: wiremock answers with the first that matches.
    mount_hop(&server, resource, Some(POS_LATE), POS_EARLY).await;
    mount_hop(&server, resource, None, POS_LATE).await;

    assert_did_not_advance(&drain_error(&server, resource).await);
    // wiremock verifies `.expect(1)` on each hop on drop: two requests, no third.
}

/// A server whose `Link` cycles A -> B -> A: every hop differs from the position just asked for,
/// so an exact-repeat guard never fires and the drain would append pages forever. The drain
/// refuses when B's continuation names A, after exactly three requests. Bites the guard
/// reverting to exact-repeat only: the fourth request, after A, has no mount left (each hop
/// answers once), so it answers 404 and the test fails on that instead of hanging.
#[tokio::test]
async fn list_all_refuses_a_link_that_cycles_back_to_an_earlier_position() {
    let resource = Uuid::now_v7();
    let server = MockServer::start().await;
    mount_hop(&server, resource, Some(POS_LATE), POS_EARLY).await;
    mount_hop(&server, resource, Some(POS_EARLY), POS_LATE).await;
    mount_hop(&server, resource, None, POS_EARLY).await;

    assert_did_not_advance(&drain_error(&server, resource).await);
    // wiremock verifies `.expect(1)` on each hop on drop: three requests, no fourth.
}

/// The position moves backward only on the tiebreak: the same `created`, a lower id. Bites a
/// guard that compares `created` alone (or `>=` on it): the continuation is not beyond LATE in
/// `(created, artifact_id)` order, so the drain refuses after two requests.
#[tokio::test]
async fn list_all_refuses_a_link_that_moves_backward_on_the_id_tiebreak() {
    let resource = Uuid::now_v7();
    let server = MockServer::start().await;
    mount_hop(&server, resource, Some(POS_LATE), POS_LATE_LOWER_ID).await;
    mount_hop(&server, resource, None, POS_LATE).await;

    assert_did_not_advance(&drain_error(&server, resource).await);
}

/// `list_page` sends the direction it was asked for, so a newest-first page asks for
/// `order=desc` and is not silently read oldest first.
#[tokio::test]
async fn list_page_sends_the_order_it_was_asked_for() {
    let resource = Uuid::now_v7();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/resources/{resource}/artifacts")))
        .and(query_param("order", "desc"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(vec![artifact(resource, 0)]))
        .expect(1)
        .mount(&server)
        .await;

    let page = test_client(&server.uri())
        .data_artifacts()
        .list_page(
            resource,
            &ArtifactListParams::default(),
            ArtifactPageRequest {
                limit: 2,
                after: None,
                order: SortOrder::Desc,
            },
        )
        .await
        .expect("a desc request matches the stub");
    assert_eq!(page.artifacts.len(), 1);
}

/// One batch-read page with one artifact, the shape and a continuation: what the stand-in answers.
fn ask_page(resource: Uuid) -> serde_json::Value {
    serde_json::json!({
        "shape": {
            "shape_id": Uuid::now_v7(),
            "shape_version": 1,
            "home_anchor_table": "kb_contexts",
            "home_anchor_id": Uuid::now_v7()
        },
        "artifacts": [serde_json::to_value(artifact(resource, 1)).expect("serializes")],
        "oversized": [],
        "population": {
            "declared_satisfied": 1,
            "declared_not_satisfied": 0,
            "declared_not_yet_checked": 0,
            "matched": 1
        },
        "ordered_by": "created",
        "next": { "created": "2026-10-24T09:15:30.123456Z", "artifact_id": Uuid::now_v7() },
        "amended_since": null
    })
}

/// **`ask` POSTs the body it is handed, unread, and reads the page back as an `AskResponse`.** The
/// body carries a field no `AskRequest` declares, which a client that parsed it first could not
/// send; the stand-in matches the body exactly and answers one page, which round-trips. Bites: the
/// method deserializing its input as `AskRequest` (the extra field is dropped and the body matcher
/// misses), or a field of `AskResponse` the page does not round-trip.
#[tokio::test]
async fn ask_sends_the_body_unread_and_reads_the_page() {
    use wiremock::matchers::body_json;

    let server = MockServer::start().await;
    let body = serde_json::json!({
        "ask": { "shape": Uuid::now_v7(), "predicates": [
            { "path": ["unit"], "op": { "op": "contains", "values": ["ms"] } }
        ] },
        "limit": 3,
        "not_a_field": true
    });
    let page = ask_page(Uuid::now_v7());
    Mock::given(method("POST"))
        .and(path("/api/data-artifacts/ask"))
        .and(body_json(&body))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page))
        .expect(1)
        .mount(&server)
        .await;

    let answer = test_client(&server.uri())
        .data_artifacts()
        .ask(&body)
        .await
        .expect("the page reads");
    assert_eq!(serde_json::to_value(&answer).expect("serializes"), page);
}

/// **A refused ask arrives as every refusal at once, `teaches` intact**, through the `PLAN_REFUSED`
/// arm the composition door already uses. Bite: the 400 read as a bare `Server` error.
#[tokio::test]
async fn a_refused_ask_arrives_as_every_refusal_with_its_teaching() {
    let server = MockServer::start().await;
    let refusals = serde_json::json!([
        { "reason": "value_outside_shape", "detail": "a value lies outside the shape",
          "teaches": { "non_closure": "stored content is not closed by its shape" } },
        { "reason": "page_limit_out_of_range", "detail": "`limit` must be at least 1" }
    ]);
    Mock::given(method("POST"))
        .and(path("/api/data-artifacts/ask"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "code": temper_core::error::PLAN_REFUSED_CODE,
                "message": "Plan refused: 2 refusal(s)",
                "details": { "refusals": refusals }
            }
        })))
        .mount(&server)
        .await;

    let err = test_client(&server.uri())
        .data_artifacts()
        .ask(&serde_json::json!({ "ask": { "shape": Uuid::now_v7() }, "limit": 0 }))
        .await
        .expect_err("refused");
    match err {
        ClientError::PlanRefused { refusals: got } => {
            assert_eq!(serde_json::to_value(&got).expect("serializes"), refusals);
        }
        other => panic!("expected PlanRefused, got {other:?}"),
    }
}
