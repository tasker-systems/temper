//! ClaudeProvider unit tests

use temper_llm::providers::claude::ClaudeProvider;
use temper_llm::LlmProvider;

// ── Test: new() rejects empty API key ─────────────────────────────────────────

#[test]
fn new_rejects_empty_api_key() {
    let result = ClaudeProvider::new("claude-sonnet-4-20250514", String::new());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("empty"),
        "expected 'empty' in error, got: {}",
        err
    );
}

// ── Test: new() accepts valid API key ──────────────────────────────────────────

#[test]
fn new_accepts_valid_api_key() {
    let result = ClaudeProvider::new("claude-sonnet-4-20250514", "test-key".to_string());
    assert!(result.is_ok());
    let provider = result.unwrap();
    assert_eq!(provider.model(), "claude-sonnet-4-20250514");
    assert_eq!(provider.provider_name(), "anthropic");
}

// ── Test: model() returns configured model ─────────────────────────────────────

#[test]
fn model_returns_configured() {
    let provider = ClaudeProvider::new("claude-opus-4-5", "sk-test".to_string()).unwrap();
    assert_eq!(provider.model(), "claude-opus-4-5");
}

// ── Test: provider_name() returns "anthropic" ──────────────────────────────────

#[test]
fn provider_name_is_anthropic() {
    let provider = ClaudeProvider::new("claude-sonnet-4", "sk-test".to_string()).unwrap();
    assert_eq!(provider.provider_name(), "anthropic");
}

// ── Test: Debug impl does not panic ───────────────────────────────────────────

#[test]
fn debug_impl_does_not_panic() {
    let provider = ClaudeProvider::new("claude-sonnet-4", "sk-test".to_string()).unwrap();
    let debug = format!("{:?}", provider);
    assert!(debug.contains("ClaudeProvider"));
    assert!(debug.contains("claude-sonnet-4"));
}

// ── Test: Send + Sync (required by trait object) ──────────────────────────────

#[test]
fn provider_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ClaudeProvider>();
}
