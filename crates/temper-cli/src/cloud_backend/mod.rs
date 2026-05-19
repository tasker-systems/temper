//! `CloudBackend` — cloud-mode impl of [`temper_core::operations::Backend`].
//!
//! Per-request construction: CLI action commands build a `CloudBackend` from
//! a `TemperClient`, owner, and config, then dispatch one command through it.
//! Each trait method translates the command into a `temper_client` API call
//! and synthesizes the appropriate `DomainEvent`s.
//!
//! Unlike `vault_backend`, cloud mode has no offline path — if no token
//! resolves, `assemble_cloud_backend` errors immediately.
//!
//! See `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`.

mod cloud_backend;
pub mod ctx;
mod translators;

#[cfg(test)]
mod tests;

pub use cloud_backend::CloudBackend;
pub use ctx::{assemble_cloud_backend, CloudBackendCtx};
