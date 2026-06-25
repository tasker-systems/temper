//! Cross-surface invocation types. `Disposition` mirrors
//! `temper_substrate::payloads::Disposition`; `NextBackend` maps between them
//! (the `map_edge_kind` pattern) since `temper-core` does not depend on
//! `temper-substrate`.

use serde::{Deserialize, Serialize};

/// Terminal outcome of an invocation. Mirrors the Postgres / temper-substrate
/// `Disposition`. `open` is NOT representable here — closing requires a
/// terminal value.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invocation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inline into MCP input schemas — Anthropic tool-use does not resolve `$ref`.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Completed,
    Failed,
    Abandoned,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(Disposition::Completed).unwrap(),
            serde_json::json!("completed")
        );
        let back: Disposition = serde_json::from_value(serde_json::json!("abandoned")).unwrap();
        assert_eq!(back, Disposition::Abandoned);
    }
}
