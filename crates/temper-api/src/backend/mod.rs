//! `DbBackend` — Postgres-backed impl of [`temper_core::operations::Backend`] over the substrate.
//!
//! Per-request construction: handlers and MCP tools build a `DbBackend` from their auth context and
//! dispatch one command through it. Reads go through the [`read_selector`] dispatcher (the substrate
//! read path); writes compose `temper_next::writes` and fire through the event ledger.

mod db_backend;
pub mod read_selector;

pub use db_backend::DbBackend;
