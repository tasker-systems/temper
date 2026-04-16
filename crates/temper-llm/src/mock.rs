// mock.rs — MockLlmProvider for tests

use async_trait::async_trait;
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::provider::{LlmError, LlmProvider, LlmResponse, Message, ToolSchema};

/// Scenario driving what a mock provider returns.
#[derive(Debug, Clone)]
pub enum MockScenario {
    /// Always return a final response immediately (single-turn).
    SingleTurn(Value),
    /// Sequence of responses: ToolUse N times, then Final.
    Sequence {
        /// Name of the tool to call on each tool-use turn.
        tool_name: String,
        /// Inputs for each tool call in order.
        tool_inputs: Vec<Value>,
        /// Final response once tool_inputs are exhausted.
        final_response: Value,
    },
}

/// In-memory mock provider for testing.
pub struct MockLlmProvider {
    name: String,
    model: String,
    scenario: MockScenario,
    call_count: AtomicUsize,
}

impl MockLlmProvider {
    pub fn new(name: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            model: model.into(),
            scenario: MockScenario::SingleTurn(serde_json::json!({})),
            call_count: AtomicUsize::new(0),
        }
    }

    pub fn scenario(mut self, scenario: MockScenario) -> Self {
        self.scenario = scenario;
        self
    }

    /// Current call count (snapshot).
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn complete(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[ToolSchema],
        _response_format: Option<&schemars::schema::Schema>,
    ) -> Result<LlmResponse, LlmError> {
        use MockScenario::*;
        let count = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;

        match &self.scenario {
            SingleTurn(content) => Ok(LlmResponse::Final {
                content: content.clone(),
            }),
            Sequence {
                tool_name,
                tool_inputs,
                final_response,
            } => {
                let tool_idx = count.saturating_sub(1);
                if tool_idx < tool_inputs.len() {
                    Ok(LlmResponse::ToolUse {
                        calls: vec![crate::provider::ToolCall {
                            id: format!("call_{}", tool_idx),
                            name: tool_name.clone(),
                            input: tool_inputs[tool_idx].clone(),
                        }],
                    })
                } else {
                    Ok(LlmResponse::Final {
                        content: final_response.clone(),
                    })
                }
            }
        }
    }

    fn provider_name(&self) -> &str {
        &self.name
    }

    fn model(&self) -> &str {
        &self.model
    }
}
