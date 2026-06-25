//! `temper-agents` — the runtime-neutral contract for temper's agent surface.
//!
//! A deliberately thin layer (WS7 decision #6): owns the
//! [`profile::DeploymentProfile`] policy object and re-exports the
//! invocation-envelope + agent-authorship contract from `temper-substrate`
//! ([`envelope`]). See the design spec under
//! `docs/superpowers/specs/2026-06-18-temper-agents-neutral-contract-crate-design.md`.

pub mod envelope;
pub mod profile;
