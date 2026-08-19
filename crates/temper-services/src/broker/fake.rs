//! Non-Vercel broker impls: a configurable [`FakeBroker`] for tests and the
//! [`NullBroker`] for deployments with no broker configured.
//!
//! Following temper-client's precedent (`MemoryTokenStore` is shipped code, not
//! `#[cfg(test)]`), the fake is a real impl the resolver can select — which is
//! also the seam's swap-proof: a second `CredentialBroker` behind the same
//! `Arc<dyn _>`.

use super::{
    BrokerError, BrokerToken, CredentialBroker, InboundRequest, MintRequest, Minted, MintedReach,
    VerifiedInbound,
};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// What a [`FakeBroker`] does when asked to mint.
#[derive(Debug, Clone)]
pub enum FakeMint {
    /// Mint succeeds; the given value is the reported reach (`metadata`-shaped).
    Grants(serde_json::Value),
    /// The broker rejects the credential.
    Rejected,
    /// The connector exists but needs an OAuth consent first.
    NeedsConsent,
}

/// What a [`FakeBroker`] does when asked to verify an inbound webhook.
#[derive(Debug, Clone)]
pub enum FakeInbound {
    /// Verification succeeds, attributing the body to this connector identity.
    Accepts {
        provider: String,
        connector_uid: String,
        connector_id: String,
    },
    /// The attestation does not verify.
    Refuses,
}

/// A configurable broker for tests. Does no I/O.
///
/// **Counts mints.** Goal C3 says receipt produces no egress to the remote, and the broker's
/// `mint` is the only path by which intake could reach one. A count the transport test can
/// assert on turns "no egress" from a claim about the code's shape into a witness of what the
/// request actually did — which is what the clause asks for and reading the service cannot give.
#[derive(Debug, Clone)]
pub struct FakeBroker {
    mint: FakeMint,
    inbound: FakeInbound,
    mint_calls: Arc<AtomicUsize>,
}

impl FakeBroker {
    /// Verifies every inbound webhook, attributing it to the given connector. Mints are rejected:
    /// a receipt that mints is a C3 violation, and rejecting makes it fail rather than pass
    /// quietly.
    pub fn accepting_inbound(provider: &str, connector_uid: &str, connector_id: &str) -> Self {
        Self {
            mint: FakeMint::Rejected,
            inbound: FakeInbound::Accepts {
                provider: provider.to_string(),
                connector_uid: connector_uid.to_string(),
                connector_id: connector_id.to_string(),
            },
            mint_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Refuses every inbound attestation.
    pub fn refusing_inbound() -> Self {
        Self {
            mint: FakeMint::Rejected,
            inbound: FakeInbound::Refuses,
            mint_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many times `mint` has been called on this broker. The C3 witness.
    pub fn mint_calls(&self) -> usize {
        self.mint_calls.load(Ordering::SeqCst)
    }

    /// Mints successfully, reporting the given reach.
    pub fn granting(reach: serde_json::Value) -> Self {
        Self::with_mint(FakeMint::Grants(reach))
    }
    /// Rejects every mint (an `Unauthorized` credential).
    pub fn rejecting() -> Self {
        Self::with_mint(FakeMint::Rejected)
    }
    /// Reports the connector needs consent.
    pub fn needs_consent() -> Self {
        Self::with_mint(FakeMint::NeedsConsent)
    }

    /// The mint-configured constructors share one body so a new field cannot be added to two of
    /// the three.
    fn with_mint(mint: FakeMint) -> Self {
        Self {
            mint,
            inbound: FakeInbound::Refuses,
            mint_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl CredentialBroker for FakeBroker {
    async fn mint(&self, _req: MintRequest<'_>) -> Result<Minted, BrokerError> {
        self.mint_calls.fetch_add(1, Ordering::SeqCst);
        match &self.mint {
            FakeMint::Grants(reach) => Ok(Minted {
                token: BrokerToken::new("fake-token".into()),
                // A fixed, far-future expiry — deterministic (no clock read).
                expires_at: Utc.timestamp_opt(4_102_444_800, 0).single().unwrap(),
                reach: MintedReach { raw: reach.clone() },
            }),
            FakeMint::Rejected => Err(BrokerError::Unauthorized("fake rejection".into())),
            FakeMint::NeedsConsent => Err(BrokerError::NeedsConsent {
                authorize_url: None,
            }),
        }
    }

    async fn verify_inbound(
        &self,
        req: InboundRequest<'_>,
    ) -> Result<VerifiedInbound, BrokerError> {
        // The fake does not authenticate; it returns the configured verdict so a caller's
        // behaviour on both branches is testable without a signing key. The cryptographic
        // path is witnessed by `vercel_connect`'s own tests against a static JWKS key.
        match &self.inbound {
            FakeInbound::Accepts {
                provider,
                connector_uid,
                connector_id,
            } => Ok(VerifiedInbound {
                provider: provider.clone(),
                connector_uid: connector_uid.clone(),
                connector_id: connector_id.clone(),
                payload: req.body.to_vec(),
            }),
            FakeInbound::Refuses => Err(BrokerError::Verification("fake refusal".into())),
        }
    }
}

/// The broker for a deployment that has not configured one. Every operation
/// fails clearly rather than silently — a mint that cannot happen must say so.
#[derive(Debug, Clone, Default)]
pub struct NullBroker;

#[async_trait]
impl CredentialBroker for NullBroker {
    async fn mint(&self, _req: MintRequest<'_>) -> Result<Minted, BrokerError> {
        Err(BrokerError::NotConfigured)
    }
    async fn verify_inbound(
        &self,
        _req: InboundRequest<'_>,
    ) -> Result<VerifiedInbound, BrokerError> {
        Err(BrokerError::NotConfigured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::connection::ConnectionCredential;

    fn cred() -> ConnectionCredential {
        ConnectionCredential {
            broker: "fake".into(),
            connector: "fake/connector".into(),
            installation: None,
        }
    }
    fn mint_req(c: &ConnectionCredential) -> MintRequest<'_> {
        MintRequest {
            credential: c,
            subject: super::super::MintSubject::App,
            scopes: vec![],
        }
    }

    #[tokio::test]
    async fn granting_fake_reports_the_configured_reach() {
        let b = FakeBroker::granting(serde_json::json!({"repository_selection": "all"}));
        let c = cred();
        let minted = b.mint(mint_req(&c)).await.expect("mint ok");
        assert_eq!(
            minted
                .reach
                .raw
                .get("repository_selection")
                .and_then(|v| v.as_str()),
            Some("all")
        );
    }

    #[tokio::test]
    async fn rejecting_fake_returns_unauthorized() {
        let b = FakeBroker::rejecting();
        let c = cred();
        assert!(matches!(
            b.mint(mint_req(&c)).await.unwrap_err(),
            BrokerError::Unauthorized(_)
        ));
    }

    #[tokio::test]
    async fn null_broker_reports_not_configured() {
        let b = NullBroker;
        let c = cred();
        assert!(matches!(
            b.mint(mint_req(&c)).await.unwrap_err(),
            BrokerError::NotConfigured
        ));
    }
}
