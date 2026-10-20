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
