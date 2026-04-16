//! temper-llm — LLM provider abstraction and agent harness
//!
//! Provides a vendor-agnostic `LlmProvider` trait and an `Agent<S>` harness
//! that loops tool-use turns until the model produces a final response or
//! `max_turns` is reached.

mod agent;
mod mock;
mod provider;

pub use agent::{Agent, AgentError, AgentOutcome, Tool, ToolHandler};
pub use mock::{MockLlmProvider, MockScenario};
pub use provider::{JsonSchema, LlmError, LlmProvider, LlmResponse, Message, ToolCall, ToolSchema};
pub use schemars::schema::Schema;
