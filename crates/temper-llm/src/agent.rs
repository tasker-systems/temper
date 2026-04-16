// agent.rs — Agent<S> harness

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tracing::instrument::Instrument;

use crate::provider::{LlmError, LlmProvider, LlmResponse, Message, ToolCall, ToolSchema};

/// Outcome of a completed agent run.
#[derive(Debug)]
pub enum AgentOutcome {
    /// Model produced a final structured response.
    Final { content: Value },
    /// Max turns reached without a final response.
    MaxTurns,
}

/// Errors raised by the agent harness.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),
    #[error("too many turns")]
    TooManyTurns,
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("tool handler error: {0}")]
    ToolHandler(String),
}

/// Tool handler invoked by the agent turn loop.
#[async_trait]
pub trait ToolHandler<S>: Send + Sync {
    async fn call(&self, input: &Value, state: &mut S) -> Result<Value, String>;
}

/// Tool with a handler closure.
pub struct Tool<S> {
    pub name: String,
    pub description: String,
    pub input_schema: schemars::Schema,
    pub handler: Box<dyn ToolHandler<S>>,
}

impl<S> Tool<S> {
    pub fn new(
        name: String,
        description: String,
        input_schema: impl Into<schemars::Schema>,
        handler: impl ToolHandler<S> + 'static,
    ) -> Self {
        Self {
            name,
            description,
            input_schema: input_schema.into(),
            handler: Box::new(handler),
        }
    }
}

/// Agent that loops tool-use turns until the model produces a final response.
pub struct Agent<S> {
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Tool<S>>,
    max_turns: usize,
    state: S,
}

impl<S> Agent<S> {
    /// Create a new agent.
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: Vec<Tool<S>>,
        max_turns: usize,
        state: S,
    ) -> Self {
        Self {
            provider,
            tools,
            max_turns,
            state,
        }
    }

    /// Run the agent with the given system prompt and user message.
    pub async fn run(&mut self, system: &str, user: &str) -> Result<AgentOutcome, AgentError> {
        let span = tracing::info_span!("agent_run", max_turns = self.max_turns);
        self.run_internal(system, user).instrument(span).await
    }

    async fn run_internal(&mut self, system: &str, user: &str) -> Result<AgentOutcome, AgentError> {
        let mut messages = vec![Message {
            role: "user".to_string(),
            content: user.to_string(),
        }];

        for turn in 0..self.max_turns {
            tracing::debug!(turn, "agent turn");

            let tool_schemas: Vec<ToolSchema> = self
                .tools
                .iter()
                .map(|t| ToolSchema {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: t.input_schema.clone(),
                })
                .collect();

            let response = self
                .provider
                .complete(system, &messages, &tool_schemas, None)
                .await?;

            match response {
                LlmResponse::Final { content } => {
                    return Ok(AgentOutcome::Final { content });
                }
                LlmResponse::ToolUse { calls } => {
                    for call in calls {
                        let result = self.dispatch_tool(&call).await?;
                        messages.push(Message {
                            role: "user".to_string(),
                            content: serde_json::json!({
                                "tool_call_id": call.id,
                                "content": result,
                            })
                            .to_string(),
                        });
                    }
                }
            }
        }

        Ok(AgentOutcome::MaxTurns)
    }

    async fn dispatch_tool(&mut self, call: &ToolCall) -> Result<String, AgentError> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name == call.name)
            .ok_or_else(|| AgentError::ToolNotFound(call.name.clone()))?;

        let input_val: Value = call.input.clone();
        let result = tool
            .handler
            .call(&input_val, &mut self.state)
            .await
            .map_err(|e| AgentError::ToolHandler(e))?;

        Ok(result.to_string())
    }
}
