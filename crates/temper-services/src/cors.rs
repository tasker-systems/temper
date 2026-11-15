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

use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::{HeaderMap, Method};
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
///   bearer. `Link` is exposed: a bounded list's continuation rides it (`rel="next"`), and a
///   browser hides every response header not exposed, so an allowlisted client would otherwise
///   read the first page as the whole list.
///
/// An origin that fails to parse is skipped rather than fataled, which means a typo narrows the
/// allowlist instead of widening it.
pub fn cors_layer(cors_origins: &[String]) -> CorsLayer {
    build_layer(&Allowed::from_config(cors_origins), &[header::LINK])
}

/// Which origins the configuration admits. Parsed once, by one function, so the origin a
/// [`CorsPolicy`] admits as a preflight is exactly the origin its layer answers with ACAO.
#[derive(Clone, Debug)]
enum Allowed {
    None,
    Any,
    List(Vec<HeaderValue>),
}

impl Allowed {
    fn from_config(cors_origins: &[String]) -> Self {
        if cors_origins.is_empty() {
            Allowed::None
        } else if cors_origins.len() == 1 && cors_origins[0] == "*" {
            Allowed::Any
        } else {
            Allowed::List(cors_origins.iter().filter_map(|o| o.parse().ok()).collect())
        }
    }
}

fn build_layer(allowed: &Allowed, expose: &[HeaderName]) -> CorsLayer {
    match allowed {
        Allowed::None => CorsLayer::new(),
        Allowed::Any => CorsLayer::permissive(),
        Allowed::List(origins) => CorsLayer::new()
            .allow_origin(origins.clone())
            .allow_methods(Any)
            .allow_headers(AllowHeaders::mirror_request())
            .expose_headers(expose.to_vec()),
    }
}

/// The cross-origin policy temper-api applies **outside routing**: the same layer [`cors_layer`]
/// builds, plus the predicate that decides which requests may see it as a preflight.
///
/// `tower_http`'s `CorsLayer` answers *every* `OPTIONS` itself, with 200, and never calls inward —
/// no `Origin` needed. Applied per endpoint, that made `OPTIONS` an anonymous route map (the router
/// then stamps the path's `Allow` on that 200). temper-api therefore hands the layer only the
/// requests [`CorsPolicy::is_admitted_preflight`] accepts, and routes every other `OPTIONS` like any
/// other method. The header bytes on both kinds of answer stay `tower_http`'s.
///
/// The MCP server keeps [`cors_layer`] where it is; moving it is that surface's own decision.
#[derive(Clone, Debug)]
pub struct CorsPolicy {
    allowed: Allowed,
    layer: CorsLayer,
}

impl CorsPolicy {
    /// The policy for these origins. An allowlist also exposes `x-request-id`, so a cross-origin
    /// browser client can read the id it will quote to an operator; permissive mode exposes every
    /// header already.
    pub fn new(cors_origins: &[String]) -> Self {
        let allowed = Allowed::from_config(cors_origins);
        let layer = build_layer(
            &allowed,
            &[header::LINK, crate::transport::REQUEST_ID_HEADER],
        );
        Self { allowed, layer }
    }

    /// Whether this request is a CORS preflight this instance answers: `OPTIONS`, an `Origin` the
    /// configuration admits, and an `Access-Control-Request-Method`. The answer to one depends only
    /// on the origin, never on the path, so it is answered before routing.
    pub fn is_admitted_preflight(&self, method: &Method, headers: &HeaderMap) -> bool {
        method == Method::OPTIONS
            && headers.contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
            && headers
                .get(header::ORIGIN)
                .is_some_and(|origin| self.admits(origin))
    }

    fn admits(&self, origin: &HeaderValue) -> bool {
        match &self.allowed {
            Allowed::None => false,
            Allowed::Any => true,
            Allowed::List(origins) => origins.contains(origin),
        }
    }

    /// The layer itself, for the outer service to wrap a preflight answerer or the router.
    pub fn layer(&self) -> CorsLayer {
        self.layer.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preflight(origin: Option<&str>, acrm: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(o) = origin {
            h.insert(header::ORIGIN, o.parse().unwrap());
        }
        if acrm {
            h.insert(
                header::ACCESS_CONTROL_REQUEST_METHOD,
                "POST".parse().unwrap(),
            );
        }
        h
    }

    // FAILS IF: the predicate admits a preflight the configuration does not, or misses one it does.
    #[test]
    fn a_preflight_is_admitted_only_for_an_allowed_origin_with_a_request_method() {
        let list = CorsPolicy::new(&["https://app.example".to_string()]);
        let any = CorsPolicy::new(&["*".to_string()]);
        let none = CorsPolicy::new(&[]);
        let ok = preflight(Some("https://app.example"), true);
        assert!(list.is_admitted_preflight(&Method::OPTIONS, &ok));
        assert!(!list.is_admitted_preflight(&Method::POST, &ok));
        assert!(!list.is_admitted_preflight(
            &Method::OPTIONS,
            &preflight(Some("https://app.example"), false)
        ));
        assert!(!list.is_admitted_preflight(
            &Method::OPTIONS,
            &preflight(Some("https://evil.example"), true)
        ));
        assert!(!list.is_admitted_preflight(&Method::OPTIONS, &preflight(None, true)));
        assert!(any.is_admitted_preflight(
            &Method::OPTIONS,
            &preflight(Some("https://evil.example"), true)
        ));
        assert!(!none.is_admitted_preflight(&Method::OPTIONS, &ok));
    }
}
