//! `temper-substrate` — deterministic, telos-lens region producer over two regimes.
//!
//! Production-quality clustering core (`affinity`, `cluster`) written to lift wholesale into
//! `temper-cogmap` later (spec §6b).
//!
//! ONE producer, TWO regimes, selected entirely by the lens's `w_cos` (spec §3.1):
//!   - `w_cos == 0` — the **cogmap** regime. The declared graph (edges + facets) is the whole signal;
//!     cosine never enters region *formation* and appears only as a downstream SQL readout.
//!   - `w_cos > 0` — the **context** regime. A sparse exact-kNN cosine term ([`knn`]) joins the
//!     kernel, and the embedding becomes the PRIMARY evidence of regionality — which is what lets a
//!     context, carrying no facets and almost no declared edges, form regions at all.
pub mod affinity;
pub mod cluster;
pub mod content;
pub mod drift;
pub mod embed;
pub mod events;
pub mod fingerprint;
pub mod ids;
pub mod keys;
pub mod knn;
pub mod payloads;
pub mod readback;
pub mod replay;
pub mod scenario;
pub mod substrate;
pub mod text;
pub mod write;
pub mod writes;

/// The shared sqlx migration chain (workspace `migrations/`). Exposed so self-contained integration
/// tests can spin up an isolated ephemeral DB via `#[sqlx::test(migrator = ...)]` with the full chain
/// applied. (The scenario write-path tests use the shared dev DB + psql `reset_artifact` instead; this
/// is only for the self-contained tests.)
///
/// Declared once, in temper-migrate — a crate light enough for the deploy's build command to
/// compile without dragging ort in behind it. Re-exported here so every existing
/// `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` keeps resolving.
pub use temper_migrate::MIGRATOR;

/// The ledger-recording migration runner, re-exported from temper-migrate under the name it had
/// while it lived here (`temper_substrate::migrate_ledger`).
pub use temper_migrate::ledger as migrate_ledger;
