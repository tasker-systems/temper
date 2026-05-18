//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod errors;
pub mod types;

pub use errors::LedgerError;
pub use types::{Entity, Profile};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
