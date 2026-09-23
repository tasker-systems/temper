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

/// The HTTP header the MCP relay sets to carry the calling surface across the hop
/// (the network door's attribution carrier — design §D5, ruling 6).
///
/// The relay sets it to the fixed value `mcp` on every forwarded request. At the API the
/// header is trusted ONLY beside a valid service credential (`X-Temper-Service-Credential`
/// naming `TEMPER_MCP_SERVICE_SECRET`): a valid credential plus a value on the relay
/// allowlist — exactly `{mcp}` — inserts a server-side [`RelayedSurface`] extension; any
/// other combination leaves the carrier ignored and the act attributed `@web`. The
/// single-value allowlist is load-bearing: a stolen service secret cannot generalize to
/// `cli` attribution, because the carrier's vocabulary is one value.
pub const RELAYED_SURFACE_HEADER: &str = "X-Temper-Relayed-Surface";

/// The HTTP header carrying the MCP relay's service credential (design §D2, ruling 6).
///
/// A bare secret — `Authorization` is taken by the caller's own bearer. Validates relay
/// trust at the API; authorizes nothing. The value is `TEMPER_MCP_SERVICE_SECRET`'s
/// contents, compared constant-time, fail-closed semantics per the network-door design.
pub const SERVICE_CREDENTIAL_HEADER: &str = "X-Temper-Service-Credential";

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

/// The surface of a request that arrived through the **in-process door**.
///
/// The in-process transport (temper-client's `Router::oneshot` binding) inserts this into
/// request extensions after constructing the request server-side. Extensions cannot be
/// written by a remote caller — no header reaches them — so this is the one trusted channel
/// for a surface the `X-Temper-Surface` allowlist deliberately does not admit: an MCP act
/// arriving over this door attributes `@mcp`, where the same claim sent as a header from
/// across the wire is untrusted by construction. Surface is provenance, never
/// authorization; this type moves provenance, and grants nothing.
///
/// The newtype exists so the extension map cannot confuse it with any other carried value.
#[derive(Debug, Clone, Copy)]
pub struct InProcessSurface(pub Surface);

/// The surface of a request that arrived through the **network door** — the MCP relay's
/// forwarded call (design §D5).
///
/// temper-api's relay-trust middleware inserts this into request extensions AFTER the service
/// credential validates — the one condition under which the carrier (`X-Temper-Relayed-Surface`)
/// is trusted. Like [`InProcessSurface`], it moves provenance and grants nothing: the request
/// still passes the auth middleware on the caller's own bearer, and an absent or invalid
/// credential simply means this extension is never inserted (the act degrades to `@web` —
/// mis-attribution of a provably-relayed act is what §D7's degrade detector watches for, not a
/// state this design permits by default). The extension cannot be written by a remote caller:
/// only middleware holds the credential it takes to insert it.
///
/// Priority: second, below [`InProcessSurface`] and above the public `X-Temper-Surface`
/// allowlist — resolve_surface owns the order.
#[derive(Debug, Clone, Copy)]
pub struct RelayedSurface(pub Surface);

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
