//! The token-endpoint response wire type.

/// RFC 6749 token response. Shared so both surfaces deserialize the same shape.
///
/// `id_token` is carried but unused: the access token is what we persist and decode.
///
/// `Debug` is hand-written to REDACT all three token fields, matching the convention every other
/// credential-bearing type on this path already follows (`MintOutcome`, `NewGrant`, `VaultKey`,
/// `SlackLinkConfig`). This type is bound on the Slack mint path
/// (`slack_grant_vault_service::mint_access_token`) where it holds BOTH the rotated refresh token
/// — the durable grant — and an access token carrying a human's full reach. A derived `Debug` plus
/// one stray `?tokens` in a `tracing::` macro writes both into the platform log, where they are
/// retained and indexed and are not revoked by fixing the code afterwards.
///
/// Redaction is PRESENCE-PRESERVING for the two `Option` fields (`Some("redacted")` vs `None`):
/// "the IdP returned no refresh token" is a real, diagnosable condition — it is precisely the
/// misconfiguration the link callback's no-grant arm reports — and a flat `"redacted"` would erase
/// it. `expires_in` is not a credential and stays visible.
#[derive(Clone, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"redacted")
            .field("id_token", &self.id_token.as_ref().map(|_| "redacted"))
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "redacted"),
            )
            .field("expires_in", &self.expires_in)
            .finish()
    }
}
