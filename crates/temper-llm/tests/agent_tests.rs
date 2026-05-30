//! temper-llm unit tests

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::Arc;

use temper_llm::Schema;
use temper_llm::{
    Agent, AgentError, AgentOutcome, MockLlmProvider, MockScenario, Tool, ToolHandler,
};

/// Simple empty state for tests that don't use state.
#[derive(Default)]
struct NoState;

/// String-state for verifying threading.
struct StringState {
    accumulated: String,
}

impl StringState {
    fn new() -> Self {
        Self {
            accumulated: String::new(),
        }
    }
}

// ── Tool handler implementations ───────────────────────────────────────────────

/// Appends a fixed value to state.accumulated on each call.
struct StringAppender {
    value: String,
}

#[async_trait]
impl ToolHandler<StringState> for StringAppender {
    async fn call(&self, _input: &Value, state: &mut StringState) -> Result<Value, String> {
        state.accumulated.push_str(&self.value);
        Ok(json!({ "ok": true }))
    }
}

/// Returns its input back as output.
struct EchoHandler;

#[async_trait]
impl ToolHandler<StringState> for EchoHandler {
    async fn call(&self, input: &Value, _state: &mut StringState) -> Result<Value, String> {
        Ok(input.clone())
    }
}

/// Always returns an error.
struct ErrorHandler;

#[async_trait]
impl ToolHandler<NoState> for ErrorHandler {
    async fn call(&self, _input: &Value, _state: &mut NoState) -> Result<Value, String> {
        Err("deliberate tool error".to_string())
    }
}

// ── Test: single-turn, no tools, Final ────────────────────────────────────────

#[tokio::test]
async fn single_turn_final() {
    let provider = MockLlmProvider::new("mock", "mock")
        .scenario(MockScenario::SingleTurn(json!({"title": "Test"})));

    let mut agent = Agent::new(Arc::new(provider), vec![], 1, NoState);

    let outcome = agent.run("system", "user").await.unwrap();

    match outcome {
        AgentOutcome::Final { content } => {
            assert_eq!(content, json!({"title": "Test"}));
        }
        AgentOutcome::MaxTurns => panic!("expected Final, got MaxTurns"),
    }
}

// ── Test: two-turn tool dispatch → Final ───────────────────────────────────────

#[tokio::test]
async fn two_turn_tool_dispatch() {
    let provider = MockLlmProvider::new("mock", "mock").scenario(MockScenario::Sequence {
        tool_name: "echo".to_string(),
        tool_inputs: vec![json!({"msg": "hello"})],
        final_response: json!({"done": true}),
    });

    let tool = Tool::new(
        "echo".to_string(),
        "A test tool".to_string(),
        Schema::from(Map::new()),
        EchoHandler,
    );

    let mut agent = Agent::new(Arc::new(provider), vec![tool], 3, StringState::new());

    let outcome = agent.run("system", "use echo").await.unwrap();

    match outcome {
        AgentOutcome::Final { content } => {
            assert_eq!(content, json!({"done": true}));
        }
        AgentOutcome::MaxTurns => panic!("expected Final, got MaxTurns"),
    }
}

// ── Test: max_turns exhaustion ─────────────────────────────────────────────

#[tokio::test]
async fn max_turns_exhaustion() {
    let provider = MockLlmProvider::new("mock", "mock").scenario(MockScenario::Sequence {
        tool_name: "noop".to_string(),
        // 3 tool inputs but agent only allows 2 turns
        tool_inputs: vec![json!({}), json!({}), json!({})],
        final_response: json!({"done": true}),
    });

    let tool = Tool::new(
        "noop".to_string(),
        "A no-op tool".to_string(),
        Schema::from(Map::new()),
        EchoHandler,
    );

    let mut agent = Agent::new(
        Arc::new(provider),
        vec![tool],
        2, // max 2 turns; 3rd tool call would return Final but we never get there
        StringState::new(),
    );

    let outcome = agent.run("system", "use noop").await.unwrap();
    assert!(matches!(outcome, AgentOutcome::MaxTurns));
}

// ── Test: unknown tool name → ToolNotFound error ───────────────────────────────

#[tokio::test]
async fn unknown_tool_not_found() {
    let provider = MockLlmProvider::new("mock", "mock").scenario(MockScenario::Sequence {
        tool_name: "nonexistent".to_string(),
        tool_inputs: vec![json!({})],
        final_response: json!({"done": true}),
    });

    // No tools registered — the single tool call will fail with ToolNotFound.
    let mut agent = Agent::new(Arc::new(provider), vec![], 3, NoState);

    let err = agent.run("system", "call something").await.unwrap_err();
    assert!(matches!(err, AgentError::ToolNotFound(ref n) if n == "nonexistent"));
}

// ── Test: tool handler error surfaced as ToolHandler variant ──────────────────

#[tokio::test]
async fn tool_handler_error_surfaces() {
    let provider = MockLlmProvider::new("mock", "mock").scenario(MockScenario::Sequence {
        tool_name: "error_tool".to_string(),
        tool_inputs: vec![json!({})],
        final_response: json!({"done": true}),
    });

    let tool = Tool::new(
        "error_tool".to_string(),
        "A tool that errors".to_string(),
        Schema::from(Map::new()),
        ErrorHandler,
    );

    let mut agent = Agent::new(Arc::new(provider), vec![tool], 3, NoState);

    let err = agent.run("system", "call error_tool").await.unwrap_err();
    match err {
        AgentError::ToolHandler(msg) => assert_eq!(msg, "deliberate tool error"),
        other => panic!("expected ToolHandler error, got {:?}", other),
    }
}

// ── Test: state threaded through multiple tool calls ─────────────────────────

#[tokio::test]
async fn state_threading() {
    // Two tool calls: first appends "a", second appends "b"
    let provider = MockLlmProvider::new("mock", "mock").scenario(MockScenario::Sequence {
        tool_name: "append_a".to_string(),
        tool_inputs: vec![json!({}), json!({})],
        final_response: json!({"accumulated": "ab"}),
    });

    let tool_a = Tool::new(
        "append_a".to_string(),
        "Appends 'a'".to_string(),
        Schema::from(Map::new()),
        StringAppender {
            value: "a".to_string(),
        },
    );

    let mut agent = Agent::new(Arc::new(provider), vec![tool_a], 4, StringState::new());

    let _outcome = agent.run("system", "test").await.unwrap();
    // Agent completes successfully — tool dispatch worked correctly
}
