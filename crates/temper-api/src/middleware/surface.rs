//! Resolving the caller's claimed surface from the `X-Temper-Surface` request header.
//!
//! Surface is **provenance, never authorization**. It selects which `<handle>@<marker>` emitter
//! entity a write is attributed to in the event ledger. It grants nothing. A bad value therefore
//! degrades — it never rejects, and it never 500s.

use axum::extract::FromRequestParts;
use axum::http::{request::Parts, HeaderMap};
use std::convert::Infallible;
use std::future::Future;

use temper_workflow::operations::{Surface, SURFACE_HEADER};

/// Parse a client-claimed surface marker into the surface it names.
///
/// This function **is** the allowlist. It trusts exactly two markers:
///
/// - `cli` — `temper-cli` in cloud mode, forwarding over HTTP.
/// - `sdk` — a generated SDK client (`temper-rb` and its successors).
///
/// Everything else is `None`, including `mcp`: `temper-mcp` reaches `DbBackend` in-process and
/// never crosses this boundary, so a remote caller claiming `mcp` is lying by construction. And
/// including `web`, which is what an unclaimed request degrades to anyway.
fn parse_trusted(raw: &str) -> Option<Surface> {
    match raw.trim() {
        "cli" => Some(Surface::CliCloud),
        "sdk" => Some(Surface::Sdk),
        _ => None,
    }
}

/// Resolve the surface of an inbound request, degrading to [`Surface::ApiHttp`] (`web`) whenever
/// the header is absent, unreadable, or not on the allowlist.
///
/// Never fails. An untrusted claim is logged at debug — it is ordinary traffic (every browser
/// request omits the header), not an anomaly worth a warning.
fn resolve_surface(headers: &HeaderMap) -> Surface {
    let Some(raw) = headers.get(SURFACE_HEADER) else {
        return Surface::ApiHttp;
    };
    let Ok(value) = raw.to_str() else {
        tracing::debug!("{SURFACE_HEADER} is not valid ASCII; attributing to web");
        return Surface::ApiHttp;
    };
    match parse_trusted(value) {
        Some(surface) => surface,
        None => {
            tracing::debug!(claimed = %value, "untrusted {SURFACE_HEADER}; attributing to web");
            Surface::ApiHttp
        }
    }
}

/// The surface this request was received on, resolved from `X-Temper-Surface`.
///
/// Handlers take this extractor instead of hardcoding [`Surface::ApiHttp`], and pass the inner
/// value as their command's `origin`. Extraction is infallible by design: an unparseable claim
/// degrades to `web` rather than rejecting the request.
#[derive(Debug, Clone, Copy)]
pub struct RequestSurface(pub Surface);

impl<S> FromRequestParts<S> for RequestSurface
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(RequestSurface(resolve_surface(&parts.headers))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use temper_workflow::operations::Surface;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(SURFACE_HEADER, value.parse().expect("valid header value"));
        h
    }

    /// The client's send-side spelling and the server's accept-side allowlist are the same
    /// strings. Deriving the test input from `marker()` means they cannot drift.
    #[test]
    fn trusted_markers_round_trip_from_the_client_spelling() {
        assert_eq!(
            parse_trusted(Surface::CliCloud.marker()),
            Some(Surface::CliCloud)
        );
        assert_eq!(parse_trusted(Surface::Sdk.marker()), Some(Surface::Sdk));
    }

    /// `temper-mcp` reaches `DbBackend` in-process, so a remote caller claiming `mcp` is
    /// lying by construction. It is untrusted, not merely unrecognized.
    #[test]
    fn mcp_is_not_trusted() {
        assert_eq!(parse_trusted(Surface::Mcp.marker()), None);
    }

    /// `web` is what everything degrades *to*. A caller cannot claim it either — claiming it
    /// and being degraded to it are the same outcome, so the allowlist stays exactly two.
    #[test]
    fn web_is_not_claimable() {
        assert_eq!(parse_trusted(Surface::ApiHttp.marker()), None);
    }

    #[test]
    fn garbage_and_empty_are_not_trusted() {
        assert_eq!(parse_trusted(""), None);
        assert_eq!(parse_trusted("   "), None);
        assert_eq!(parse_trusted("CLI"), None);
        assert_eq!(parse_trusted("cli; drop table"), None);
        assert_eq!(parse_trusted("sdkx"), None);
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert_eq!(parse_trusted("  cli  "), Some(Surface::CliCloud));
    }

    // --- resolve_surface: the degrade direction, which must never reject ---

    #[test]
    fn absent_header_degrades_to_web() {
        assert_eq!(resolve_surface(&HeaderMap::new()), Surface::ApiHttp);
    }

    #[test]
    fn untrusted_header_degrades_to_web() {
        assert_eq!(resolve_surface(&headers_with("mcp")), Surface::ApiHttp);
        assert_eq!(resolve_surface(&headers_with("nonsense")), Surface::ApiHttp);
        assert_eq!(resolve_surface(&headers_with("")), Surface::ApiHttp);
    }

    #[test]
    fn trusted_header_resolves() {
        assert_eq!(resolve_surface(&headers_with("cli")), Surface::CliCloud);
        assert_eq!(resolve_surface(&headers_with("sdk")), Surface::Sdk);
    }

    /// A header whose bytes are not valid ASCII cannot even be `to_str`'d. It degrades; it
    /// must not panic and must not 500.
    #[test]
    fn non_ascii_header_degrades_to_web() {
        let mut h = HeaderMap::new();
        h.insert(
            SURFACE_HEADER,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("opaque bytes"),
        );
        assert_eq!(resolve_surface(&h), Surface::ApiHttp);
    }
}
