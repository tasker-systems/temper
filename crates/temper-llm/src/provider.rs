// provider.rs — LLM provider trait and types
// TODO(task-3): implement LlmProvider trait with Anthropic and OpenAI backends

use async_trait::async_trait;
pub use schemars::JsonSchema;

/// Message sent to the LLM.
#[derive(Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Tool call returned when the model requests a tool invocation.
#[derive(Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Schema for a tool the model may invoke.
#[derive(Debug)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: schemars::Schema,
}

/// LLM error variants.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("model error: {0}")]
    Model(String),
    #[error("rate limit")]
    RateLimit,
    #[error("timeout")]
    Timeout,
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        Self::Provider(e.to_string())
    }
}

/// Successful LLM response.
#[derive(Debug)]
pub enum LlmResponse {
    /// Model produced a final structured response.
    Final { content: serde_json::Value },
    /// Model requested one or more tool calls.
    ToolUse { calls: Vec<ToolCall> },
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a completion.
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],
        response_format: Option<&schemars::Schema>,
    ) -> Result<LlmResponse, LlmError>;

    /// Canonical name of this provider (e.g. "anthropic", "openai").
    fn provider_name(&self) -> &str;

    /// Model identifier (e.g. "claude-sonnet-4-20250514").
    fn model(&self) -> &str;
}
