//! OpenAiCompatibleProvider retry-with-backoff behavior.
//!
//! These tests use `wiremock` to serve mock chat-completions responses so the
//! retry loop can be exercised deterministically without real network traffic.

use serde_json::json;
use temper_llm::{LlmError, LlmProvider, LlmResponse, OpenAiCompatibleProvider};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn chat_success_body() -> serde_json::Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "{\"answer\": \"ok\"}"
            },
            "finish_reason": "stop"
        }]
    })
}

#[tokio::test]
async fn test_retries_on_5xx_then_succeeds() {
    let server = MockServer::start().await;

    // First two requests: 500. Third: 200 success.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_success_body()))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(&server.uri(), "test-model", None, 30).unwrap();

    let result = provider.complete("sys", &[], &[], None).await;

    match result {
        Ok(LlmResponse::Final { .. }) => {}
        other => panic!("expected Ok(Final), got {other:?}"),
    }
    // wiremock verifies .expect(n) on drop via the MockServer.
}

#[tokio::test]
async fn test_no_retry_on_4xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("Bad Request"))
        .expect(1) // exactly one hit — no retries
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(&server.uri(), "test-model", None, 30).unwrap();

    let result = provider.complete("sys", &[], &[], None).await;

    match result {
        Err(LlmError::Provider(_)) => {}
        other => panic!("expected Err(Provider), got {other:?}"),
    }
}

#[tokio::test]
async fn test_exhausts_retries_then_returns_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .expect(3)
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(&server.uri(), "test-model", None, 30).unwrap();

    let result = provider.complete("sys", &[], &[], None).await;

    match result {
        Err(LlmError::Provider(msg)) => {
            assert!(
                msg.contains("500"),
                "expected error message to mention HTTP 500, got: {msg}"
            );
        }
        other => panic!("expected Err(Provider) mentioning 500, got {other:?}"),
    }
}
