//! `HttpClient::send` retry-with-backoff behavior.
//!
//! These tests use `wiremock` to serve mock responses so the retry loop can be
//! exercised deterministically without real network traffic. They verify the
//! cold-start resilience contract: safe (idempotent) requests retry transient
//! 5xx failures; writes never do; and retries are bounded.

use reqwest::Method;
use temper_client::error::ClientError;
use temper_client::http::HttpClient;
use temper_workflow::operations::Surface;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_retries_on_5xx_then_succeeds() {
    let server = MockServer::start().await;

    // First two GETs: 500. Third: 200. up_to_n_times caps the 500 mock so the
    // 200 mock answers the third attempt.
    Mock::given(method("GET"))
        .and(path("/api/resources"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::CliCloud, None);
    let req = client.get("/api/resources");
    let resp = client
        .send(&Method::GET, "/api/resources", req, None)
        .await
        .expect("GET should succeed after retrying the cold-start 500s");
    assert!(resp.status().is_success());
    // wiremock verifies .expect(n) on drop: exactly 3 requests were made.
}

#[tokio::test]
async fn get_exhausts_retries_and_returns_server_error() {
    let server = MockServer::start().await;

    // Persistent 500: every attempt fails. expect(3) asserts the loop is
    // bounded at MAX_ATTEMPTS and does not retry forever.
    Mock::given(method("GET"))
        .and(path("/api/resources"))
        .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
        .expect(3)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::CliCloud, None);
    let req = client.get("/api/resources");
    let err = client
        .send(&Method::GET, "/api/resources", req, None)
        .await
        .expect_err("persistent 5xx should propagate after exhausting retries");
    assert!(matches!(err, ClientError::Server { status: 503, .. }));
}

#[tokio::test]
async fn post_does_not_retry_on_5xx() {
    let server = MockServer::start().await;

    // A write that 500s must be sent exactly once — retrying could duplicate a
    // server-side effect.
    Mock::given(method("POST"))
        .and(path("/api/ingest"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::CliCloud, None);
    let req = client.post("/api/ingest");
    let err = client
        .send(&Method::POST, "/api/ingest", req, None)
        .await
        .expect_err("POST 500 should propagate without retrying");
    assert!(matches!(err, ClientError::Server { status: 500, .. }));
}

#[tokio::test]
async fn sends_surface_header_on_every_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .and(header("X-Temper-Surface", "cli"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::CliCloud, None);
    let req = client.get("/api/health");
    // The `header` matcher plus `.expect(1)` asserts `X-Temper-Surface: cli` was sent;
    // the mock verifies on drop.
    let _ = client.send(&Method::GET, "/api/health", req, None).await;
}

#[tokio::test]
async fn sdk_client_sends_sdk_marker() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/health"))
        .and(header("X-Temper-Surface", "sdk"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpClient::new(&server.uri(), None, Surface::Sdk, None);
    let req = client.get("/api/health");
    let _ = client.send(&Method::GET, "/api/health", req, None).await;
}
