// llm.rs — LLM provider factory for temper-cli

use std::sync::Arc;

use temper_core::types::config::{LlmConfig, LlmProviderType};
use temper_llm::{ClaudeProvider, LlmProvider, OpenAiCompatibleProvider};

/// Build an LLM provider from config.
///
/// Precedence: `TEMPER_LLM_API_KEY` env var → config.api_key (file).
/// For Claude, `ANTHROPIC_API_KEY` is always required.
/// For Ollama/OpenAI-compatible, API key is optional.
pub async fn build_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, String> {
    match config.provider {
        LlmProviderType::Claude => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| "ANTHROPIC_API_KEY environment variable is not set".to_string())?;
            let provider =
                ClaudeProvider::new(&config.model, api_key, config.request_timeout_secs)?;
            Ok(Arc::new(provider))
        }
        LlmProviderType::Ollama | LlmProviderType::OpenAiCompatible => {
            let api_key = std::env::var("TEMPER_LLM_API_KEY")
                .ok()
                .or_else(|| config.api_key.clone());
            let provider = OpenAiCompatibleProvider::new(
                &config.url,
                &config.model,
                api_key.as_deref(),
                config.request_timeout_secs,
            )?;
            Ok(Arc::new(provider))
        }
    }
}
