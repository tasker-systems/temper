//! `VaultBackend` — vault-file impl of [`temper_core::operations::Backend`].
//!
//! Per-request construction: CLI action commands build a `VaultBackend` from
//! vault root, manifest, client, and owner, then dispatch one command through it.
//! Each trait method handles vault-file persistence (read/write/delete) and
//! optional push to cloud via the client. Events are synthesized at each step.
//!
//! See `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`.

mod per_doctype;
mod translators;
mod vault_backend;

#[cfg(all(test, feature = "test-db"))]
mod tests;

pub use vault_backend::{VaultBackend, VaultBackendCtx};
