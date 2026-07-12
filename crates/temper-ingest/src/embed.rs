//! Text embedding using BAAI/bge-base-en-v1.5 via ONNX Runtime.
//!
//! Two model-loading strategies, selected at compile time by feature flags:
//!
//! - **`embed`** (default): model bundled via `include_bytes!()` at compile
//!   time (no runtime downloads).  Requires git-lfs checkout.
//! - **`embed-download`**: model downloaded at runtime from Hugging Face via
//!   hf-hub.  Safe on machines without git-lfs; used by the CLI.
//!
//! ONNX session created once per process via `OnceLock`.
//!
//! ORT runtime loading is platform-aware:
//! - **Linux** (Vercel deploy): the bundled `libonnxruntime.so` is written to
//!   `/tmp` and loaded via `ort::init_from`.
//! - **Other platforms** (macOS dev): ORT is loaded from the system library
//!   path.  Install via `brew install onnxruntime` and set `ORT_DYLIB_PATH`
//!   if needed.
//!
//! Pipeline: tokenize -> build tensors -> inference -> mean pool -> normalize

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use ndarray::{Array2, ArrayView3};
use ort::session::Session;
use ort::value::TensorRef;
use tokenizers::{Encoding, Tokenizer};

use crate::error::{EmbedError, Result};

/// Embedding dimension for bge-base-en-v1.5.
pub const EMBEDDING_DIM: usize = 768;

#[cfg(all(feature = "embed", not(feature = "embed-download")))]
static MODEL_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/model_quantized.onnx");
static TOKENIZER_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/tokenizer.json");

/// Bundled Linux x86_64 libonnxruntime.so — only compiled into the binary on
/// Linux embed targets (Vercel deploy).  On other platforms this is unused
/// and the system-installed ORT is loaded instead.
#[cfg(all(
    target_os = "linux",
    feature = "embed",
    not(feature = "embed-download")
))]
static ORT_LIB_BYTES: &[u8] = include_bytes!("../lib/x86_64-unknown-linux-gnu/libonnxruntime.so");

// ---- ORT runtime initialization ----

// The three helpers below are consumed only by the "load-dynamic" init branch
// further down and by unit tests. On the Vercel/Linux path (embed without
// embed-download) only the bundled-.so branch compiles, leaving them dead —
// hence the explicit allow. The tests always exercise them via cfg(test).

#[allow(dead_code)]
fn resolve_dylib_from_candidates(candidates: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

/// Covers the two archive layouts the release installer produces:
///   - mac/linux:  <exe_dir>/lib/libonnxruntime.{dylib,so}
///   - windows:    <exe_dir>/onnxruntime.dll (flat)
#[allow(dead_code)]
fn binary_adjacent_candidates(exe_path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(exe_dir) = exe_path.parent() else {
        return Vec::new();
    };
    vec![
        exe_dir.join("lib").join("libonnxruntime.dylib"),
        exe_dir.join("lib").join("libonnxruntime.so"),
        exe_dir.join("onnxruntime.dll"),
    ]
}

/// Linux fallback for when `temper` is symlinked onto PATH while the actual
/// install lives at `~/.local/share/temper/`. On macOS, `dirs::data_local_dir()`
/// resolves to `~/Library/Application Support/` (platform-idiomatic) — but the
/// binary-adjacent candidates fire first there, so this fallback is effectively
/// Linux-only in practice.
#[allow(dead_code)]
fn xdg_data_candidates() -> Vec<std::path::PathBuf> {
    let Some(data_dir) = dirs::data_local_dir() else {
        return Vec::new();
    };
    let lib_dir = data_dir.join("temper").join("lib");
    vec![
        lib_dir.join("libonnxruntime.dylib"),
        lib_dir.join("libonnxruntime.so"),
    ]
}

static ORT_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Initialize the ORT runtime.
///
/// On Linux: write the bundled `.so` to `/tmp` and load it explicitly.
/// On other platforms: let ORT's `load-dynamic` search system paths
/// (`ORT_DYLIB_PATH`, Homebrew, etc.).
fn init_ort_runtime() -> std::result::Result<(), String> {
    ORT_INIT.get_or_init(|| {
        // Bundled Linux deploy: write the bundled .so to /tmp and load it.
        #[cfg(all(
            target_os = "linux",
            feature = "embed",
            not(feature = "embed-download")
        ))]
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

        // All other cases (macOS, Linux with embed-download): search system paths.
        #[cfg(not(all(
            target_os = "linux",
            feature = "embed",
            not(feature = "embed-download")
        )))]
        {
            // Search order:
            //   1. ORT_DYLIB_PATH env var (explicit override)
            //   2. Binary-adjacent: <exe_dir>/lib/libonnxruntime.{dylib,so} OR
            //      <exe_dir>/onnxruntime.dll (installer-bundled layout)
            //   3. XDG data: ~/.local/share/temper/lib/libonnxruntime.{dylib,so}
            //   4. Homebrew ARM64: /opt/homebrew/lib/libonnxruntime.dylib
            //   5. Homebrew Intel: /usr/local/lib/libonnxruntime.dylib
            //   6. Linux system: /usr/lib/libonnxruntime.so
            let dylib_path = std::env::var("ORT_DYLIB_PATH")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::current_exe().ok().and_then(|exe| {
                        resolve_dylib_from_candidates(&binary_adjacent_candidates(&exe))
                    })
                })
                .or_else(|| resolve_dylib_from_candidates(&xdg_data_candidates()))
                .or_else(|| {
                    resolve_dylib_from_candidates(&[
                        std::path::PathBuf::from("/opt/homebrew/lib/libonnxruntime.dylib"),
                        std::path::PathBuf::from("/usr/local/lib/libonnxruntime.dylib"),
                        std::path::PathBuf::from("/usr/lib/libonnxruntime.so"),
                    ])
                })
                .map(|p| p.to_string_lossy().into_owned());

            if let Some(path) = dylib_path {
                ort::init_from(path)
                    .map_err(|e| format!("ort::init_from: {e}"))?
                    .commit();
            } else {
                return Err(
                    "ONNX Runtime not found. Install via `brew install onnxruntime` \
                     or set ORT_DYLIB_PATH to the library location."
                        .to_owned(),
                );
            }
        }

        Ok(())
    });
    match ORT_INIT.get() {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => Err(e.clone()),
        None => Err("ORT_INIT not initialized".to_owned()),
    }
}

// ---- ONNX intra-op thread count ----
//
// ORT parallelizes a single inference across an internal "intra-op" thread
// pool. `0` lets ORT size that pool to all cores; `N` pins it to N. This is
// the dominant throughput lever for the embed path: the whole chunk batch is
// one ORT run (see pipeline.rs), so the intra-op pool *is* the parallelism.
// Measured on a 10-core box (1.2 MB body ⇒ ~840 chunks): pinning to 1 core
// took ~473s; all cores took ~155s — 3.1× on this one knob (task 019f57d2).
//
// The right count differs by surface. The CLI is one user embedding one
// document, so all cores is correct. The server (temper-api) may run N
// concurrent ingests, where "every embed grabs every core" risks
// oversubscription — its ideal count is an open question pending a
// measurement under concurrent load, so the server does NOT opt in here and
// inherits the conservative pinned default below.

/// Env override for the ONNX intra-op thread count (`TEMPER_ONNX_INTRA_THREADS`).
/// `0` = all cores, `N` = pin to N. Wins over a surface's programmatic default
/// so a Vercel deploy (or a CLI on a shared box) can be tuned without a rebuild.
const INTRA_THREADS_ENV: &str = "TEMPER_ONNX_INTRA_THREADS";

/// Sentinel for "no surface set an explicit count"; resolution then falls
/// through to the env var and the built-in default.
const INTRA_THREADS_UNSET: usize = usize::MAX;

/// Built-in default when nothing else is set: pin to a single core. Preserves
/// the historical behavior for any process that never opts in (the server), so
/// this plumbing changes no threading on its own — a surface or operator must
/// ask for more.
const INTRA_THREADS_DEFAULT: usize = 1;

/// Process-global intra-op thread count, set once by a surface before the
/// first embed. `INTRA_THREADS_UNSET` until then.
static INTRA_OP_THREADS: AtomicUsize = AtomicUsize::new(INTRA_THREADS_UNSET);

/// Declare this process's default ONNX intra-op thread count.
///
/// `0` lets ORT parallelize inference across all cores; `N` pins it to N. Call
/// once at startup **before** the first embed — the count is read when the ORT
/// session is lazily built and ignored afterward. The `TEMPER_ONNX_INTRA_THREADS`
/// env var overrides whatever a surface sets here.
///
/// The CLI calls this with `0` (one user, one document ⇒ all cores). The server
/// leaves it unset and inherits the conservative single-core default, pending a
/// measurement of concurrent-ingest oversubscription (task 019f57d2).
pub fn set_intra_op_threads(threads: usize) {
    INTRA_OP_THREADS.store(threads, Ordering::Relaxed);
}

/// Resolve the intra-op thread count for the ORT session.
///
/// Precedence: `TEMPER_ONNX_INTRA_THREADS` env → [`set_intra_op_threads`] →
/// `INTRA_THREADS_DEFAULT`. A malformed env value is ignored (falls through)
/// rather than silently selecting a surprising count.
fn resolve_intra_op_threads() -> usize {
    if let Ok(raw) = std::env::var(INTRA_THREADS_ENV) {
        if let Ok(n) = raw.trim().parse::<usize>() {
            return n;
        }
    }
    match INTRA_OP_THREADS.load(Ordering::Relaxed) {
        INTRA_THREADS_UNSET => INTRA_THREADS_DEFAULT,
        n => n,
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

        let session = build_session()?;

        let tokenizer = load_tokenizer()?;

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

/// Build ORT session from bundled model bytes.
#[cfg(all(feature = "embed", not(feature = "embed-download")))]
fn build_session() -> std::result::Result<Session, String> {
    Session::builder()
        .map_err(|e| format!("ort session builder: {e}"))?
        .with_intra_threads(resolve_intra_op_threads())
        .map_err(|e| format!("ort threads: {e}"))?
        .commit_from_memory(MODEL_BYTES)
        .map_err(|e| format!("ort load: {e}"))
}

/// Build ORT session by downloading model from Hugging Face Hub.
#[cfg(feature = "embed-download")]
fn build_session() -> std::result::Result<Session, String> {
    let api = hf_hub::api::sync::Api::new().map_err(|e| format!("hf-hub init: {e}"))?;
    let repo = api.model("BAAI/bge-base-en-v1.5".to_owned());
    let model_path = repo
        .get("onnx/model.onnx")
        .map_err(|e| format!("download model: {e}"))?;

    Session::builder()
        .map_err(|e| format!("ort session builder: {e}"))?
        .with_intra_threads(resolve_intra_op_threads())
        .map_err(|e| format!("ort threads: {e}"))?
        .commit_from_file(&model_path)
        .map_err(|e| format!("ort load: {e}"))
}

// ---- Tokenization ----

/// Load the bundled tokenizer with truncation configured to the model's
/// token limit, so `encode_batch` bounds its own work per input instead of
/// relying on the downstream [`truncate_encoding`] trim.
fn load_tokenizer() -> std::result::Result<Tokenizer, String> {
    let mut tokenizer =
        Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| format!("load tokenizer: {e}"))?;
    tokenizer
        .with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_MODEL_TOKENS,
            ..Default::default()
        }))
        .map_err(|e| format!("tokenizer truncation config: {e}"))?;
    Ok(tokenizer)
}

/// Tokenize a batch of texts using the model's tokenizer.
pub fn tokenize(tokenizer: &Tokenizer, texts: &[&str]) -> Result<Vec<Encoding>> {
    tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|e| EmbedError::Embedding(format!("tokenize: {e}")))
}

// ---- Tensor construction ----

/// Input tensors for the ONNX model.
#[derive(Debug)]
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

    // ---- Feature guard tests ----

    #[test]
    #[cfg(all(feature = "embed", not(feature = "embed-download")))]
    fn bundled_model_bytes_are_not_lfs_pointer() {
        let header = &super::MODEL_BYTES[..std::cmp::min(30, super::MODEL_BYTES.len())];
        let header_str = String::from_utf8_lossy(header);
        assert!(
            !header_str.starts_with("version https://git-lfs"),
            "MODEL_BYTES contains a git-lfs pointer, not the actual model binary. \
             Run `git lfs pull` or use the `embed-download` feature instead."
        );
    }

    // ---- Unit tests (no model download needed) ----

    #[test]
    fn intra_op_threads_precedence() {
        // Owns the whole precedence sequence in one test so the process-global
        // env var + atomic are never raced by a sibling test. Saves/restores
        // both so it leaves no residue.
        let saved_env = std::env::var(super::INTRA_THREADS_ENV).ok();
        let saved_atomic = super::INTRA_OP_THREADS.load(Ordering::Relaxed);

        // Unset atomic + no env → built-in default (historical pinned behavior).
        std::env::remove_var(super::INTRA_THREADS_ENV);
        super::INTRA_OP_THREADS.store(super::INTRA_THREADS_UNSET, Ordering::Relaxed);
        assert_eq!(resolve_intra_op_threads(), super::INTRA_THREADS_DEFAULT);

        // A surface's programmatic default is honored when no env is set.
        set_intra_op_threads(0);
        assert_eq!(resolve_intra_op_threads(), 0);
        set_intra_op_threads(3);
        assert_eq!(resolve_intra_op_threads(), 3);

        // Env wins over the surface default.
        std::env::set_var(super::INTRA_THREADS_ENV, "2");
        assert_eq!(resolve_intra_op_threads(), 2);

        // A malformed env value is ignored, falling back to the surface default.
        std::env::set_var(super::INTRA_THREADS_ENV, "not-a-number");
        assert_eq!(resolve_intra_op_threads(), 3);

        match saved_env {
            Some(v) => std::env::set_var(super::INTRA_THREADS_ENV, v),
            None => std::env::remove_var(super::INTRA_THREADS_ENV),
        }
        super::INTRA_OP_THREADS.store(saved_atomic, Ordering::Relaxed);
    }

    #[test]
    fn test_l2_normalize_scales_to_unit_length() {
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
    fn test_mean_pool_averages_unmasked_positions() {
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

    #[test]
    fn tokenizer_truncates_oversized_input_at_encode_time() {
        // Truncation must be configured on the tokenizer itself so
        // encode_batch bounds its own work per input — not left to the
        // downstream truncate_encoding trim (issue #316).
        let tokenizer = load_tokenizer().expect("load tokenizer");
        let huge = "word ".repeat(50_000); // ~250k chars, far over 512 tokens
        let encodings = tokenize(&tokenizer, &[huge.as_str()]).expect("tokenize");
        assert!(
            encodings[0].get_ids().len() <= MAX_MODEL_TOKENS,
            "expected encoding truncated to {} tokens, got {}",
            MAX_MODEL_TOKENS,
            encodings[0].get_ids().len()
        );
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

    // -- Dylib discovery fallback chain --
    //
    // These tests exercise the pure helpers extracted from the ORT init logic.
    // They don't actually load ONNX — just verify path selection against a
    // constructed set of candidate paths.

    use std::fs;

    fn make_dummy_lib(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"not a real library").expect("write dummy lib");
        path
    }

    #[test]
    fn resolve_picks_first_existing_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let first = make_dummy_lib(tmp.path(), "first.dylib");
        let second = make_dummy_lib(tmp.path(), "second.dylib");

        let picked = super::resolve_dylib_from_candidates(&[first.clone(), second.clone()]);

        assert_eq!(picked, Some(first));
    }

    #[test]
    fn resolve_skips_missing_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist.dylib");
        let exists = make_dummy_lib(tmp.path(), "real.dylib");

        let picked = super::resolve_dylib_from_candidates(&[missing, exists.clone()]);

        assert_eq!(picked, Some(exists));
    }

    #[test]
    fn resolve_returns_none_when_all_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let candidates = vec![tmp.path().join("a.dylib"), tmp.path().join("b.dylib")];

        let picked = super::resolve_dylib_from_candidates(&candidates);

        assert_eq!(picked, None);
    }

    #[test]
    fn binary_adjacent_candidates_include_lib_subdir_and_flat() {
        // Given an exe at /opt/tool/bin/temper, candidates should include both
        // /opt/tool/bin/lib/libonnxruntime.{dylib,so} (installed-tree layout)
        // and /opt/tool/bin/onnxruntime.dll (Windows flat layout).
        let fake_exe = std::path::PathBuf::from("/opt/tool/bin/temper");
        let candidates = super::binary_adjacent_candidates(&fake_exe);

        let as_str: Vec<String> = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        assert!(
            as_str
                .iter()
                .any(|s| s.ends_with("lib/libonnxruntime.dylib")),
            "missing lib/libonnxruntime.dylib: {as_str:?}"
        );
        assert!(
            as_str.iter().any(|s| s.ends_with("lib/libonnxruntime.so")),
            "missing lib/libonnxruntime.so: {as_str:?}"
        );
        assert!(
            as_str.iter().any(|s| s.ends_with("onnxruntime.dll")),
            "missing onnxruntime.dll: {as_str:?}"
        );
    }

    #[test]
    fn xdg_data_candidates_point_at_temper_lib() {
        let candidates = super::xdg_data_candidates();
        let as_str: Vec<String> = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(
            as_str.iter().any(|s| s.contains("/temper/lib/")),
            "candidates should include ~/.local/share/temper/lib/: {as_str:?}"
        );
    }
}
