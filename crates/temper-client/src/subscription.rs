//! Typed sub-client for the operator-only `/api/subscriptions` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
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
        let req = self.http.post("/api/subscriptions").json(body);
        self.http
            .send_json(&Method::POST, "/api/subscriptions", req, Some(&token))
            .await
    }

    /// Enumerate subscriptions visible to the caller. Optional `connection_id` filter.
    pub async fn list(
        &self,
        include_revoked: bool,
        connection_id: Option<Uuid>,
    ) -> Result<Vec<Subscription>> {
        let token = self.http.resolve_token()?;
        let mut path = format!("/api/subscriptions?include_revoked={include_revoked}");
        if let Some(cid) = connection_id {
            path.push_str(&format!("&connection_id={cid}"));
        }
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Load one subscription.
    pub async fn get(&self, id: Uuid) -> Result<Subscription> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/subscriptions/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Revoke a subscription. Rows are never deleted — a revoked subscription stops matching
    /// but stays resolvable for the delivery row's research-corpus property.
    pub async fn revoke(&self, id: Uuid) -> Result<Subscription> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/subscriptions/{id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
}
