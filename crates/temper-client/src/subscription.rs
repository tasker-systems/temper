//! Typed sub-client for the operator-only `/api/subscriptions` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::subscription::{CreateSubscriptionRequest, Subscription};

/// Sub-client for subscription management.
pub struct SubscriptionsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SubscriptionsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionsClient")
            .finish_non_exhaustive()
    }
}

impl<'a> SubscriptionsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Create a subscription. The two-leg authz gate (authoring-team manage-capable + reach
    /// grant held) runs server-side before the INSERT.
    pub async fn create(&self, body: &CreateSubscriptionRequest) -> Result<Subscription> {
        let token = self.http.resolve_token()?;
        let op = &ops::CREATE_SUBSCRIPTION;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Enumerate subscriptions visible to the caller. Optional `connection_id` filter.
    pub async fn list(
        &self,
        include_revoked: bool,
        connection_id: Option<Uuid>,
    ) -> Result<Vec<Subscription>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_SUBSCRIPTIONS;
        let mut path = format!("{}?include_revoked={include_revoked}", op.path(&[])?);
        if let Some(cid) = connection_id {
            path.push_str(&format!("&connection_id={cid}"));
        }
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Load one subscription.
    pub async fn get(&self, id: Uuid) -> Result<Subscription> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_SUBSCRIPTION;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Revoke a subscription. Rows are never deleted — a revoked subscription stops matching
    /// but stays resolvable for the delivery row's research-corpus property.
    pub async fn revoke(&self, id: Uuid) -> Result<Subscription> {
        let token = self.http.resolve_token()?;
        let op = &ops::REVOKE_SUBSCRIPTION;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
