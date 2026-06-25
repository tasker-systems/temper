//! The runtime-neutral agent accountability contract.
//!
//! Re-exports the invocation-envelope + agent-authorship data types that
//! currently live in `temper-substrate` (shipped in PR #148). This crate is a thin
//! *consumer* of those types, never their owner — the definitional home stays
//! `temper-substrate` now and moves to `temper-core` at the convergence lift, at
//! which point only the `pub use` paths below change.
//!
//! Data types only: the substrate-side write helpers (`EventContext`,
//! `fire_with`) are deliberately NOT re-exported — a remote (Claude-managed)
//! binding reaches the substrate over MCP and cannot use sqlx-bound helpers.

pub use temper_substrate::ids::InvocationId;
pub use temper_substrate::payloads::{
    AgentAuthorship, ConfidenceBand, DelegatedLaunch, Disposition, InvocationClosed,
};

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-only guard: the re-export path resolves for every contract type.
    #[expect(
        dead_code,
        reason = "compile-only guard that the re-export paths resolve"
    )]
    fn contract_resolves(
        _id: InvocationId,
        _launch: DelegatedLaunch,
        _closed: InvocationClosed,
        _authorship: AgentAuthorship,
        _band: ConfidenceBand,
    ) {
    }

    #[test]
    fn disposition_round_trips_via_reexport() {
        let value = serde_json::to_value(Disposition::Completed).unwrap();
        let back: Disposition = serde_json::from_value(value).unwrap();
        assert!(matches!(back, Disposition::Completed));
    }
}
