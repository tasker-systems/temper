//! `CloudBackend` — cloud-mode impl of [`temper_workflow::operations::Backend`].
//!
//! Per-request construction: CLI action commands build a `CloudBackend` from
//! a `TemperClient`, owner, and config, then dispatch one command through it.
//! Each trait method translates the command into a `temper_client` API call
//! and synthesizes the appropriate `DomainEvent`s.
//!
//! Cloud mode has no offline path — if no token resolves,
//! `assemble_cloud_backend` errors immediately.
//!
//! See `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`.

mod backend;
pub mod ctx;
// `pub(crate)` (not private): `actions::ingest::run_segmented_create` (Beat 3) reuses
// `translators::cmd_to_segmented_begin_payload` to build segment 0's wire payload with the
// same home/managed_meta/open_meta/goal/act mapping `cmd_to_ingest_payload` uses, without
// duplicating that logic outside `cloud_backend`.
pub(crate) mod translators;

#[cfg(test)]
mod tests;

pub use backend::CloudBackend;
pub use ctx::{assemble_cloud_backend, CloudBackendCtx};
