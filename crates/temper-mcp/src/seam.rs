//! The identity seam — where a host says, per request, who the relay acts as.
//!
//! The tool layer relays every act to temper's API. What it relays **as** is the host's
//! business, never this crate's: a host implements [`IdentitySeam`], and for each request the
//! seam yields an [`OutgoingIdentity`] (the bearer, the [`Surface`], any opaque extra headers,
//! an optional `correlation_id`) or nothing. Nothing means every tool answers the crate's one
//! host-neutral not-connected refusal ([`not_connected`]).
//!
//! The crate alone builds the [`TemperClient`] from that identity, in one place
//! (`TemperMcpService::relay_client`): the shared connection pool built from the host's
//! [`RelayConfig`], the non-idempotent attempt count, and the no-redirect policy. Every host's
//! client is built the same way, so retry and redirect behaviour cannot drift per host.
//!
//! The crate never reads an incoming credential, never verifies one, and never stores one
//! across requests. What a host puts in the extra headers is opaque here: the crate never
//! names a header, a secret or an environment variable. (One dependency does read the
//! environment: temper-client's endpoint check honours its insecure-http override, which a
//! host's process environment can set. The crate's own source reads none — a source gate holds
//! that, as a tripwire for review rather than a sandbox.)
//!
//! [`TemperClient`]: temper_client::TemperClient

use std::time::Duration;

use temper_workflow::operations::Surface;

/// Where the relay sends acts, and how long one may take. Plain values: a host builds it from
/// its own sources, and this crate never reads configuration of any kind.
///
/// The timeout is the host's because it is the one transport figure that depends on the host's
/// runtime budget: a serverless host sizes it below its function's ceiling, a long-lived process
/// can afford more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    /// Base URL of the temper API every tool act is relayed to (e.g. `https://temperkb.io`).
    pub api_base_url: String,
    /// The per-request timeout of the relay's connection pool.
    pub request_timeout: Duration,
}

impl RelayConfig {
    pub fn new(api_base_url: impl Into<String>, request_timeout: Duration) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            request_timeout,
        }
    }

    /// Build the relay's ONE connection pool. Called once, when the service is constructed;
    /// every per-request client reuses it (refcount-cloned), so a tool call never pays a fresh
    /// TLS handshake.
    pub(crate) fn connection_pool(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(self.request_timeout)
            // Redirects are refused, never followed: every per-request client carries the
            // caller's bearer and the host's extra headers as default headers, and reqwest
            // replays default headers on every redirect hop. A 3xx answered by anything in
            // front of the API must not be able to re-send them to an origin of its choosing.
            // No API route emits a 3xx; this makes the relay independent of that fact.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build the relay's shared HTTP client")
    }
}

/// Who one relayed request acts as: the identity a host's [`IdentitySeam`] yields.
///
/// Built with [`OutgoingIdentity::new`] and the `with_*` setters. `Debug` renders the presence
/// of the bearer and the extra headers' names, never their values.
#[derive(Clone)]
pub struct OutgoingIdentity {
    bearer: String,
    surface: Surface,
    extra_headers: http::HeaderMap,
    correlation_id: Option<uuid::Uuid>,
}

impl OutgoingIdentity {
    /// The bearer presented to the API, and the surface the client declares.
    pub fn new(bearer: impl Into<String>, surface: Surface) -> Self {
        Self {
            bearer: bearer.into(),
            surface,
            extra_headers: http::HeaderMap::new(),
            correlation_id: None,
        }
    }

    /// Headers sent on every request of this identity, verbatim. Opaque to the crate.
    pub fn with_extra_headers(mut self, headers: http::HeaderMap) -> Self {
        self.extra_headers = headers;
        self
    }

    /// The act-grain thread a write joins when the agent sent none. Applied only to a tool whose
    /// input declares `correlation_id`, and never over a value the agent supplied.
    pub fn with_correlation_id(mut self, correlation_id: uuid::Uuid) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    pub fn surface(&self) -> Surface {
        self.surface
    }

    pub fn correlation_id(&self) -> Option<uuid::Uuid> {
        self.correlation_id
    }

    pub(crate) fn into_parts(self) -> (String, Surface, http::HeaderMap) {
        (self.bearer, self.surface, self.extra_headers)
    }
}

impl std::fmt::Debug for OutgoingIdentity {
    /// Hand-written, not derived: the bearer and (typically) the extra headers are live
    /// credentials. Presence and header names, never values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutgoingIdentity")
            .field("bearer", &"<redacted>")
            .field("surface", &self.surface)
            .field(
                "extra_headers",
                &self
                    .extra_headers
                    .keys()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("correlation_id", &self.correlation_id)
            .finish()
    }
}

/// What a host implements: per request, the identity the relay acts as, or nothing.
///
/// `parts` are the request's HTTP parts when the transport is HTTP; on a transport without them
/// the seam is handed empty parts, so a host whose credential does not ride the request (a
/// credential store, a signed-in session) answers the same way on every transport.
///
/// The seam is consulted once per tool call, before dispatch; the relay client for that call is
/// built from the identity it answered. (A resources-protocol read consults it once more, on its
/// own.)
///
/// What the seam is for, and what it is not:
/// - it says where the outgoing credential comes from — it never verifies one. A host verifies
///   an incoming credential at its own edge, if it has one; the API re-verifies every act.
/// - the crate stores nothing across requests; a host that caches a credential owns that cache.
/// - extra headers are opaque, but may not restate identity: a header the relay sets itself
///   (`authorization`, the surface, the device id, trace context) refuses the call. A shared
///   service credential belongs here only if the API is meant to trust this host's carrier.
pub trait IdentitySeam: Send + Sync + 'static {
    fn outgoing_identity(&self, parts: &http::request::Parts) -> Option<OutgoingIdentity>;
}

/// The refusal every tool answers when the host's seam yields no identity. Defined once, here,
/// and host-neutral: it names no host, no variable and no remedy a particular host would offer.
pub fn not_connected() -> rmcp::ErrorData {
    rmcp::ErrorData::new(
        rmcp::model::ErrorCode::INVALID_REQUEST,
        NOT_CONNECTED_SENTENCE.to_string(),
        None,
    )
}

/// The not-connected refusal's sentence.
pub const NOT_CONNECTED_SENTENCE: &str =
    "This MCP host is not connected to temper: it supplied no credential for this call, so no \
     tool can act. Connect the host to a temper account, then retry.";

#[cfg(test)]
mod tests {
    use super::*;

    /// FAILS IF: an identity's Debug renders the bearer or a header value.
    #[test]
    fn outgoing_identity_debug_never_renders_a_credential() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "x-opaque",
            http::HeaderValue::from_static("header-secret-value"),
        );
        let rendered = format!(
            "{:?}",
            OutgoingIdentity::new("bearer-secret-value", Surface::CliCloud)
                .with_extra_headers(headers)
        );
        assert!(!rendered.contains("bearer-secret-value"), "{rendered}");
        assert!(!rendered.contains("header-secret-value"), "{rendered}");
        assert!(
            rendered.contains("x-opaque"),
            "names are the useful fact: {rendered}"
        );
    }

    /// The relay pool never follows a redirect. Its clients carry the host's extra
    /// headers (for the deployed door, the service credential) and the caller's bearer as constructor-level default headers, and
    /// reqwest replays default headers on every redirect hop — a followed 3xx would hand
    /// both secrets to whatever origin the `Location` names. The probe binds a real
    /// listener whose `/here` answers 307 at a canary on the same server: a following
    /// client returns the canary's 200 marker, the relay's pool returns the 302 itself.
    ///
    /// `[added — 2026-09-23, found in review]` reqwest's default policy follows up to ten
    /// hops; the API emits no 3xx today, so this witness pins the relay's behavior to that
    /// fact's independence.
    #[tokio::test]
    async fn the_relay_pool_does_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // A hand-rolled HTTP/1.1 probe (the tool layer assembles no router, even in a test):
        // `/here` answers 307 at the canary, the canary answers the marker. Each connection
        // serves one request.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the probe listener");
        let addr = listener.local_addr().expect("probe address");
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let response = if request.starts_with("GET /here ") {
                    "HTTP/1.1 307 Temporary Redirect\r\n\
                     location: /canary-that-must-never-be-fetched\r\n\
                     content-length: 0\r\nconnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\ncontent-length: 16\r\nconnection: close\r\n\r\n\
                     SECRETS REPLAYED"
                };
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let client =
            RelayConfig::new("http://unused.invalid", Duration::from_secs(5)).connection_pool();
        let resp = client
            .get(format!("http://{addr}/here"))
            .send()
            .await
            .expect("the probe answers");

        assert_eq!(
            resp.status().as_u16(),
            307,
            "the redirect itself is the answer, never the location it names"
        );
        let body = resp.text().await.expect("redirect body reads");
        assert!(
            !body.contains("SECRETS REPLAYED"),
            "the canary must never be fetched: {body:?}"
        );

        server.abort();
    }
}
