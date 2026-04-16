// providers/claude.rs — Anthropic Messages API provider

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::time::Duration;

use crate::provider::{LlmError, LlmProvider, LlmResponse, Message, ToolCall, ToolSchema};

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
}

impl ClaudeProvider {
    /// Create a new Anthropic provider.
    ///
    /// # Errors
    /// Returns an error if the API key is empty.
    pub fn new(model: &str, api_key: String) -> Result<Self, String> {
        if api_key.is_empty() {
            return Err("API key must not be empty".to_string());
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            client,
            api_key,
            model: model.to_string(),
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

#[async_trait]
impl LlmProvider for ClaudeProvider {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],
        response_format: Option<&schemars::Schema>,
    ) -> Result<LlmResponse, LlmError> {
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

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            // Attempt to parse as API error for structured error type mapping
            if let Ok(api_err) = serde_json::from_str::<ApiError>(&body) {
                match api_err.error_type.as_str() {
                    "rate_limit" => return Err(LlmError::RateLimit),
                    "timeout" => return Err(LlmError::Timeout),
                    _ => return Err(LlmError::Provider(api_err.error.message)),
                }
            }
            return Err(LlmError::Provider(format!(
                "HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }

        let api_response: MessagesResponse = response.json().await?;

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
            other => Err(LlmError::Model(format!(
                "unexpected response type: {}",
                other
            ))),
        }
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }
}
