//! Event-sourced ledger: append-only, scoped, registry-backed.
//!
//! Limb 0 of the event-primary reorientation. See
//! `docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md`.

pub mod errors;
pub mod ledger;
pub mod types;

pub use errors::LedgerError;
pub use ledger::{append_event, append_event_tx};
pub use types::{
    Event, EventReference, EventToWrite, EventType, Porosity, ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
