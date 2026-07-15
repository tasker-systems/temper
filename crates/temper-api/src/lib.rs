//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally or wrapped by temper-cloud for Vercel.
//! Use [`routes::create_app`] to get the composable Router, or
//! [`routes::create_internal_app`] for the internal/system-only surface.

pub mod handlers;
pub mod middleware;
pub mod openapi;
pub mod routes;

pub use routes::{create_app, create_internal_app, openapi_spec};

/// Migrator used by `#[sqlx::test]` to create isolated per-test databases.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
