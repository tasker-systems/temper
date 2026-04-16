//! OpenAI-compatible LLM provider (ollama, togetherai, groq, etc.)
//!
//! Uses the OpenAI Chat Completions API at `{base_url}/v1/chat/completions`.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::provider::{LlmError, LlmProvider, LlmResponse, Message, ToolCall, ToolSchema};

// ── Request types ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
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
    #[serde(rename = "type")]
    tool_type: String,
    function: ApiFunction,
}

#[derive(Debug, Serialize)]
struct ApiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ApiResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: ApiJsonSchema,
}

#[derive(Debug, Serialize)]
struct ApiJsonSchema {
    name: String,
    strict: bool,
}

// ── Response types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    error: Option<ResponseError>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AssistantMessage {
    WithContent {
        #[allow(dead_code)]
        role: String,
        content: Option<String>,
    },
    WithToolCalls {
        #[allow(dead_code)]
        role: String,
        tool_calls: Vec<ToolCallJson>,
    },
}

#[derive(Debug, Deserialize)]
struct ToolCallJson {
    id: String,
    #[serde(rename = "function")]
    function: FunctionJson,
}

#[derive(Debug, Deserialize)]
struct FunctionJson {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ResponseError {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Strip markdown JSON code fences that some models wrap around their responses.
///
/// e.g. ```json\n{"foo": "bar"}\n``` → {"foo": "bar"}
fn strip_json_fence(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json\n") {
        if let Some(content) = rest.strip_suffix("\n```") {
            return content.trim().to_string();
        }
    }
    if let Some(rest) = s.strip_prefix("```\n") {
        if let Some(content) = rest.strip_suffix("\n```") {
            return content.trim().to_string();
        }
    }
    if let Some(rest) = s.strip_prefix("```json") {
        // ```json ... ```  (no newline after ```json)
        if let Some(content) = rest.strip_prefix('`') {
            if let Some(c) = content.strip_prefix('\n') {
                if let Some(final_content) = c.strip_suffix("\n```") {
                    return final_content.trim().to_string();
                }
            }
        }
    }
    s.to_string()
}

// ── Provider implementation ─────────────────────────────────────────────────────

/// OpenAI-compatible LLM provider.
///
/// Handles ollama and any endpoint that implements the OpenAI Chat Completions
/// API (togetherai, groq, etc.). Bearer auth is optional — ollama does not
/// require an API key.
#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    client: Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatibleProvider {
    /// Create a new provider.
    ///
    /// `base_url` is the root of the API (e.g. `http://localhost:11434` for ollama).
    /// `model` is the model identifier (e.g. `llama3.2:latest`).
    /// `api_key` is optional — pass `None` for ollama.
    pub fn new(base_url: &str, model: &str, api_key: Option<&str>) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: api_key.map(String::from),
        })
    }

    fn build_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url)
    }

    fn auth_header(&self) -> Option<String> {
        self.api_key.as_ref().map(|k| format!("Bearer {k}"))
    }
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],
        response_format: Option<&schemars::Schema>,
    ) -> Result<LlmResponse, LlmError> {
        // Build messages: prepend system message if non-empty.
        let mut api_messages = Vec::with_capacity(messages.len() + 1);
        if !system.is_empty() {
            api_messages.push(ApiMessage {
                role: "system".to_string(),
                content: system.to_string(),
            });
        }
        for msg in messages {
            api_messages.push(ApiMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            });
        }

        // Build tools list only when non-empty.
        let api_tools = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| ApiTool {
                        tool_type: "function".to_string(),
                        function: ApiFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: serde_json::to_value(&t.input_schema)
                                .unwrap_or(serde_json::Value::Object(Default::default())),
                        },
                    })
                    .collect(),
            )
        };

        // tool_choice: "auto" only when tools are present.
        let tool_choice = api_tools.as_ref().map(|_| "auto".to_string());

        // response_format: when provided, include it.
        let api_response_format = response_format.and_then(|schema| {
            let json_value = serde_json::to_value(schema).ok()?;
            let name = json_value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("output")
                .to_string();
            Some(ApiResponseFormat {
                format_type: "json_schema".to_string(),
                json_schema: ApiJsonSchema { name, strict: true },
            })
        });

        let request = ChatRequest {
            model: self.model.clone(),
            messages: api_messages,
            tools: api_tools,
            tool_choice,
            response_format: api_response_format,
        };

        // Build request headers.
        let mut req_builder = self.client.post(self.build_url());
        if let Some(auth) = self.auth_header() {
            req_builder = req_builder.header("Authorization", auth);
        }
        req_builder = req_builder.header("Content-Type", "application/json");

        let response = req_builder
            .json(&request)
            .send()
            .await
            .map_err(|e| LlmError::Provider(e.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| LlmError::Provider(e.to_string()))?;

        if !status.is_success() {
            // Try to parse error body; fall back to a generic message.
            if let Ok(err_resp) = serde_json::from_str::<ChatResponse>(&body) {
                if let Some(err) = err_resp.error {
                    if err.error_type.as_deref() == Some("rate_limit") {
                        return Err(LlmError::RateLimit);
                    }
                    return Err(LlmError::Provider(err.message));
                }
            }
            return Err(LlmError::Provider(format!("HTTP {status}: {body}")));
        }

        let chat_resp: ChatResponse =
            serde_json::from_str(&body).map_err(|e| LlmError::Provider(e.to_string()))?;

        if let Some(err) = chat_resp.error {
            if err.error_type.as_deref() == Some("rate_limit") {
                return Err(LlmError::RateLimit);
            }
            return Err(LlmError::Provider(err.message));
        }

        let choice = chat_resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Provider("empty choices array".to_string()))?;

        match choice.message {
            AssistantMessage::WithContent { content, .. } => {
                let content = content.unwrap_or_default();
                let cleaned = strip_json_fence(&content);
                let parsed: serde_json::Value =
                    serde_json::from_str(&cleaned).unwrap_or(serde_json::Value::String(content));
                Ok(LlmResponse::Final { content: parsed })
            }
            AssistantMessage::WithToolCalls { tool_calls, .. } => {
                let calls = tool_calls
                    .into_iter()
                    .map(|tc| {
                        let arguments: serde_json::Value =
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                        ToolCall {
                            id: tc.id,
                            name: tc.function.name,
                            input: arguments,
                        }
                    })
                    .collect();
                Ok(LlmResponse::ToolUse { calls })
            }
        }
    }

    fn provider_name(&self) -> &str {
        "openai-compatible"
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_constructs() {
        let p = OpenAiCompatibleProvider::new("http://localhost:11434", "llama3.2:latest", None)
            .unwrap();
        assert_eq!(p.provider_name(), "openai-compatible");
        assert_eq!(p.model(), "llama3.2:latest");
    }

    #[test]
    fn provider_constructs_with_api_key() {
        let p = OpenAiCompatibleProvider::new(
            "https://api.groq.com",
            "llama-3.2-90b-vision-preview",
            Some("sk-..."),
        )
        .unwrap();
        assert_eq!(p.model(), "llama-3.2-90b-vision-preview");
    }

    #[test]
    fn debug_does_not_leak_api_key() {
        let p = OpenAiCompatibleProvider::new(
            "http://localhost:11434",
            "llama3.2:latest",
            Some("secret-key"),
        )
        .unwrap();
        let debug = format!("{p:?}");
        assert!(!debug.contains("secret-key"));
        assert!(debug.contains("OpenAiCompatibleProvider"));
    }
}
