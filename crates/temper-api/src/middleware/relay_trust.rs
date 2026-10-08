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
//! credential ⇒ carrier inert, (b) invalid credential ⇒ carrier inert, (c) valid credential ⇒
//! carrier honored if its value is on the relay allowlist. Which face a relay-shaped request
//! met is recorded on its root span as `relay_trust` (`RelayTrust`) — never as a log line,
//! which an internet-reachable caller could pull as a volume lever (the repo's `UnknownKid`
//! precedent).
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

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};

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

/// What a relay-shaped request's trust headers amounted to — the degrade detector (design §D7).
///
/// Recorded as the `relay_trust` field on the request's root span, so production can see it:
/// the exporter ships spans at `info`, and every request already exports one root span, so the
/// signal adds no line an internet-reachable caller can multiply. It was a `debug` event, which
/// kept it out of anyone's reach — including the operator's, since nothing below `info` is
/// exported. A sustained non-`trusted` value in a deployment whose relay is configured is the
/// alert: `invalid_credential` is the rotation-skew signature, `no_credential` the spoofing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayTrust {
    /// Valid credential and the allowlisted carrier: the act attributes `@mcp`.
    Trusted,
    /// A carrier arrived with no credential — every spoofing attempt's shape.
    NoCredential,
    /// A credential arrived and did not match, carrier or not.
    InvalidCredential,
    /// A valid credential with no carrier — a relay misbehaving.
    CarrierMissing,
    /// A valid credential with a carrier off the allowlist (`cli`, garbage).
    CarrierRefused,
}

impl RelayTrust {
    /// The span value. A closed vocabulary: the carrier's own (attacker-writable) value is never
    /// recorded.
    fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::NoCredential => "no_credential",
            Self::InvalidCredential => "invalid_credential",
            Self::CarrierMissing => "carrier_missing",
            Self::CarrierRefused => "carrier_refused",
        }
    }
}

/// Classify a request's trust headers against the configured secret. `None` when the request is
/// not relay-shaped: the API has no secret configured (an unconfigured deployment is a posture,
/// not an event — design §D2), or neither a credential nor a carrier arrived (all direct traffic,
/// which must not drown the signal the alert keys on).
fn assess(configured: Option<&str>, headers: &HeaderMap) -> Option<RelayTrust> {
    let expected = configured?;
    let carrier = headers
        .get(RELAYED_SURFACE_HEADER)
        .map(|v| v.to_str().map(str::trim).unwrap_or(""));
    let presented = headers
        .get(SERVICE_CREDENTIAL_HEADER)
        .map(|v| v.to_str().unwrap_or(""));
    match (presented, carrier) {
        (None, None) => None,
        (None, Some(_)) => Some(RelayTrust::NoCredential),
        (Some(p), _) if !secret_matches(p, expected) => Some(RelayTrust::InvalidCredential),
        (Some(_), None) => Some(RelayTrust::CarrierMissing),
        (Some(_), Some(RELAY_ALLOWED_CARRIER)) => Some(RelayTrust::Trusted),
        (Some(_), Some(_)) => Some(RelayTrust::CarrierRefused),
    }
}

/// Validate the relay credential and, beside it, honor the attribution carrier.
///
/// Never rejects — the return is a plain `Response` because there is no rejection arm to
/// type. The `RelayedSurface` extension is inserted only when BOTH hold: the presented
/// credential matches the configured secret, AND the carrier value is exactly `mcp`.
/// Inbound `Authorization` is untouched — the caller's bearer rides it and is the auth
/// middleware's concern alone.
pub async fn require_relay_trust(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if let Some(trust) = assess(
        state.config.mcp_service_secret.as_deref(),
        request.headers(),
    ) {
        // The root span, as `require_auth` records `profile_id` onto it: a middleware body runs
        // inside the root span layer, so `current()` is that span here.
        tracing::Span::current().record("relay_trust", trust.as_str());
        if trust == RelayTrust::Trusted {
            request
                .extensions_mut()
                .insert(RelayedSurface(Surface::Mcp));
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_lowercase(k.as_bytes()).expect("lowercase name"),
                HeaderValue::from_str(v).expect("visible value"),
            );
        }
        h
    }

    #[test]
    fn valid_credential_and_mcp_carrier_is_trusted() {
        let h = headers(&[
            ("x-temper-service-credential", "s3cret"),
            ("x-temper-relayed-surface", "mcp"),
        ]);
        assert_eq!(assess(Some("s3cret"), &h), Some(RelayTrust::Trusted));
    }

    #[test]
    fn a_stolen_credential_cannot_generalize_to_cli() {
        // The residual the secret's confidentiality bounds: its holder can claim the one
        // value the allowlist admits. `cli` — and every other spelling — is refused. Case
        // matters (`parse_trusted` is case-sensitive; so is this), surrounding whitespace
        // is tolerated exactly as the surface header's convention tolerates it.
        for carrier in ["cli", "sdk", "web", "MCP", "mcp; drop table", ""] {
            let h = headers(&[
                ("x-temper-service-credential", "s3cret"),
                ("x-temper-relayed-surface", carrier),
            ]);
            assert_eq!(
                assess(Some("s3cret"), &h),
                Some(RelayTrust::CarrierRefused),
                "carrier {carrier:?} must be refused"
            );
        }
        let padded = headers(&[
            ("x-temper-service-credential", "s3cret"),
            ("x-temper-relayed-surface", "  mcp "),
        ]);
        assert_eq!(assess(Some("s3cret"), &padded), Some(RelayTrust::Trusted));
    }

    /// Each degrade face names its own cause, so the span value says which fault it is.
    #[test]
    fn each_degrade_face_is_its_own_value() {
        let cases = [
            (
                vec![("x-temper-relayed-surface", "mcp")],
                RelayTrust::NoCredential,
            ),
            (
                vec![
                    ("x-temper-service-credential", "wrong"),
                    ("x-temper-relayed-surface", "mcp"),
                ],
                RelayTrust::InvalidCredential,
            ),
            (
                vec![("x-temper-service-credential", "wrong")],
                RelayTrust::InvalidCredential,
            ),
            (
                vec![("x-temper-service-credential", "s3cret")],
                RelayTrust::CarrierMissing,
            ),
        ];
        for (pairs, expected) in cases {
            assert_eq!(
                assess(Some("s3cret"), &headers(&pairs)),
                Some(expected),
                "{pairs:?}"
            );
        }
    }

    /// Not relay-shaped, so no value at all: direct traffic with neither header, and every
    /// request while the API has no secret configured. Recording these would bury the signal.
    #[test]
    fn direct_traffic_and_an_unconfigured_api_record_nothing() {
        assert_eq!(assess(Some("s3cret"), &HeaderMap::new()), None);
        let relay_shaped = headers(&[
            ("x-temper-service-credential", "s3cret"),
            ("x-temper-relayed-surface", "mcp"),
        ]);
        assert_eq!(assess(None, &relay_shaped), None);
    }

    /// The span vocabulary is closed and distinct per face.
    #[test]
    fn the_recorded_values_are_distinct() {
        let all = [
            RelayTrust::Trusted,
            RelayTrust::NoCredential,
            RelayTrust::InvalidCredential,
            RelayTrust::CarrierMissing,
            RelayTrust::CarrierRefused,
        ];
        let values: std::collections::HashSet<_> = all.iter().map(|t| t.as_str()).collect();
        assert_eq!(values.len(), all.len());
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
