//! `InvocationsClient::list` pinned to the wire: the filters ride the query string as
//! `InvocationListQuery` serializes them, encoded, and an absent filter is omitted. Same
//! `wiremock` pattern as `segments_client_test.rs`.

use std::sync::Arc;

use uuid::Uuid;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use temper_client::auth::MemoryTokenStore;
use temper_client::TemperClient;

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

#[tokio::test]
async fn list_sends_both_filters_on_the_query() {
    let server = MockServer::start().await;
    let cogmap = Uuid::now_v7();
    Mock::given(method("GET"))
        .and(path("/api/invocations"))
        .and(query_param("cogmap", cogmap.to_string()))
        .and(query_param("status", "open"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;

    let rows = test_client(&server.uri())
        .invocations()
        .list(Some(cogmap), Some("open".to_string()))
        .await
        .expect("list");
    assert!(rows.is_empty());
}

/// FAILS IF an absent filter is sent (as `cogmap=` or `status=null`) or a status value can add a
/// parameter of its own: the query string is serialized, not concatenated.
#[tokio::test]
async fn list_omits_absent_filters_and_encodes_the_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/invocations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let client = test_client(&server.uri());
    client.invocations().list(None, None).await.expect("list");
    client
        .invocations()
        .list(None, Some("open&cogmap=x".to_string()))
        .await
        .expect("list");

    let requests = server.received_requests().await.expect("recording is on");
    let queries: Vec<_> = requests
        .iter()
        .map(|r| r.url.query().map(str::to_owned))
        .collect();
    assert_eq!(
        queries,
        vec![None, Some("status=open%26cogmap%3Dx".to_owned())]
    );
}
