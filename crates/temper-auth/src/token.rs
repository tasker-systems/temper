//! The token-endpoint response wire type.

/// RFC 6749 token response. Shared so both surfaces deserialize the same shape.
///
/// `id_token` is carried but unused: the access token is what we persist and decode.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}
