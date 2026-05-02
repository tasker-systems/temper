//! Surface enum — identifies the originating surface of a command.
//!
//! Each command carries a `Surface` so backends can adjust output formatting,
//! error shaping, and telemetry tagging based on where the command came from.

use serde::{Deserialize, Serialize};

/// The originating surface of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// CLI binary with a local vault (no `TEMPER_VAULT_STATE=cloud`).
    CliLocalVault,
    /// CLI binary in cloud mode (`TEMPER_VAULT_STATE=cloud`).
    CliCloud,
    /// MCP server (rmcp tools, in-process to temper-api).
    Mcp,
    /// API server (Axum handlers receiving inbound HTTP).
    ApiHttp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_serializes_snake_case() {
        let s = serde_json::to_string(&Surface::CliLocalVault).unwrap();
        assert_eq!(s, "\"cli_local_vault\"");
    }

    #[test]
    fn surface_round_trips() {
        for variant in [
            Surface::CliLocalVault,
            Surface::CliCloud,
            Surface::Mcp,
            Surface::ApiHttp,
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            let back: Surface = serde_json::from_str(&s).unwrap();
            assert_eq!(variant, back);
        }
    }
}
