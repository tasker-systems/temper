//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub mod entities;
pub mod errors;
pub mod payloads;
pub mod types;

pub use entities::{create_entity, discard_profile, move_entity};
pub use errors::LedgerError;
pub use payloads::{ConceptCreatedPayload, ConceptMutatedPayload};
pub use types::{
    Concept, Entity, Event, EventReference, EventToWrite, EventType, Porosity, Profile,
    ReferenceKind, Scope, Topic,
};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
