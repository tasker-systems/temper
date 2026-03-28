//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally via `cargo run` or wrapped by temper-cloud
//! for Vercel deployment. Exports `create_app(state) -> Router` for composition.

pub mod config;
pub mod error;
pub mod state;
