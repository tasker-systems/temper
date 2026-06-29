//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally or wrapped by temper-cloud for Vercel.
//! Use [`routes::create_app`] to get the composable Router.

pub mod handlers;
pub mod middleware;
pub mod openapi;
pub mod routes;

// Transitional re-exports (temper-services extraction, goal 019f149b):
// `config`/`error`/`state` (Chunk 1), `services` (Chunk 2) and `backend` (Chunk 3) now live in
// temper-services. temper-api's own code references them as `temper_services::*`; these re-exports
// keep the not-yet-migrated external consumers (temper-mcp, the e2e suite, the api/ Vercel adapters)
// compiling against `temper_api::{backend,config,error,services,state}`. Removed in Chunk 4 when
// temper-mcp repoints directly.
pub use temper_services::{backend, config, error, services, state};

pub use routes::create_app;

/// Migrator used by `#[sqlx::test]` to create isolated per-test databases.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
