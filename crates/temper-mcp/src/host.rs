//! What a host hands the tool layer per process — as plain values this crate owns, never a
//! services type.
//!
//! Besides the [`RelayConfig`](crate::seam::RelayConfig) and its seam
//! ([`crate::seam`]), a host supplies the [`BlobDoor`]: whether it serves the blob tools, the
//! single-request ceiling when it does, and the sentence it refuses with when it does not.
//! Nothing here can reach a pool, a key store or the API's configuration.

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
