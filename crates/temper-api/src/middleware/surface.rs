//! Resolving the caller's claimed surface from the `X-Temper-Surface` request header.
//!
//! Surface is **provenance, never authorization**. It selects which `<handle>@<marker>` emitter
//! entity a write is attributed to in the event ledger. It grants nothing. A bad value therefore
//! degrades — it never rejects, and it never 500s.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use std::convert::Infallible;
use std::future::Future;

use temper_workflow::operations::{InProcessSurface, RelayedSurface, Surface, SURFACE_HEADER};

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

/// Resolve the surface of an inbound request.
///
/// The trusted channels speak first, in priority order; the header allowlist decides only
/// when neither spoke:
///
/// 1. **`InProcessSurface`** — inserted server-side by the `Router::oneshot` transport, where
///    no remote caller can write. Trusted outright, overrides everything.
/// 2. **`RelayedSurface`** — inserted by the relay-trust middleware ONLY beside a valid service
///    credential (the network door's attribution carrier, design §D5). A remote caller can send
///    the carrier header, but cannot make the middleware insert the extension without the
///    secret, so the extension is trusted exactly as far as the credential is.
/// 3. The `X-Temper-Surface` header allowlist — exactly `{cli, sdk}`.
/// 4. Degrade to [`Surface::ApiHttp`] (`web`).
///
/// Never fails. An untrusted claim is logged at debug — it is ordinary traffic (every browser
/// request omits the header), not an anomaly worth a warning.
fn resolve_surface(
    in_process: Option<InProcessSurface>,
    relayed: Option<RelayedSurface>,
    headers: &HeaderMap,
) -> Surface {
    if let Some(InProcessSurface(surface)) = in_process {
        return surface;
    }
    if let Some(RelayedSurface(surface)) = relayed {
        return surface;
    }
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

/// The surface this request was received on — the in-process door's trusted extension when
/// present, else `X-Temper-Surface`.
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
        let in_process = parts.extensions.get::<InProcessSurface>().copied();
        let relayed = parts.extensions.get::<RelayedSurface>().copied();
        std::future::ready(Ok(RequestSurface(resolve_surface(
            in_process,
            relayed,
            &parts.headers,
        ))))
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
        assert_eq!(
            resolve_surface(None, None, &HeaderMap::new()),
            Surface::ApiHttp
        );
    }

    #[test]
    fn untrusted_header_degrades_to_web() {
        assert_eq!(
            resolve_surface(None, None, &headers_with("mcp")),
            Surface::ApiHttp
        );
        assert_eq!(
            resolve_surface(None, None, &headers_with("nonsense")),
            Surface::ApiHttp
        );
        assert_eq!(
            resolve_surface(None, None, &headers_with("")),
            Surface::ApiHttp
        );
    }

    #[test]
    fn trusted_header_resolves() {
        assert_eq!(
            resolve_surface(None, None, &headers_with("cli")),
            Surface::CliCloud
        );
        assert_eq!(
            resolve_surface(None, None, &headers_with("sdk")),
            Surface::Sdk
        );
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
        assert_eq!(resolve_surface(None, None, &h), Surface::ApiHttp);
    }

    // --- the in-process door: the trusted extension, which no header can reach ---

    /// Build `request::Parts` the way the extractor sees them, from headers and extensions.
    fn parts_with(
        in_process: Option<InProcessSurface>,
        relayed: Option<RelayedSurface>,
        headers: HeaderMap,
    ) -> axum::http::request::Parts {
        let mut builder = axum::http::Request::builder();
        for (k, v) in headers.iter() {
            builder = builder.header(k, v);
        }
        let mut request = builder.body(()).expect("headerless body request");
        if let Some(surface) = in_process {
            request.extensions_mut().insert(surface);
        }
        if let Some(surface) = relayed {
            request.extensions_mut().insert(surface);
        }
        let (parts, _) = request.into_parts();
        parts
    }

    /// The whole point of the extension: an MCP act arriving through the in-process door
    /// attributes `@mcp`, where the same claim crossing the wire as a header is untrusted.
    #[test]
    fn the_in_process_extension_resolves_its_surface() {
        for surface in Surface::ALL {
            let parts = parts_with(Some(InProcessSurface(surface)), None, HeaderMap::new());
            let in_process = parts.extensions.get::<InProcessSurface>().copied();
            assert_eq!(resolve_surface(in_process, None, &parts.headers), surface);
        }
    }

    /// The extension is the trusted channel and outranks the header: an in-process MCP
    /// request carrying a spoofed `sdk` header still attributes `@mcp`.
    #[test]
    fn the_in_process_extension_outranks_any_header_claim() {
        let parts = parts_with(
            Some(InProcessSurface(Surface::Mcp)),
            None,
            headers_with("sdk"),
        );
        let in_process = parts.extensions.get::<InProcessSurface>().copied();
        assert_eq!(
            resolve_surface(in_process, None, &parts.headers),
            Surface::Mcp
        );
    }

    /// The extractor itself: an extension-less request resolves exactly as before, through
    /// the header allowlist — the in-process door adds a channel, it does not rewire the old one.
    #[test]
    fn without_the_extension_the_extractor_resolves_headers_as_before() {
        let untrusted = parts_with(None, None, headers_with("mcp"));
        let resolved = tokio::runtime::Runtime::new().unwrap().block_on(async {
            RequestSurface::from_request_parts(&mut untrusted.clone(), &())
                .await
                .expect("infallible")
                .0
        });
        assert_eq!(resolved, Surface::ApiHttp);

        let trusted = parts_with(None, None, headers_with("cli"));
        let resolved = tokio::runtime::Runtime::new().unwrap().block_on(async {
            RequestSurface::from_request_parts(&mut trusted.clone(), &())
                .await
                .expect("infallible")
                .0
        });
        assert_eq!(resolved, Surface::CliCloud);
    }

    // --- the network door: the relayed extension, trusted beside the service credential ---

    /// The network door's whole point: an MCP act arriving through the relay attributes `@mcp`,
    /// where the same claim sent as a header is untrusted. The relay-trust middleware only
    /// inserts the extension beside a valid service credential — that condition is ITS
    /// contract (see `relay_trust.rs`); resolution trusts the extension it finds.
    #[test]
    fn the_relayed_extension_resolves_mcp() {
        let parts = parts_with(None, Some(RelayedSurface(Surface::Mcp)), HeaderMap::new());
        let in_process = parts.extensions.get::<InProcessSurface>().copied();
        let relayed = parts.extensions.get::<RelayedSurface>().copied();
        assert_eq!(
            resolve_surface(in_process, relayed, &parts.headers),
            Surface::Mcp
        );
    }

    /// Priority: in-process speaks first. The in-process transport is the incumbent door and
    /// never relays; if both extensions are present, the in-process claim wins.
    #[test]
    fn the_in_process_extension_outranks_the_relayed_one() {
        let parts = parts_with(
            Some(InProcessSurface(Surface::CliCloud)),
            Some(RelayedSurface(Surface::Mcp)),
            HeaderMap::new(),
        );
        let in_process = parts.extensions.get::<InProcessSurface>().copied();
        let relayed = parts.extensions.get::<RelayedSurface>().copied();
        assert_eq!(
            resolve_surface(in_process, relayed, &parts.headers),
            Surface::CliCloud
        );
    }

    /// The relayed extension outranks the header: a relayed MCP request carrying a spoofed
    /// `sdk` header still attributes `@mcp` — the carrier channel is trusted, the header is not.
    #[test]
    fn the_relayed_extension_outranks_any_header_claim() {
        let parts = parts_with(
            None,
            Some(RelayedSurface(Surface::Mcp)),
            headers_with("sdk"),
        );
        let in_process = parts.extensions.get::<InProcessSurface>().copied();
        let relayed = parts.extensions.get::<RelayedSurface>().copied();
        assert_eq!(
            resolve_surface(in_process, relayed, &parts.headers),
            Surface::Mcp
        );
    }

    /// Without the extension, the header allowlist decides exactly as before — the network
    /// door adds a channel, it does not rewire the old ones.
    #[test]
    fn the_relayed_arm_is_absent_by_default() {
        let parts = parts_with(None, None, headers_with("mcp"));
        let in_process = parts.extensions.get::<InProcessSurface>().copied();
        let relayed = parts.extensions.get::<RelayedSurface>().copied();
        assert_eq!(
            resolve_surface(in_process, relayed, &parts.headers),
            Surface::ApiHttp
        );
    }
}
