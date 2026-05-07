//! temper-api — Axum HTTP server implementing the temper cloud API.
//!
//! Platform-agnostic: runs locally or wrapped by temper-cloud for Vercel.
//! Use [`routes::create_app`] to get the composable Router.

pub mod backend;
pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod openapi;
pub mod routes;
pub mod services;
pub mod state;

pub use routes::create_app;

/// Migrator used by `#[sqlx::test]` to create isolated per-test databases.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
