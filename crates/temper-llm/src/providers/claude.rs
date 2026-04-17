// providers/claude.rs — Anthropic Messages API provider

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::time::Duration;

use crate::provider::{LlmError, LlmProvider, LlmResponse, Message, ToolCall, ToolSchema};

// ── Retry constants ────────────────────────────────────────────────────────────

/// Total number of attempts (initial try + retries).
const MAX_ATTEMPTS: u32 = 3;

/// Base delay in milliseconds between retries; doubled each attempt
/// (1s before retry #2, 2s before retry #3).
const BASE_DELAY_MS: u64 = 1000;

/// Default Anthropic Messages API endpoint.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Crate-internal error wrapper used by `complete_once` so the retry loop can
/// distinguish transient failures (retry) from permanent ones (propagate).
enum AttemptError {
    Transient(String),
    Permanent(LlmError),
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<ApiMessage>,
    tools: Vec<ApiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ApiResponseFormat>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ApiResponseFormat {
    #[serde(rename = "type")]
    format_type: &'static str,
    json_schema: ApiJsonSchema,
}

#[derive(Debug, Serialize)]
struct ApiJsonSchema {
    name: String,
    strict: bool,
}

// ── Response types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    #[serde(rename = "type")]
    response_type: String,
    content: Vec<ResponseContent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponseContent {
    Text {
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(rename = "type")]
    error_type: String,
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

// ── Provider ───────────────────────────────────────────────────────────────────

/// Anthropic Messages API provider.
pub struct ClaudeProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl ClaudeProvider {
    /// Create a new Anthropic provider.
    ///
    /// `timeout_secs` is the HTTP request timeout applied to the reqwest client.
    ///
    /// # Errors
    /// Returns an error if the API key is empty.
    pub fn new(model: &str, api_key: String, timeout_secs: u64) -> Result<Self, String> {
        if api_key.is_empty() {
            return Err("API key must not be empty".to_string());
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| e.to_string())?;

        // Test hook: `TEMPER_LLM_CLAUDE_BASE_URL_OVERRIDE` redirects requests
        // to a mock server for integration tests. The env var is never set in
        // production and only consulted once at construction time, so the cost
        // is negligible and the alternative (cfg-gated resolver) does not work
        // for `tests/` targets which compile the lib without `cfg(test)`.
        let base_url = std::env::var("TEMPER_LLM_CLAUDE_BASE_URL_OVERRIDE")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

        Ok(Self {
            client,
            api_key,
            model: model.to_string(),
            base_url,
        })
    }
}

impl Debug for ClaudeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeProvider")
            .field("model", &self.model)
            .finish()
    }
}

impl ClaudeProvider {
    /// Single attempt against the Messages API. Returns `AttemptError::Transient`
    /// for retryable failures (network errors, HTTP 5xx, HTTP 429) and
    /// `AttemptError::Permanent` for non-retryable ones (other 4xx, JSON parse
    /// failures, unexpected response types).
    async fn complete_once(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],
        response_format: Option<&schemars::Schema>,
    ) -> Result<LlmResponse, AttemptError> {
        let api_messages: Vec<ApiMessage> = messages
            .iter()
            .map(|m| ApiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let api_tools: Vec<ApiTool> = tools
            .iter()
            .map(|t| {
                let json_schema =
                    serde_json::to_value(&t.input_schema).unwrap_or(serde_json::Value::Null);
                ApiTool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: json_schema,
                }
            })
            .collect();

        let api_response_format = response_format.and_then(|schema| {
            let json_value = serde_json::to_value(schema).ok()?;
            let name = json_value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("output")
                .to_string();
            Some(ApiResponseFormat {
                format_type: "json_schema",
                json_schema: ApiJsonSchema { name, strict: true },
            })
        });

        let request = MessagesRequest {
            model: self.model.clone(),
            max_tokens: 8192,
            system: if system.is_empty() {
                None
            } else {
                Some(system.to_string())
            },
            messages: api_messages,
            tools: api_tools,
            response_format: api_response_format,
        };

        // Transport-level errors are always transient.
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AttemptError::Transient(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let is_retryable = status.is_server_error() || status.as_u16() == 429;

            // Attempt to parse as API error for structured error type mapping
            let permanent_err = if let Ok(api_err) = serde_json::from_str::<ApiError>(&body) {
                match api_err.error_type.as_str() {
                    "rate_limit" => LlmError::RateLimit,
                    "timeout" => LlmError::Timeout,
                    _ => LlmError::Provider(api_err.error.message),
                }
            } else {
                LlmError::Provider(format!("HTTP {}: {}", status.as_u16(), body))
            };

            return if is_retryable {
                Err(AttemptError::Transient(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    body
                )))
            } else {
                Err(AttemptError::Permanent(permanent_err))
            };
        }

        // JSON parse failures on a 2xx body are semantic, not transport.
        let api_response: MessagesResponse = response
            .json()
            .await
            .map_err(|e| AttemptError::Permanent(LlmError::Provider(e.to_string())))?;

        match api_response.response_type.as_str() {
            "message" => {
                let mut tool_calls = Vec::new();
                let mut final_text = None;

                for content in api_response.content {
                    match content {
                        ResponseContent::Text { text } => {
                            final_text = Some(text);
                        }
                        ResponseContent::ToolUse { id, name, input } => {
                            tool_calls.push(ToolCall { id, name, input });
                        }
                    }
                }

                if !tool_calls.is_empty() {
                    Ok(LlmResponse::ToolUse { calls: tool_calls })
                } else {
                    // Final text response — parse as structured JSON
                    let text = final_text.unwrap_or_default();
                    let content: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or_else(|_| {
                            // If parsing fails, return the raw text as a JSON string
                            serde_json::json!({ "text": text })
                        });
                    Ok(LlmResponse::Final { content })
                }
            }
            other => Err(AttemptError::Permanent(LlmError::Model(format!(
                "unexpected response type: {}",
                other
            )))),
        }
    }
}

#[async_trait]
impl LlmProvider for ClaudeProvider {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],
        response_format: Option<&schemars::Schema>,
    ) -> Result<LlmResponse, LlmError> {
        let mut last_transient: Option<String> = None;

        for attempt in 1..=MAX_ATTEMPTS {
            match self
                .complete_once(system, messages, tools, response_format)
                .await
            {
                Ok(resp) => return Ok(resp),
                Err(AttemptError::Permanent(err)) => return Err(err),
                Err(AttemptError::Transient(msg)) => {
                    if attempt < MAX_ATTEMPTS {
                        tracing::warn!(
                            attempt,
                            max_attempts = MAX_ATTEMPTS,
                            error = %msg,
                            "transient provider error, retrying"
                        );
                        let delay_ms = BASE_DELAY_MS << (attempt - 1);
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    last_transient = Some(msg);
                }
            }
        }

        Err(LlmError::Provider(
            last_transient.unwrap_or_else(|| "retry attempts exhausted".to_string()),
        ))
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }
}
