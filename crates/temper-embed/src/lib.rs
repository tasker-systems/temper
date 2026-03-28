//! temper-embed — Embedding and extraction pipeline.
//!
//! Separate binary with kreuzberg/ONNX for chunking, embedding, and document
//! extraction. Runs as a background worker processing uploads from Cloudflare R2.
//! Heavy dependencies (kreuzberg, ONNX runtime) are isolated here.
