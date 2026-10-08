//! The cross-origin policy, derived from configuration — once, for every surface.
//!
//! This lives here rather than in a transport crate because both surfaces reach it and neither
//! depends on the other: temper-api applies it in `apply_transport_layers` (shared by the public
//! and internal apps) and temper-mcp-server applies it when assembling its own router. It takes
//! the origins, not an `ApiConfig`, so the MCP server needs no API configuration to apply it.
//!
//! It is shared for a reason paid for once. The MCP router previously ended in a literal
//! `CorsLayer::permissive()`, so `CORS_ORIGINS` was read into `ApiConfig`, carried into
//! `AppState`, and then dropped at the only layer that acts — on the surface that takes the most
//! automated traffic. An operator tightening the allowlist saw no change there and nothing said
//! so. Two hand-assembled stacks are what let that happen, so there is now one function and the
//! surfaces call it.

use tower_http::cors::{AllowHeaders, Any, CorsLayer};

/// Read `CORS_ORIGINS` — comma-separated, trimmed, empties dropped. The one parse both surfaces'
/// boots run, so the allowlist the API applies and the one the MCP server applies cannot differ
/// in how they read the same variable.
pub fn parse_cors_origins(lookup: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let cors_origins: Vec<String> = lookup("CORS_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if cors_origins.is_empty() {
        tracing::info!(
            "CORS_ORIGINS is not set — cross-origin requests will be denied. \
             Set CORS_ORIGINS=* for permissive mode in development."
        );
    }
    cors_origins
}

/// Build the CORS layer this instance's configuration asks for.
///
/// Three cases, and the empty one is a deliberate default rather than an oversight:
/// - **no origins configured** — deny all cross-origin requests. Set `CORS_ORIGINS=*` for
///   permissive mode in development.
/// - **exactly `*`** — permissive.
/// - **an allowlist** — those origins, any method, and the request headers the preflight names,
///   echoed back. Echoed rather than `*`: the Fetch standard never lets the `*` wildcard cover
///   `Authorization`, so a `*` answer would bar an allowlisted browser client from sending its
///   bearer.
///
/// An origin that fails to parse is skipped rather than fataled, which means a typo narrows the
/// allowlist instead of widening it.
pub fn cors_layer(cors_origins: &[String]) -> CorsLayer {
    if cors_origins.is_empty() {
        CorsLayer::new()
    } else if cors_origins.len() == 1 && cors_origins[0] == "*" {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(
                cors_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect::<Vec<_>>(),
            )
            .allow_methods(Any)
            .allow_headers(AllowHeaders::mirror_request())
    }
}
