//! The broker seam — temper's outbound reach to a remote system, behind one
//! swappable interface.
//!
//! A connection stores an abstract credential *reference* (`{broker, connector,
//! installation?}` — [`temper_core::types::connection::ConnectionCredential`]),
//! never a bare Vercel connector id. `broker` names the implementation, so a
//! platform swap costs one adapter. The seam is deliberately two operations:
//!
//! - [`CredentialBroker::mint`] — temper → remote: obtain a scoped token so an
//!   agent can read the remote's own MCP server.
//! - [`CredentialBroker::verify_inbound`] — remote → temper: authenticate a
//!   webhook the broker forwarded.
//!
//! Nothing above this seam knows Vercel Connect exists. The connector id lives on
//! the connection row, per instance — which is also what lets a self-hosted
//! operator provision their own connectors in their own broker account.
//!
//! Shape borrowed from [`crate::state::JwksKeyStore`] / temper-client's
//! `TokenStore`: a small `Send + Sync` trait, held as an `Arc<dyn _>`, selected
//! by a `resolve_*` function reading config.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use temper_core::types::connection::ConnectionCredential;

mod fake;
mod vercel_connect;

pub use fake::{FakeBroker, NullBroker};
pub use vercel_connect::{VercelConnectBroker, VercelConnectConfig};

/// Select the broker for a deployment: the Vercel Connect adapter when
/// configured, else the [`NullBroker`] (mints fail clearly). This is the seam's
/// one selection point — the shape borrowed from temper-cli's
/// `resolve_token_store`. Tests inject a [`FakeBroker`] directly.
pub fn resolve_broker(vercel: Option<VercelConnectConfig>) -> Arc<dyn CredentialBroker> {
    match vercel {
        Some(cfg) => Arc::new(VercelConnectBroker::new(cfg)),
        None => Arc::new(NullBroker),
    }
}

/// A minted remote token. Holds no `Debug`/`Serialize` of the value — a bearer
/// token must never reach a log line or a wire payload by accident. Same
/// discipline as temper-client's `MemoryTokenStore`.
#[derive(Clone)]
pub struct BrokerToken(String);

impl BrokerToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    /// Read the token value. The name is deliberately loud: every call site is a
    /// place a secret leaves the type.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for BrokerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrokerToken(<redacted>)")
    }
}

/// What a connector actually granted, as the provider reported it at mint time.
/// Provider-shaped (GitHub's `permissions` map differs from Linear's scopes), so
/// the raw value is preserved and read through accessors.
///
/// This is the input to the reach **drift check**: a connection *declares* its
/// reach (`reach_granularity`/`reach_covers`); the mint response says what the
/// credential can *actually* see. B4 surfaces the gap; B3 records the
/// acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedReach {
    /// The provider's `metadata`-shaped reach description, verbatim.
    pub raw: serde_json::Value,
}

/// A successful mint.
#[derive(Debug, Clone)]
pub struct Minted {
    pub token: BrokerToken,
    pub expires_at: DateTime<Utc>,
    pub reach: MintedReach,
}

/// Which subject a token is minted for. Only `App` is reachable from a static
/// broker credential (a per-user subject needs the caller's own delegated
/// identity, which the unattended paths do not have) — but the enum is modelled
/// so a future human-in-the-loop path is a variant, not a rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MintSubject {
    /// The connector's shared app identity. One token per connection.
    App,
}

/// A mint request. Borrows the credential (the connector id + installation the
/// broker holds the secret for).
#[derive(Debug, Clone)]
pub struct MintRequest<'a> {
    pub credential: &'a ConnectionCredential,
    pub subject: MintSubject,
    /// Forwarded to the provider. **Not enforced by the broker** — on GitHub's
    /// managed connector `scopes` is a silent no-op (the token's permissions come
    /// from the App). Never treat a mint's success as proof of narrowing.
    pub scopes: Vec<String>,
}

/// An inbound webhook the broker forwarded, awaiting verification.
#[derive(Debug, Clone)]
pub struct InboundRequest<'a> {
    /// The `Authorization` header value (the attestation bearer). **Not**
    /// `x-vercel-oidc-token` — that is the receiver's own ambient identity.
    pub authorization: Option<&'a str>,
    /// The raw request body — the provider's event, trusted only after verify.
    pub body: &'a [u8],
}

/// A verified inbound webhook. The broker authenticates the attestation and
/// extracts the **signed** connector identity; resolving it to a `kb_connections`
/// row is the caller's job (the broker stays DB-free, hence swappable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedInbound {
    /// Provider name from the attestation (e.g. `slack`, `github`).
    pub provider: String,
    /// The connector uid from the signed `trigger` claim (e.g. `slack/acme`).
    pub connector_uid: String,
    /// The connector id from the signed `trigger` claim (e.g. `scl_…`).
    pub connector_id: String,
    /// The event body, now trusted.
    pub payload: Vec<u8>,
}

/// Why a broker operation failed. Models the actionable cases distinctly — a
/// connector needing consent is not the same as a rejected credential, and a
/// caller must be able to tell them apart.
#[derive(Debug, thiserror::Error)]
pub enum BrokerError {
    /// The connector exists but a human must complete an OAuth consent first
    /// (`user_authorization_required` / `client_installation_required`).
    #[error("connector needs authorization{}", .authorize_url.as_ref().map(|u| format!(": {u}")).unwrap_or_default())]
    NeedsConsent { authorize_url: Option<String> },
    /// The credential was rejected by the broker (unknown/revoked connector, bad
    /// access token).
    #[error("broker rejected the credential: {0}")]
    Unauthorized(String),
    /// An inbound attestation failed verification (signature, claims, or the
    /// anti-decoy `client_id` check).
    #[error("inbound verification failed: {0}")]
    Verification(String),
    /// A network or broker-side transport failure (no signal about the credential
    /// itself).
    #[error("broker transport error: {0}")]
    Transport(String),
    /// A broker response could not be parsed into the expected shape.
    #[error("malformed broker response: {0}")]
    Malformed(String),
    /// No broker is configured on this deployment (the `NullBroker`).
    #[error("no credential broker is configured")]
    NotConfigured,
}

/// The seam. Two operations, both provider-agnostic above this line.
///
/// `Debug` is a supertrait so an `Arc<dyn CredentialBroker>` can live on the
/// `Debug`-deriving [`crate::state::AppState`]; every impl redacts its secret in
/// `Debug`.
#[async_trait]
pub trait CredentialBroker: Send + Sync + std::fmt::Debug {
    /// temper → remote: mint a scoped token for the connection's credential.
    async fn mint(&self, req: MintRequest<'_>) -> Result<Minted, BrokerError>;

    /// remote → temper: verify a forwarded webhook and extract its signed
    /// connector identity.
    async fn verify_inbound(&self, req: InboundRequest<'_>)
        -> Result<VerifiedInbound, BrokerError>;
}
