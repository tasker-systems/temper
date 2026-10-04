//! The deployed door's identity seam — the one place the shell's outgoing identity is stated.
//!
//! Every relayed act from this deployment acts as the caller whose JWT the edge verified
//! ([`BearerToken`], planted by `require_mcp_auth`), at the [`Surface::Mcp`] surface, with two
//! opaque headers the tool layer never names: the service credential
//! (`TEMPER_MCP_SERVICE_SECRET`) and the `mcp` attribution carrier. Beside a valid credential
//! the API trusts the carrier and attributes the act to `<handle>@mcp`; without one it ignores
//! the carrier and the act attributes `@web`. `device_id` stays absent — the tool layer pins it
//! for every host, because a relay is not a device.
//!
//! **The trap this module exists to hold.** A mistake here changes *attribution* without
//! failing any wire-shape test. The e2e attribution witnesses (the per-family parity suites,
//! `mcp_span_link_test`, the ingest/blobs/artifacts attribution suite) are its gate, and
//! `tests/deployed_seam_test.rs` pins the exact identity this impl yields.

use temper_mcp::{IdentitySeam, OutgoingIdentity};
use temper_workflow::operations::{Surface, RELAYED_SURFACE_HEADER, SERVICE_CREDENTIAL_HEADER};

/// The raw, already-verified bearer token of the current request.
///
/// The JWT edge plants it in the request's extensions after the token verifies; the deployed
/// seam reads it back and the relay re-issues it, and the API re-verifies it on every act. A
/// newtype rather than a bare `String` so it cannot be confused with any other string in the
/// extensions map.
#[derive(Clone)]
pub struct BearerToken(pub String);

impl std::fmt::Debug for BearerToken {
    /// Hand-written, not derived — presence, never value. `http::request::Parts` derives
    /// `Debug` and formats its extension map, so a future `tracing::debug!(?parts)` on the
    /// relay path would have printed the live caller bearer verbatim. The
    /// `audit-credential-debug` tripwire cannot see tuple structs (its regex reads
    /// `name: Type` field position only), which is exactly why this impl exists rather than a
    /// baseline entry.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BearerToken")
            .field("0", &"<redacted>")
            .finish()
    }
}

/// The deployed door's seam: the edge-verified bearer, `Surface::Mcp`, and the service
/// credential plus `mcp` carrier as opaque extra headers.
#[derive(Clone)]
pub struct DeployedDoorSeam {
    headers: http::HeaderMap,
}

impl DeployedDoorSeam {
    /// The seam over this deployment's service credential. Refuses a value that cannot ride
    /// a header; the caller words the refusal (it names the variable the value came from).
    pub fn new(service_secret: &str) -> Result<Self, http::header::InvalidHeaderValue> {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::HeaderName::try_from(SERVICE_CREDENTIAL_HEADER)
                .expect("the service credential header name is a valid header name"),
            http::HeaderValue::from_str(service_secret)?,
        );
        headers.insert(
            http::HeaderName::try_from(RELAYED_SURFACE_HEADER)
                .expect("the relayed surface header name is a valid header name"),
            http::HeaderValue::from_static("mcp"),
        );
        Ok(Self { headers })
    }
}

impl IdentitySeam for DeployedDoorSeam {
    fn outgoing_identity(&self, parts: &http::request::Parts) -> Option<OutgoingIdentity> {
        let bearer = parts.extensions.get::<BearerToken>()?;
        Some(
            OutgoingIdentity::new(bearer.0.clone(), Surface::Mcp)
                .with_extra_headers(self.headers.clone()),
        )
    }
}

impl std::fmt::Debug for DeployedDoorSeam {
    /// The service credential is a live secret: header names, never values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeployedDoorSeam")
            .field(
                "headers",
                &self.headers.keys().map(|k| k.as_str()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Debug impl is the whole contract: the bearer must never render, in full or in
    /// prefix. FAILS IF: the formatted output contains any substring of the token.
    #[test]
    fn bearer_token_debug_never_renders_the_token() {
        let token = "aaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbb.cccccccccccccccc";
        let rendered = format!("{:?}", BearerToken(token.to_string()));
        assert!(
            !rendered.contains("bbbbbbbbbbbbbbbb"),
            "Debug leaks the token: {rendered}"
        );
        assert!(
            rendered.contains("redacted"),
            "presence-preserving shape: {rendered}"
        );
    }

    /// FAILS IF: the seam's Debug renders the service credential.
    #[test]
    fn the_deployed_seam_debug_never_renders_the_credential() {
        let seam = DeployedDoorSeam::new("the-service-credential-value").unwrap();
        let rendered = format!("{seam:?}");
        assert!(
            !rendered.contains("the-service-credential-value"),
            "{rendered}"
        );
    }
}
