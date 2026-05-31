//! ClaudeProvider retry-with-backoff behavior.
//!
//! The Anthropic Messages API lives at `https://api.anthropic.com/v1/messages`,
//! so we use a wiremock server and inject its URI via a test-only env var
//! read by `ClaudeProvider`. The real constructor uses the hardcoded base URL;
//! tests use the same constructor but the retry path exercised by these tests
//! is URL-agnostic — only HTTP status handling matters here.

use serde_json::json;
use temper_llm::providers::claude::ClaudeProvider;
use temper_llm::{LlmError, LlmProvider, LlmResponse};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn claude_success_body() -> serde_json::Value {
    json!({
        "id": "msg_test",
        "type": "message",
        "role": "assistant",
        "content": [
            { "type": "text", "text": "{\"answer\": \"ok\"}" }
        ],
        "model": "claude-sonnet-4",
        "stop_reason": "end_turn"
    })
}

/// Build a `ClaudeProvider` whose internal HTTP calls are redirected to the
/// given mock-server URI. Tests use the `TEMPER_LLM_CLAUDE_BASE_URL_OVERRIDE`
/// test hook in the provider.
fn build_provider_with_override(uri: &str) -> ClaudeProvider {
    // The override is read once inside `ClaudeProvider::new` and stored on the
    // provider, so scoping the env var to just the constructor call is enough —
    // `temp_env::with_var` restores the prior value (or unsets it) on return.
    temp_env::with_var("TEMPER_LLM_CLAUDE_BASE_URL_OVERRIDE", Some(uri), || {
        ClaudeProvider::new("claude-sonnet-4", "test-key".to_string(), 30).unwrap()
    })
}

#[tokio::test]
async fn test_claude_retries_on_5xx_then_succeeds() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(claude_success_body()))
        .expect(1)
        .mount(&server)
        .await;

    let provider = build_provider_with_override(&server.uri());

    let result = provider.complete("sys", &[], &[], None).await;

    match result {
        Ok(LlmResponse::Final { .. }) => {}
        other => panic!("expected Ok(Final), got {other:?}"),
    }
}

#[tokio::test]
async fn test_claude_no_retry_on_4xx() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad"))
        .expect(1)
        .mount(&server)
        .await;

    let provider = build_provider_with_override(&server.uri());

    let result = provider.complete("sys", &[], &[], None).await;

    match result {
        Err(LlmError::Provider(_)) => {}
        other => panic!("expected Err(Provider), got {other:?}"),
    }
}

#[tokio::test]
async fn test_claude_exhausts_retries() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .expect(3)
        .mount(&server)
        .await;

    let provider = build_provider_with_override(&server.uri());

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
