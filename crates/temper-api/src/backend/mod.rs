//! `DbBackend` — Postgres-backed impl of [`temper_core::operations::Backend`].
//!
//! Per-request construction: handlers (3b) and MCP tools (3c) build a
//! `DbBackend` from their auth context and dispatch one command through it.
//! Each trait method is a thin translator over an existing service function;
//! events are synthesized post-hoc on success.
//!
//! See `docs/superpowers/specs/2026-05-07-wave1-phase3-dbbackend-design.md`.

mod db_backend;
pub mod selection;
mod translators;

#[cfg(all(test, feature = "test-db"))]
mod tests;

pub use db_backend::DbBackend;
pub use selection::{require_legacy_backend, select_backend, BackendSelection};
