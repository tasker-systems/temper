// Candle embedding model management
use std::path::PathBuf;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use hf_hub::api::sync::Api;
use tokenizers::Tokenizer;

use crate::error::{Result, TemperError};

const MODEL_ID: &str = "sentence-transformers/all-MiniLM-L6-v2";
const DIMENSIONS: usize = 384;

struct LoadedModel {
    model: BertModel,
    tokenizer: Tokenizer,
}

pub struct Embedder {
    model: Option<LoadedModel>,
    cache_dir: PathBuf,
}

impl Embedder {
    /// Creates a new Embedder — cheap, stores path only. Model is loaded lazily.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            model: None,
            cache_dir,
        }
    }

    /// Downloads the model from HuggingFace if not cached, then loads it into memory.
    pub fn ensure_model(&mut self) -> Result<()> {
        if self.model.is_some() {
            return Ok(());
        }

        // Use default HuggingFace cache (~/.cache/huggingface/) for model storage.
        // The cache_dir field is kept for future use but hf-hub manages its own cache.
        let api = Api::new()
            .map_err(|e| TemperError::Embedding(format!("hf-hub API init failed: {e}")))?;

        let repo = api.model(MODEL_ID.to_string());

        let model_path = repo
            .get("model.safetensors")
            .map_err(|e| TemperError::Embedding(format!("failed to get model.safetensors: {e}")))?;

        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| TemperError::Embedding(format!("failed to get tokenizer.json: {e}")))?;

        let config_path = repo
            .get("config.json")
            .map_err(|e| TemperError::Embedding(format!("failed to get config.json: {e}")))?;

        let config_bytes = std::fs::read(&config_path)
            .map_err(|e| TemperError::Embedding(format!("failed to read config.json: {e}")))?;
        let config: Config = serde_json::from_slice(&config_bytes)
            .map_err(|e| TemperError::Embedding(format!("failed to parse config.json: {e}")))?;

        let device = Device::Cpu;

        // Safety: the mmap is read-only from an existing file on disk
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                .map_err(|e| TemperError::Embedding(format!("failed to load safetensors: {e}")))?
        };

        let model = BertModel::load(vb, &config)
            .map_err(|e| TemperError::Embedding(format!("failed to load BertModel: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| TemperError::Embedding(format!("failed to load tokenizer: {e}")))?;

        self.model = Some(LoadedModel { model, tokenizer });
        Ok(())
    }

    /// Returns the embedding dimensionality (384 for all-MiniLM-L6-v2).
    pub fn dimensions(&self) -> usize {
        DIMENSIONS
    }

    /// Embeds a single text string, returning a L2-normalized vector of length 384.
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        self.ensure_model()?;
        let loaded = self
            .model
            .as_ref()
            .expect("model must be loaded after ensure_model");

        let encoding = loaded
            .tokenizer
            .encode(text, true)
            .map_err(|e| TemperError::Embedding(format!("tokenization failed: {e}")))?;

        let input_ids: Vec<u32> = encoding.get_ids().to_vec();
        let token_type_ids: Vec<u32> = encoding.get_type_ids().to_vec();
        let attention_mask: Vec<u32> = encoding.get_attention_mask().to_vec();

        let device = &loaded.model.device;
        let seq_len = input_ids.len();

        let input_ids_tensor = Tensor::from_vec(input_ids, (1, seq_len), device)
            .map_err(|e| TemperError::Embedding(format!("tensor creation failed: {e}")))?;
        let token_type_ids_tensor = Tensor::from_vec(token_type_ids, (1, seq_len), device)
            .map_err(|e| TemperError::Embedding(format!("tensor creation failed: {e}")))?;
        let attention_mask_tensor = Tensor::from_vec(attention_mask, (1, seq_len), device)
            .map_err(|e| TemperError::Embedding(format!("tensor creation failed: {e}")))?;

        let output = loaded
            .model
            .forward(
                &input_ids_tensor,
                &token_type_ids_tensor,
                Some(&attention_mask_tensor),
            )
            .map_err(|e| TemperError::Embedding(format!("model forward pass failed: {e}")))?;

        // Mean pool over token dimension: output shape is (1, seq_len, hidden_size)
        let pooled = output
            .mean(1)
            .map_err(|e| TemperError::Embedding(format!("mean pooling failed: {e}")))?;

        // Squeeze batch dimension: (1, hidden_size) -> (hidden_size,)
        let pooled = pooled
            .squeeze(0)
            .map_err(|e| TemperError::Embedding(format!("squeeze failed: {e}")))?;

        let vec: Vec<f32> = pooled
            .to_vec1()
            .map_err(|e| TemperError::Embedding(format!("tensor to_vec1 failed: {e}")))?;

        Ok(l2_normalize(vec))
    }

    /// Embeds a batch of texts, returning one vector per input.
    pub fn embed_batch(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
}

/// Prepend `header_path` with ": " separator, strip markdown link syntax,
/// and normalize whitespace. If `header_path` is empty, return content as-is
/// (after stripping and normalizing).
pub fn preprocess_chunk(content: &str, header_path: &str) -> String {
    use regex_lite::Regex;

    // Strip [[wikilinks]] -> inner text
    let re_wiki = Regex::new(r"\[\[([^\]]+)\]\]").expect("valid regex");
    let content = re_wiki.replace_all(content, "$1");

    // Strip [text](url) -> text
    let re_link = Regex::new(r"\[([^\]]+)\]\([^)]*\)").expect("valid regex");
    let content = re_link.replace_all(&content, "$1");

    // Strip ![alt](url) images -> alt text
    let re_img = Regex::new(r"!\[([^\]]*)\]\([^)]*\)").expect("valid regex");
    let content = re_img.replace_all(&content, "$1");

    // Normalize multiple blank lines (3+ newlines -> 2 newlines)
    let re_blanks = Regex::new(r"\n{3,}").expect("valid regex");
    let content = re_blanks.replace_all(&content, "\n\n");

    let content = content.trim().to_string();

    if header_path.is_empty() {
        content
    } else {
        format!("{}: {}", header_path, content)
    }
}

/// Pass-through: returns the input unchanged.
pub fn preprocess_frontmatter(text: &str) -> String {
    text.to_string()
}

fn l2_normalize(mut vec: Vec<f32>) -> Vec<f32> {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}
