//! temper-ingest — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime (bundled model)
//! - `embed-download`: same embedding, but downloads model from Hugging Face Hub at runtime

pub mod chunk;
pub mod error;
pub mod extract;
pub mod merge;

#[cfg(any(feature = "embed", feature = "embed-download"))]
pub mod embed;

#[cfg(any(feature = "embed", feature = "embed-download"))]
pub mod pipeline;
