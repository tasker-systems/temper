//! Deployment-profile policy object: how and where an agent binding is deployed.
//!
//! Read by the runtime-binding layer; the substrate never reads it
//! (WS7 decision #3 — the kernel never branches on stratum).

use serde::{Deserialize, Serialize};

/// Which agent runtime this deployment binds to. (WS7 decision #1.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "deployment_profile.ts")
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBinding {
    /// Vercel Eve durable agents.
    Eve,
    /// Claude Managed Agents (`/v1/agents` + `/v1/sessions`).
    ClaudeManaged,
}

/// Where tool execution runs. Orthogonal to [`RuntimeBinding`] — both runtimes
/// offer both (Eve: Vercel-managed vs docker/self-deploy; CMA: cloud vs
/// self-hosted). This is WS7's "stratum" made concrete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "deployment_profile.ts")
)]
#[serde(rename_all = "snake_case")]
pub enum Residency {
    /// Runtime-operator-hosted sandbox (Vercel sandbox / CMA cloud env).
    Managed,
    /// Customer-infrastructure execution (Eve docker/self-deploy / CMA self-hosted worker).
    SelfHosted,
}

/// How an agent binding is deployed and paced. Carried by the binding layer;
/// never read by the substrate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "deployment_profile.ts")
)]
pub struct DeploymentProfile {
    pub runtime: RuntimeBinding,
    pub residency: Residency,
    /// Token-denominated budget — neither runtime exposes a managed dollar
    /// budget, so spend governance is expressed in tokens. `None` = runtime default.
    pub token_budget: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_profile_round_trips_snake_case() {
        let profile = DeploymentProfile {
            runtime: RuntimeBinding::ClaudeManaged,
            residency: Residency::SelfHosted,
            token_budget: Some(50_000),
        };
        let value = serde_json::to_value(&profile).unwrap();
        assert_eq!(value["runtime"], "claude_managed");
        assert_eq!(value["residency"], "self_hosted");
        assert_eq!(value["token_budget"], 50_000);

        let back: DeploymentProfile = serde_json::from_value(value).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn absent_token_budget_serializes_as_null() {
        let profile = DeploymentProfile {
            runtime: RuntimeBinding::Eve,
            residency: Residency::Managed,
            token_budget: None,
        };
        let value = serde_json::to_value(&profile).unwrap();
        assert_eq!(value["token_budget"], serde_json::Value::Null);
    }
}
