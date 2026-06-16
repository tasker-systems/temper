//! `temper-next` — deterministic, declared-only telos-lens region producer.
//!
//! Production-quality clustering core (`affinity`, `cluster`) written to lift wholesale into
//! `temper-cogmap` later (spec §6b). Cosine never enters region *formation* — it appears only as
//! a downstream SQL readout (Plan 1 functions).
pub mod affinity;
pub mod cluster;
pub mod content;
pub mod drift;
pub mod embed;
pub mod events;
pub mod fingerprint;
pub mod ids;
pub mod payloads;
pub mod readback;
pub mod replay;
pub mod scenario;
pub mod substrate;
pub mod synthesis;
pub mod write;
pub mod writes;

/// The shared sqlx migration chain (workspace `migrations/`). Exposed so synthesis + parity
/// integration tests can spin up an isolated ephemeral DB via `#[sqlx::test(migrator = ...)]` with
/// the full chain applied — including `20260613000001_install_temper_next.sql`, which installs the
/// `temper_next` namespace alongside an empty migrated `public`. (The scenario write-path tests use
/// the shared dev DB + psql `reset_artifact` instead; this is only for the self-contained tests.)
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
