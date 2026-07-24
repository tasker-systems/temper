//! `DbBackend` — Postgres-backed impl of [`temper_workflow::operations::Backend`] over the substrate.
//!
//! Per-request construction: handlers and MCP tools build a `DbBackend` from their auth context and
//! dispatch one command through it. Reads go through the [`substrate_read`] dispatcher (the substrate
//! read path); writes compose `temper_substrate::writes` and fire through the event ledger.

mod db_backend;
pub mod region_clocks;
pub mod substrate_read;

pub use db_backend::{DbBackend, ACT_SPAN_FIELDS};
