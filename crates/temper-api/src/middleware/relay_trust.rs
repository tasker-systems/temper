//! Relay-trust middleware — validating the MCP relay's service credential, and the only
//! condition under which the attribution carrier is trusted (the network door, design §D2/§D5).
//!
//! The MCP function (the relay) forwards every tool act to this API over HTTPS, carrying the
//! caller's own bearer plus two headers it set itself: `X-Temper-Service-Credential` (the shared
//! service secret) and `X-Temper-Relayed-Surface: mcp` (the attribution carrier). This
//! middleware validates the credential and — only beside a valid one — inserts the
//! [`RelayedSurface`] extension that `resolve_surface` reads to attribute the act `@mcp`.
//!
//! **It gates trust, not routes.** A missing or invalid credential never rejects: the request
//! continues exactly as any direct caller's would (the auth middleware on the caller's bearer
//! owns authorization — one-trust-domain), and the carrier is simply ignored, degrading the act
//! to `@web`. There is no wire difference an attacker can probe: the faces are (a) no
//! credential ⇒ carrier inert, (b) invalid credential ⇒ carrier inert (debug-sampled, never a
//! warn an internet-reachable line could pull — the repo's `UnknownKid` precedent), (c) valid
//! credential ⇒ carrier honored if its value is on the relay allowlist.
//!
//! **The relay allowlist is exactly `{mcp}`.** A carrier value of anything else — `cli`,
//! `sdk`, garbage — is ignored even beside a valid credential, so a stolen service secret
//! cannot forge CLI attribution: the network door carries MCP acts, and its carrier's
//! vocabulary is one value by design.
//!
//! Scope: mounted on the gated stack only — the one stack whose handlers extract
//! [`RequestSurface`](crate::middleware::surface::RequestSurface) (wherever
//! surface-attributed writes happen). The auth-only stack mounts it NOT: none of its
//! handlers extracts `RequestSurface`, so there is no attribution to trust there — a
//! handler that later extracts it on that stack MUST bring the middleware with it or
//! its relayed acts silently degrade to `@web` (§D7's watched-for symptom). Internal
//! routes are excluded for the same reason plus a different one: a secret-holder gains
//! nothing there (their writes attribute to their own emitters). Ordering is free
//! among pre-handler layers; the only semantic requirement is preceding
//! `RequestSurface` extraction, which every middleware satisfies by construction.

use axum::{body::Body, extract::State, http::Request, middleware::Next, response::Response};

use temper_services::state::AppState;
use temper_workflow::operations::{
    RelayedSurface, Surface, RELAYED_SURFACE_HEADER, SERVICE_CREDENTIAL_HEADER,
};

/// Constant-time equality over the presented and expected secret — avoids a byte-by-byte
/// early-exit timing oracle on the shared secret. Same shape as
/// `handlers::embed::secret_matches`, kept local rather than cross-imported so the two gates
/// cannot drift into a shared mutable dependency (they gate different secrets with different
/// failure faces, and only the comparison is common).
fn secret_matches(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The one relay-allowed carrier value. The network door carries MCP acts and only MCP acts;
/// a `const` makes the allowlist a fact of the code rather than a comparison to re-derive.
const RELAY_ALLOWED_CARRIER: &str = "mcp";

/// Validate the relay credential and, beside it, honor the attribution carrier.
///
/// Never rejects — the return is a plain `Response` because there is no rejection arm to
/// type. The `RelayedSurface` extension is inserted only when BOTH hold: the presented
/// credential matches the configured secret, AND the carrier value is exactly `mcp`.
/// Inbound `Authorization` is untouched — the caller's bearer rides it and is the auth
/// middleware's concern alone.
pub async fn require_relay_trust(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let credential_valid = match (
        state.config.mcp_service_secret.as_deref(),
        request
            .headers()
            .get(SERVICE_CREDENTIAL_HEADER)
            .and_then(|v| v.to_str().ok()),
    ) {
        // Unset at the API ⇒ the carrier is never trusted; direct traffic is unaffected and
        // nothing logs — an unconfigured deployment is a posture, not an event (design §D2).
        (None, _) => false,
        (Some(_), None) => {
            // A carrier header without a credential is the shape every spoofing attempt
            // takes — but also the shape of a misconfigured relay mid-rotation. The degrade
            // detector (§D7) is the load-bearing signal, not this line; `debug` keeps an
            // internet-reachable log-volume lever out of reach.
            //
            // Counted ONLY when a carrier is actually present. The event's trigger is the
            // carrier (the §D7 signal is "a relay-shaped claim arrived uncredentialed");
            // counting every plainly credential-less request — all direct traffic, the
            // moment the API configures a secret — would drown the signal the alert keys
            // on. No carrier, no event: a missing credential is inert, per face (a).
            if request.headers().contains_key(RELAYED_SURFACE_HEADER) {
                tracing::debug!(
                    counter = "relayed_surface_degraded",
                    "carrier present without a service credential; ignoring carrier"
                );
            }
            false
        }
        (Some(expected), Some(presented)) => {
            if secret_matches(presented, expected) {
                true
            } else {
                // Invalid credential: the same degrade, `debug`-sampled (design §D2).
                tracing::debug!(
                    counter = "relayed_surface_degraded",
                    "invalid service credential; ignoring carrier"
                );
                false
            }
        }
    };

    let mut request = request;
    if credential_valid {
        let claimed = request
            .headers()
            .get(RELAYED_SURFACE_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim);
        match claimed {
            // Valid credential + the one allowed value: insert the trusted extension.
            Some(value) if value == RELAY_ALLOWED_CARRIER => {
                request
                    .extensions_mut()
                    .insert(RelayedSurface(Surface::Mcp));
                tracing::debug!(counter = "relayed_surface_trusted", "carrier honored");
            }
            // Valid credential, carrier not on the allowlist: ignored entirely (degrade).
            // The claim is attacker-writable header content — counted, never logged verbatim.
            other => {
                tracing::debug!(
                    counter = "relayed_surface_degraded",
                    present = other.is_some(),
                    "carrier value is not on the relay allowlist; ignoring carrier"
                );
            }
        }
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut h = axum::http::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_lowercase(k.as_bytes()).expect("lowercase name"),
                HeaderValue::from_str(v).expect("visible value"),
            );
        }
        h
    }

    /// The decision the middleware makes, extracted over the parts a request
    /// carries — the same inputs `require_relay_trust` reads — so the trust
    /// matrix is assertable without mounting a router.
    fn resolve(
        configured: Option<&str>,
        request_headers: &axum::http::HeaderMap,
    ) -> Option<Surface> {
        let credential_valid = match (
            configured,
            request_headers
                .get(SERVICE_CREDENTIAL_HEADER)
                .and_then(|v| v.to_str().ok()),
        ) {
            (None, _) => false,
            (Some(_), None) => false,
            (Some(expected), Some(presented)) => secret_matches(presented, expected),
        };
        if !credential_valid {
            return None;
        }
        match request_headers
            .get(RELAYED_SURFACE_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
        {
            Some(value) if value == RELAY_ALLOWED_CARRIER => Some(Surface::Mcp),
            _ => None,
        }
    }

    #[test]
    fn valid_credential_and_mcp_carrier_honors_the_extension() {
        let h = headers(&[
            ("x-temper-service-credential", "s3cret"),
            ("x-temper-relayed-surface", "mcp"),
        ]);
        assert_eq!(resolve(Some("s3cret"), &h), Some(Surface::Mcp));
    }

    #[test]
    fn a_stolen_credential_cannot_generalize_to_cli() {
        // The residual the secret's confidentiality bounds: its holder can claim the one
        // value the allowlist admits. `cli` — and every other spelling — degrades. Case
        // matters (`parse_trusted` is case-sensitive; so is this), surrounding whitespace
        // is tolerated exactly as the surface header's convention tolerates it.
        for carrier in ["cli", "sdk", "web", "MCP", "mcp; drop table", ""] {
            let h = headers(&[
                ("x-temper-service-credential", "s3cret"),
                ("x-temper-relayed-surface", carrier),
            ]);
            assert_eq!(
                resolve(Some("s3cret"), &h),
                None,
                "carrier {carrier:?} must be ignored"
            );
        }
        let padded = headers(&[
            ("x-temper-service-credential", "s3cret"),
            ("x-temper-relayed-surface", "  mcp "),
        ]);
        assert_eq!(resolve(Some("s3cret"), &padded), Some(Surface::Mcp));
    }

    #[test]
    fn absent_or_invalid_credential_leaves_the_carrier_inert() {
        let with_carrier = headers(&[
            ("x-temper-service-credential", "wrong"),
            ("x-temper-relayed-surface", "mcp"),
        ]);
        assert_eq!(resolve(Some("s3cret"), &with_carrier), None);

        let carrier_only = headers(&[("x-temper-relayed-surface", "mcp")]);
        assert_eq!(resolve(Some("s3cret"), &carrier_only), None);

        // Unset at the API: the quiet degrade — never trusted, never an event.
        assert_eq!(resolve(None, &with_carrier), None);
    }

    #[test]
    fn constant_time_compare_agrees_with_equality() {
        assert!(secret_matches("s3cret", "s3cret"));
        assert!(!secret_matches("s3cret", "s3cerT"));
        assert!(!secret_matches("s3cret", "s3cre"));
        assert!(!secret_matches("s3cret", "s3cret2"));
        assert!(secret_matches("", ""));
    }
}
