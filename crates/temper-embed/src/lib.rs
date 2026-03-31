//! temper-embed — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime (added in Task 6)

pub mod error;
pub mod extract;
