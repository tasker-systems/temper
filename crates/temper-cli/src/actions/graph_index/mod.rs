//! Graph index module — LLM-assisted concept discovery pipeline.
//!
//! Phases:
//! 1. Seed extraction (TF-IDF) — see `seeds.rs`
//! 2. Cluster formation (HNSW) — see `cluster.rs`
//! 3. LLM judgment — see `judgment.rs`
//! 4. Materialization — see `materialize.rs`

pub mod seeds;
pub mod cluster;
pub mod judgment;
pub mod materialize;