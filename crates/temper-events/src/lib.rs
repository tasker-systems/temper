//! Event-sourced substrate foundations.
//!
//! See `docs/superpowers/specs/2026-05-18-event-substrate-foundations-design.md`.

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
