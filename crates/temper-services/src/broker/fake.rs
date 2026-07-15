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

/// A configurable broker for tests. Does no I/O.
#[derive(Debug, Clone)]
pub struct FakeBroker {
    mint: FakeMint,
}

impl FakeBroker {
    /// Mints successfully, reporting the given reach.
    pub fn granting(reach: serde_json::Value) -> Self {
        Self {
            mint: FakeMint::Grants(reach),
        }
    }
    /// Rejects every mint (an `Unauthorized` credential).
    pub fn rejecting() -> Self {
        Self {
            mint: FakeMint::Rejected,
        }
    }
    /// Reports the connector needs consent.
    pub fn needs_consent() -> Self {
        Self {
            mint: FakeMint::NeedsConsent,
        }
    }
}

#[async_trait]
impl CredentialBroker for FakeBroker {
    async fn mint(&self, _req: MintRequest<'_>) -> Result<Minted, BrokerError> {
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
        // The fake does not authenticate; it echoes a canned trigger so the trait
        // is fully implemented. Its verify path has no B4 caller (that is S3).
        Ok(VerifiedInbound {
            provider: "fake".into(),
            connector_uid: "fake/connector".into(),
            connector_id: "scl_fake".into(),
            payload: req.body.to_vec(),
        })
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
