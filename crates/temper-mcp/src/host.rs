//! What a host hands the tool layer — as plain values this crate owns, never a services type.
//!
//! The tool layer holds no database handle and no server configuration. A host (today the
//! deployed `temper-mcp-server`) verifies the caller at its own edge, then supplies:
//!
//! - per request, the caller's verified bearer, as a [`BearerToken`] in the request's extensions
//!   (the `&parts` every tool takes), which the relay re-issues across the network door;
//! - per process, the [`BlobDoor`] — whether this deployment serves the blob tools, the
//!   single-request ceiling when it does, and the sentence it refuses with when it does not.
//!
//! Both are values: nothing here can reach a pool, a key store or the API's configuration.

/// The raw, already-verified bearer token of the current request.
///
/// A newtype rather than a bare `String` so it cannot be confused with any other
/// string in the extensions map. The host's JWT edge plants it after the token verifies;
/// the relay forwards it, and the API re-verifies it on every act.
#[derive(Clone)]
pub struct BearerToken(pub String);

impl std::fmt::Debug for BearerToken {
    /// Hand-written, not derived — presence, never value (the McpConfig redaction's
    /// shape, 2026-09-24 review). `http::request::Parts` derives `Debug` and formats
    /// its extension map, so a future `tracing::debug!(?parts)` on the relay path
    /// would have printed the live caller bearer verbatim. The
    /// `audit-credential-debug` tripwire cannot see tuple structs (its regex reads
    /// `name: Type` field position only), which is exactly why this impl exists
    /// rather than a baseline entry.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BearerToken")
            .field("0", &"<redacted>")
            .finish()
    }
}

/// The deployment's blob posture, read once at the host's boot.
///
/// One knob drives both faces of the blob door: `list_tools` advertises the blob pair only
/// when it is [`BlobDoor::Open`], and a direct call against the hidden pair answers the
/// [`BlobDoor::Closed`] refusal — the sentence the host chose through the shared selector
/// (`temper_services::services::blob_service::blob_refusal`), so this door speaks the API's
/// vocabulary without naming its error type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlobDoor {
    /// Blob credentials resolve: the pair is served, and reads are bounded by the deployment's
    /// `BLOB_SINGLE_REQUEST_MAX_BYTES`.
    Open { single_request_max_bytes: usize },
    /// No blob store, or `BLOB_ENABLED` closed it: the pair refuses with `refusal`.
    Closed { refusal: String },
}

impl BlobDoor {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Debug impl is the whole contract: the bearer must never render, in full or
    /// in prefix. FAILS IF: the formatted output contains any substring of the token.
    #[test]
    fn bearer_token_debug_never_renders_the_token() {
        let token = "aaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbb.cccccccccccccccc";
        let rendered = format!("{:?}", BearerToken(token.to_string()));
        assert!(
            !rendered.contains("bbbbbbbbbbbbbbbb"),
            "Debug leaks the token payload: {rendered}"
        );
        assert!(
            rendered.contains("redacted"),
            "presence-preserving shape: {rendered}"
        );
    }
}
