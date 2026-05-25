//! temper-llm — LLM provider abstraction and agent harness
//!
//! Provides a vendor-agnostic `LlmProvider` trait and an `Agent<S>` harness
//! that loops tool-use turns until the model produces a final response or
//! `max_turns` is reached.

mod agent;
mod mock;
mod provider;
pub mod providers;

pub use agent::{Agent, AgentError, AgentOutcome, Tool, ToolHandler};
pub use mock::{MockLlmProvider, MockScenario};
pub use provider::{JsonSchema, LlmError, LlmProvider, LlmResponse, Message, ToolCall, ToolSchema};
pub use providers::claude::ClaudeProvider;
pub use providers::openai_compatible::OpenAiCompatibleProvider;
pub use schemars::Schema;
