//! temper-llm-smoke — smoke-test binary for the temper-llm provider layer
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use temper_llm::{
    Agent, AgentOutcome, ClaudeProvider, LlmProvider, LlmResponse, Message,
    OpenAiCompatibleProvider,
};

#[derive(Parser)]
#[command(name = "temper-llm-smoke")]
struct Args {
    /// Provider: "ollama", "claude", or "openai"
    #[arg(long, default_value = "ollama")]
    provider: String,

    /// Base URL for the API
    #[arg(long)]
    url: Option<String>,

    /// Model identifier
    #[arg(long)]
    model: Option<String>,

    /// System prompt
    #[arg(long, default_value = "You are a helpful assistant.")]
    system: String,

    /// User prompt (positional)
    #[arg(long)]
    prompt: Option<String>,

    /// JSON Schema for structured output (tests response_format)
    #[arg(long)]
    schema: Option<String>,

    /// Max turns for the agent loop (default 1 = single-turn)
    #[arg(long, default_value = "1")]
    max_turns: usize,

    /// Output machine-readable JSON
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("temper_llm_smoke=info".parse().unwrap())
                .add_directive("temper_llm=debug".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    // Build provider
    let (provider, provider_name, model_name) = build_provider(&args)
        .await
        .context("failed to build provider")?;

    // Parse schema if provided
    let schema: Option<schemars::Schema> = args
        .schema
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok());

    let system = &args.system;
    let prompt = args.prompt.as_deref().unwrap_or(
        "Say hello in JSON with fields: message (string), model (string). Respond with valid JSON only.",
    );

    let start = Instant::now();

    if args.max_turns == 1 && schema.is_some() {
        // Single-turn structured output: call provider directly
        let result = provider
            .complete(
                system,
                &[Message {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                }],
                &[],
                schema.as_ref(),
            )
            .await;

        let elapsed = start.elapsed();

        match result {
            Ok(LlmResponse::Final { content }) => {
                print_result_json(&provider_name, &model_name, elapsed, &content, None);
            }
            Ok(LlmResponse::ToolUse { calls }) => {
                print_error_json(
                    &provider_name,
                    &model_name,
                    elapsed,
                    "unexpected_tool_use",
                    Some(serde_json::json!({ "calls": calls.len() })),
                );
            }
            Err(e) => {
                print_error_json(&provider_name, &model_name, elapsed, &e.to_string(), None);
            }
        }
    } else {
        // Multi-turn or no schema: use Agent harness
        let mut agent = Agent::new(Arc::clone(&provider), vec![], args.max_turns, ());

        let outcome = agent.run(system, prompt).await;
        let elapsed = start.elapsed();

        match outcome {
            Ok(AgentOutcome::Final { content }) => {
                print_result_json(&provider_name, &model_name, elapsed, &content, None);
            }
            Ok(AgentOutcome::MaxTurns) => {
                print_error_json(
                    &provider_name,
                    &model_name,
                    elapsed,
                    &format!("max_turns_reached ({})", args.max_turns),
                    None,
                );
            }
            Err(e) => {
                print_error_json(&provider_name, &model_name, elapsed, &e.to_string(), None);
            }
        }
    }

    Ok(())
}

fn print_result_json(
    provider_name: &str,
    model_name: &str,
    elapsed: std::time::Duration,
    content: &serde_json::Value,
    extra: Option<serde_json::Value>,
) {
    let mut obj = serde_json::json!({
        "ok": true,
        "provider": provider_name,
        "model": model_name,
        "elapsed_ms": elapsed.as_millis() as u64,
        "content": content,
    });
    if let Some(e) = extra {
        obj["extra"] = e;
    }
    println!("{obj}");
}

fn print_error_json(
    provider_name: &str,
    model_name: &str,
    elapsed: std::time::Duration,
    error: &str,
    extra: Option<serde_json::Value>,
) {
    let mut obj = serde_json::json!({
        "ok": false,
        "provider": provider_name,
        "model": model_name,
        "elapsed_ms": elapsed.as_millis() as u64,
        "error": error,
    });
    if let Some(e) = extra {
        obj["extra"] = e;
    }
    println!("{obj}");
}

async fn build_provider(args: &Args) -> anyhow::Result<(Arc<dyn LlmProvider>, String, String)> {
    let provider_key = args.provider.to_lowercase();
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| match provider_key.as_str() {
            "claude" => "claude-sonnet-4-5".to_string(),
            _ => "llama3.2:latest".to_string(),
        });

    match provider_key.as_str() {
        "claude" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                anyhow::anyhow!("--provider claude requires ANTHROPIC_API_KEY env var")
            })?;
            let p = ClaudeProvider::new(&model, api_key)
                .map_err(|e| anyhow::anyhow!("ClaudeProvider::new: {e}"))?;
            Ok((Arc::new(p), "anthropic".to_string(), model))
        }
        "ollama" | "openai" | _ => {
            let url = args
                .url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            let api_key = std::env::var("TEMPER_LLM_API_KEY")
                .ok()
                .filter(|k| !k.is_empty());
            let p = OpenAiCompatibleProvider::new(&url, &model, api_key.as_deref())
                .map_err(|e| anyhow::anyhow!("OpenAiCompatibleProvider::new: {e}"))?;
            Ok((Arc::new(p), "openai-compatible".to_string(), model))
        }
    }
}
