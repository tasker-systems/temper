//! Synthesis-from-state (WS6 §0): regenerate the `temper_next` substrate from current production
//! (`public.*`) projected state by firing genesis events, NOT by replaying the old (incomplete)
//! ledger. This module is the explicitly-invoked operation behind the `temper-next synthesize`
//! subcommand — never a migrate-time side effect (§D).
//!
//! Synthesis covers **active state only** (§0): soft-deleted resources are not synthesized. The
//! per-resource sequence (filled in across the WS6 chunk-2 tasks) is: `resource_created` (with
//! block/chunk manifests per §8) → `property_asserted` per surviving manifest key (§7) →
//! `relationship_asserted` per edge (§4); folded rows synthesize as assert+fold event pairs.
//!
//! This file currently carries the scaffolding: [`run`] is a stub returning an empty [`SynthReport`];
//! the typed `public.*` reads live in [`source`].

pub mod bootstrap;
pub mod source;

use anyhow::Result;
use sqlx::PgPool;

/// Knobs for a synthesis run.
#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    /// Stop after N resources (rehearsal); `0` = all.
    pub limit: usize,
}

/// Counts produced by a synthesis run. Later tasks extend this as each pass lands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SynthReport {
    /// Resources synthesized (`resource_created` fired).
    pub resources: usize,
    /// Properties synthesized (`property_asserted` fired).
    pub properties: usize,
    /// Edges synthesized (`relationship_asserted` fired).
    pub edges: usize,
}

/// Synthesize the `temper_next` substrate from current `public.*` state.
///
/// Stub for now (WS6 chunk-2 Task 4 scaffolding): the bootstrap / resource / property / edge passes
/// land in the following tasks. Returns an empty report.
pub async fn run(_pool: &PgPool, _opts: RunOpts) -> Result<SynthReport> {
    Ok(SynthReport::default())
}
