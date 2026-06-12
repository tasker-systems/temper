//! `temper-next` — deterministic, declared-only telos-lens region producer.
//!
//! Production-quality clustering core (`affinity`, `cluster`) written to lift wholesale into
//! `temper-cogmap` later (spec §6b). Cosine never enters region *formation* — it appears only as
//! a downstream SQL readout (Plan 1 functions).
pub mod affinity;
pub mod cluster;
pub mod content;
pub mod embed;
pub mod events;
pub mod fingerprint;
pub mod ids;
pub mod payloads;
pub mod replay;
pub mod scenario;
pub mod substrate;
pub mod write;
