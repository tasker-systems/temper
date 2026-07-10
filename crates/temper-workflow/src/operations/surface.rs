//! Surface enum — identifies the originating surface of a command.
//!
//! Each command carries a `Surface` so backends can adjust output formatting,
//! error shaping, and telemetry tagging based on where the command came from.

use serde::{Deserialize, Serialize};

/// The HTTP header a remote client uses to claim its calling surface.
///
/// The value is the claimed surface's [`Surface::marker`] spelling — the same `<marker>` half
/// of the `<handle>@<marker>` emitter natural key the write will be attributed to. The header
/// names the emitter the caller claims to be.
///
/// The server trusts exactly `cli` and `sdk`; everything else degrades to [`Surface::ApiHttp`].
/// Surface is provenance, never authorization.
pub const SURFACE_HEADER: &str = "X-Temper-Surface";

/// The originating surface of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// CLI binary operating in cloud mode.
    CliCloud,
    /// MCP server (rmcp tools, in-process to temper-api).
    Mcp,
    /// API server (Axum handlers receiving inbound HTTP).
    ApiHttp,
    /// Generated SDK client (`temper-rb` and its successors) calling the API over HTTP.
    /// Named for the kind of surface, not the client's language, so `temper-py` and
    /// `temper-ts` inherit it.
    Sdk,
}

impl Surface {
    /// Every surface. `profile_service` provisions one `<handle>@<marker>` emitter entity per
    /// element, so adding a variant here also obliges an additive migration backfilling that
    /// emitter for profiles that already exist.
    pub const ALL: [Surface; 4] = [
        Surface::ApiHttp,
        Surface::CliCloud,
        Surface::Mcp,
        Surface::Sdk,
    ];

    /// The per-surface emitter marker: the `<marker>` half of the `<handle>@<marker>` natural key
    /// that `temper_substrate::writes::resolve_emitter` resolves against `kb_entities`.
    ///
    /// Deliberately distinct from the serde representation — `ApiHttp` emits as `web`, which is
    /// temperkb.io's surface. A marker rename renames a durable entity and needs a migration.
    pub fn marker(self) -> &'static str {
        match self {
            Surface::CliCloud => "cli",
            Surface::Mcp => "mcp",
            Surface::ApiHttp => "web",
            Surface::Sdk => "sdk",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn surface_serializes_snake_case() {
        let s = serde_json::to_string(&Surface::CliCloud).unwrap();
        assert_eq!(s, "\"cli_cloud\"");
    }

    #[test]
    fn surface_round_trips() {
        for variant in Surface::ALL {
            let s = serde_json::to_string(&variant).unwrap();
            let back: Surface = serde_json::from_str(&s).unwrap();
            assert_eq!(variant, back);
        }
    }

    /// Two surfaces sharing a marker would collapse onto one emitter entity, silently
    /// merging their ledger attribution.
    #[test]
    fn markers_are_distinct() {
        let markers: HashSet<&str> = Surface::ALL.iter().map(|s| s.marker()).collect();
        assert_eq!(markers.len(), Surface::ALL.len());
    }

    /// Both ends of the wire spell the header from this constant. A literal on either side
    /// would be a silent, untestable drift.
    #[test]
    fn surface_header_name_is_stable() {
        assert_eq!(SURFACE_HEADER, "X-Temper-Surface");
    }

    /// The markers are a durable natural key: `kb_entities.name` is `<handle>@<marker>`, and
    /// changing one orphans every emitter row already written under the old spelling.
    #[test]
    fn markers_are_stable() {
        assert_eq!(Surface::ApiHttp.marker(), "web");
        assert_eq!(Surface::CliCloud.marker(), "cli");
        assert_eq!(Surface::Mcp.marker(), "mcp");
        assert_eq!(Surface::Sdk.marker(), "sdk");
    }
}
