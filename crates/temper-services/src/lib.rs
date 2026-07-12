//! temper-services — the shared business-logic + auth-infra layer reused by both
//! surfaces (temper-api HTTP server, temper-mcp MCP server).
//!
//! Extracted from temper-api so neither surface depends on the other's transport
//! crate. Chunk 1 seeds it with the leaf types: the service error vocabulary
//! ([`error::ApiError`]) and the auth/transport infra ([`state::AppState`],
//! [`state::JwksKeyStore`], [`config::ApiConfig`]). The backend + services move in
//! later chunks. See goal `019f149b`.

pub mod auth;
pub mod auth_config;
pub mod backend;
pub mod config;
pub mod error;
pub mod services;
pub mod state;

/// Embedded workspace migrations, for `#[sqlx::test(migrator = "temper_services::MIGRATOR")]`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
