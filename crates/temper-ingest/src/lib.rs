//! temper-ingest — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime

pub mod chunk;
pub mod error;
pub mod extract;
pub mod merge;

#[cfg(feature = "embed")]
pub mod embed;

#[cfg(feature = "embed")]
pub mod pipeline;
