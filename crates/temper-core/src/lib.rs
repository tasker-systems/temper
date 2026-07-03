//! temper-core — shared types, traits, and models for the temper knowledge base system.
//!
//! This crate is the vocabulary shared by all temper crates: temper-cli, temper-api,
//! temper-client, temper-cloud, temper-ingest, and temper-mcp. It contains domain types,
//! error definitions, and ID generation utilities.

pub mod charter;
pub mod context_ref;
pub mod error;
pub mod hash;
pub mod ids;
pub mod internal_sig;
pub mod projection;
pub mod types;
pub mod validation;
