//! `temper-agents` — the runtime-neutral contract for temper's agent surface.
//!
//! A deliberately thin layer (WS7 decision #6). Owns the
//! [`profile::DeploymentProfile`] policy object. See the design spec under
//! `docs/superpowers/specs/2026-06-18-temper-agents-neutral-contract-crate-design.md`.

pub mod profile;
