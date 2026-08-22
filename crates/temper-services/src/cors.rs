//! The cross-origin policy, derived from configuration — once, for every surface.
//!
//! This lives here rather than in a transport crate because both surfaces reach it and neither
//! depends on the other: temper-api applies it in `apply_transport_layers` (shared by the public
//! and internal apps) and temper-mcp applies it when assembling its own router.
//!
//! It is shared for a reason paid for once. The MCP router previously ended in a literal
//! `CorsLayer::permissive()`, so `CORS_ORIGINS` was read into `ApiConfig`, carried into
//! `AppState`, and then dropped at the only layer that acts — on the surface that takes the most
//! automated traffic. An operator tightening the allowlist saw no change there and nothing said
//! so. Two hand-assembled stacks are what let that happen, so there is now one function and the
//! surfaces call it.

use tower_http::cors::{Any, CorsLayer};

use crate::config::ApiConfig;

/// Build the CORS layer this instance's configuration asks for.
///
/// Three cases, and the empty one is a deliberate default rather than an oversight:
/// - **no origins configured** — deny all cross-origin requests. Set `CORS_ORIGINS=*` for
///   permissive mode in development.
/// - **exactly `*`** — permissive.
/// - **an allowlist** — those origins, any method, any header.
///
/// An origin that fails to parse is skipped rather than fataled, which means a typo narrows the
/// allowlist instead of widening it.
pub fn cors_layer(config: &ApiConfig) -> CorsLayer {
    if config.cors_origins.is_empty() {
        CorsLayer::new()
    } else if config.cors_origins.len() == 1 && config.cors_origins[0] == "*" {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin(
                config
                    .cors_origins
                    .iter()
                    .filter_map(|o| o.parse().ok())
                    .collect::<Vec<_>>(),
            )
            .allow_methods(Any)
            .allow_headers(Any)
    }
}
