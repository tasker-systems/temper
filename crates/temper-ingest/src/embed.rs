//! Text embedding using BAAI/bge-base-en-v1.5 via ONNX Runtime.
//!
//! Model and tokenizer loaded from bundled bytes at compile time (no runtime
//! downloads).  ONNX session created once per process via `OnceLock`.
//!
//! ORT runtime loading is platform-aware:
//! - **Linux** (Vercel deploy): the bundled `libonnxruntime.so` is written to
//!   `/tmp` and loaded via `ort::init_from`.
//! - **Other platforms** (macOS dev): ORT is loaded from the system library
//!   path.  Install via `brew install onnxruntime` and set `ORT_DYLIB_PATH`
//!   if needed.
//!
//! Pipeline: tokenize -> build tensors -> inference -> mean pool -> normalize

use std::sync::{Mutex, OnceLock};

use ndarray::{Array2, ArrayView3};
use ort::session::Session;
use ort::value::TensorRef;
use tokenizers::{Encoding, Tokenizer};

use crate::error::{EmbedError, Result};

/// Embedding dimension for bge-base-en-v1.5.
pub const EMBEDDING_DIM: usize = 768;

static MODEL_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/model_quantized.onnx");
static TOKENIZER_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/tokenizer.json");

/// Bundled Linux x86_64 libonnxruntime.so — only compiled into the binary on
/// Linux targets (Vercel deploy).  On other platforms this is a zero-length
/// slice and the system-installed ORT is used instead.
#[cfg(target_os = "linux")]
static ORT_LIB_BYTES: &[u8] = include_bytes!("../lib/x86_64-unknown-linux-gnu/libonnxruntime.so");

// ---- ORT runtime initialization ----

static ORT_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Initialize the ORT runtime.
///
/// On Linux: write the bundled `.so` to `/tmp` and load it explicitly.
/// On other platforms: let ORT's `load-dynamic` search system paths
/// (`ORT_DYLIB_PATH`, Homebrew, etc.).
fn init_ort_runtime() -> std::result::Result<(), String> {
    ORT_INIT.get_or_init(|| {
        #[cfg(target_os = "linux")]
        {
            let lib_path = std::path::Path::new("/tmp/libonnxruntime.so");
            if !lib_path.exists() {
                std::fs::write(lib_path, ORT_LIB_BYTES)
                    .map_err(|e| format!("write libonnxruntime.so to /tmp: {e}"))?;
            }
            ort::init_from(lib_path)
                .map_err(|e| format!("ort::init_from: {e}"))?
                .commit();
        }

        #[cfg(not(target_os = "linux"))]
        {
            // On macOS / other platforms, ORT's load-dynamic feature searches:
            //   1. ORT_DYLIB_PATH env var
            //   2. Standard library search paths (e.g. /opt/homebrew/lib)
            // No explicit init needed — ort handles it on first session creation.
        }

        Ok(())
    });
    match ORT_INIT.get() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => Err(e.clone()),
        None => Err("ORT_INIT not initialized".to_owned()),
    }
}

// ---- Model management ----

struct Model {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

static MODEL: OnceLock<std::result::Result<Model, String>> = OnceLock::new();

fn load_model() -> Result<&'static Model> {
    let result = MODEL.get_or_init(|| {
        init_ort_runtime().map_err(|e| format!("ort runtime init: {e}"))?;

        let session = Session::builder()
            .map_err(|e| format!("ort session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| format!("ort threads: {e}"))?
            .commit_from_memory(MODEL_BYTES)
            .map_err(|e| format!("ort load: {e}"))?;

        let tokenizer =
            Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| format!("load tokenizer: {e}"))?;

        Ok(Model {
            session: Mutex::new(session),
            tokenizer,
        })
    });

    match result {
        Ok(m) => Ok(m),
        Err(e) => Err(EmbedError::Embedding(format!("model init: {e}"))),
    }
}

// ---- Tokenization ----

/// Tokenize a batch of texts using the model's tokenizer.
pub fn tokenize(tokenizer: &Tokenizer, texts: &[&str]) -> Result<Vec<Encoding>> {
    tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|e| EmbedError::Embedding(format!("tokenize: {e}")))
}

// ---- Tensor construction ----

/// Input tensors for the ONNX model.
pub struct InputTensors {
    pub input_ids: Array2<i64>,
    pub attention_mask: Array2<i64>,
    pub token_type_ids: Array2<i64>,
}

/// Build input tensors from tokenizer encodings.
pub fn build_input_tensors(encodings: &[Encoding]) -> InputTensors {
    let batch_size = encodings.len();
    let max_len = encodings
        .iter()
        .map(|e| e.get_ids().len())
        .max()
        .unwrap_or(0);

    let mut input_ids = Array2::<i64>::zeros((batch_size, max_len));
    let mut attention_mask = Array2::<i64>::zeros((batch_size, max_len));
    let mut token_type_ids = Array2::<i64>::zeros((batch_size, max_len));

    for (i, enc) in encodings.iter().enumerate() {
        for (j, &id) in enc.get_ids().iter().enumerate() {
            input_ids[[i, j]] = id as i64;
        }
        for (j, &mask) in enc.get_attention_mask().iter().enumerate() {
            attention_mask[[i, j]] = mask as i64;
        }
        for (j, &tid) in enc.get_type_ids().iter().enumerate() {
            token_type_ids[[i, j]] = tid as i64;
        }
    }

    InputTensors {
        input_ids,
        attention_mask,
        token_type_ids,
    }
}

// ---- Pooling and normalization ----

/// Mean pooling: average hidden states weighted by attention mask.
pub fn mean_pool(hidden_states: ArrayView3<f32>, attention_mask: &Array2<i64>) -> Vec<Vec<f32>> {
    let batch_size = hidden_states.shape()[0];
    let max_len = hidden_states.shape()[1];
    let dim = hidden_states.shape()[2];
    let mut results = Vec::with_capacity(batch_size);

    for i in 0..batch_size {
        let mut embedding = vec![0f32; dim];
        let mut mask_sum = 0f32;

        for j in 0..max_len {
            let m = attention_mask[[i, j]] as f32;
            mask_sum += m;
            for k in 0..dim {
                embedding[k] += hidden_states[[i, j, k]] * m;
            }
        }

        if mask_sum > 0.0 {
            for v in &mut embedding {
                *v /= mask_sum;
            }
        }

        results.push(embedding);
    }

    results
}

/// L2-normalize a vector in place.
pub fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

// ---- Token limit ----

/// Hard token limit for bge-base-en-v1.5 (including special tokens).
const MAX_MODEL_TOKENS: usize = 512;

/// Truncate an encoding to `max_len` tokens (modifying in place).
fn truncate_encoding(enc: &mut Encoding, max_len: usize) {
    if enc.get_ids().len() <= max_len {
        return;
    }
    enc.truncate(max_len, 0, tokenizers::TruncationDirection::Right);
}

// ---- Public API ----

/// Count the actual tokens for a text using the model's tokenizer.
pub fn token_count(text: &str) -> Result<usize> {
    let model = load_model()?;
    let encodings = tokenize(&model.tokenizer, &[text])?;
    Ok(encodings[0].get_ids().len())
}

/// Embed a single text string into a 768-dim normalized vector.
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let mut results = embed_texts(&[text])?;
    Ok(results.remove(0))
}

/// Embed multiple texts into 768-dim normalized vectors.
///
/// Encodings that exceed 512 tokens are truncated to fit the model's input
/// limit.  The chunker should keep texts under budget, but this is a safety
/// net to prevent ONNX runtime crashes.
pub fn embed_texts(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let model = load_model()?;

    let mut encodings = tokenize(&model.tokenizer, texts)?;
    for enc in &mut encodings {
        truncate_encoding(enc, MAX_MODEL_TOKENS);
    }
    let tensors = build_input_tensors(&encodings);

    let input_ids_ref = TensorRef::from_array_view(tensors.input_ids.view())
        .map_err(|e| EmbedError::Embedding(format!("ort input_ids tensor: {e}")))?;
    let attention_mask_ref = TensorRef::from_array_view(tensors.attention_mask.view())
        .map_err(|e| EmbedError::Embedding(format!("ort attention_mask tensor: {e}")))?;
    let token_type_ids_ref = TensorRef::from_array_view(tensors.token_type_ids.view())
        .map_err(|e| EmbedError::Embedding(format!("ort token_type_ids tensor: {e}")))?;

    let mut session = model
        .session
        .lock()
        .map_err(|e| EmbedError::Embedding(format!("session lock: {e}")))?;

    let outputs = session
        .run(ort::inputs![
            input_ids_ref,
            attention_mask_ref,
            token_type_ids_ref
        ])
        .map_err(|e| EmbedError::Embedding(format!("ort run: {e}")))?;

    let hidden = outputs[0]
        .try_extract_array::<f32>()
        .map_err(|e| EmbedError::Embedding(format!("extract tensor: {e}")))?;

    let hidden_view = hidden
        .view()
        .into_dimensionality::<ndarray::Ix3>()
        .map_err(|e| EmbedError::Embedding(format!("reshape tensor: {e}")))?;

    let mut pooled = mean_pool(hidden_view, &tensors.attention_mask);
    for embedding in &mut pooled {
        l2_normalize(embedding);
    }

    Ok(pooled)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Unit tests (no model download needed) ----

    #[test]
    fn test_l2_normalize() {
        let mut vec = vec![3.0, 4.0];
        l2_normalize(&mut vec);
        assert!((vec[0] - 0.6).abs() < 1e-6);
        assert!((vec[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let mut vec = vec![0.0, 0.0, 0.0];
        l2_normalize(&mut vec);
        assert_eq!(vec, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_l2_normalize_already_unit() {
        let mut vec = vec![1.0, 0.0, 0.0];
        l2_normalize(&mut vec);
        assert!((vec[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_mean_pool_basic() {
        let hidden = ndarray::array![[[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0]]];
        let mask = ndarray::array![[1i64, 1]];
        let result = mean_pool(hidden.view(), &mask);
        assert_eq!(result.len(), 1);
        assert!((result[0][0] - 2.5).abs() < 1e-6);
        assert!((result[0][1] - 3.5).abs() < 1e-6);
        assert!((result[0][2] - 4.5).abs() < 1e-6);
    }

    #[test]
    fn test_mean_pool_with_padding() {
        let hidden = ndarray::array![[[1.0f32, 2.0, 3.0], [99.0, 99.0, 99.0]]];
        let mask = ndarray::array![[1i64, 0]];
        let result = mean_pool(hidden.view(), &mask);
        assert!((result[0][0] - 1.0).abs() < 1e-6);
        assert!((result[0][1] - 2.0).abs() < 1e-6);
        assert!((result[0][2] - 3.0).abs() < 1e-6);
    }

    // ---- Integration tests (require model — may be slow on first run) ----

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_embed_text_dimension() {
        let vec = embed_text("hello world").unwrap();
        assert_eq!(vec.len(), EMBEDDING_DIM);
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_embed_text_is_normalized() {
        let vec = embed_text("hello world").unwrap();
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "norm was {norm}");
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_embed_texts_batch() {
        let vecs = embed_texts(&["hello", "world"]).unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);
        assert_eq!(vecs[1].len(), EMBEDDING_DIM);
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn test_similar_texts_higher_similarity() {
        let v1 = embed_text("rust programming language").unwrap();
        let v2 = embed_text("rust cargo build system").unwrap();
        let v3 = embed_text("chocolate cake recipe").unwrap();

        let sim_related: f32 = v1.iter().zip(&v2).map(|(a, b)| a * b).sum();
        let sim_unrelated: f32 = v1.iter().zip(&v3).map(|(a, b)| a * b).sum();
        assert!(sim_related > sim_unrelated);
    }
}
