//! Typed sub-client for the operator-only `/api/connections` endpoints.

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::connection::{
    AttachCredentialResponse, Connection, ConnectionCredential, GrantConnectionReachRequest,
    ProvisionConnectionRequest, SetToolManifestRequest, SetWebhookEventsRequest,
};

/// Sub-client for connection provisioning.
pub struct ConnectionsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for ConnectionsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionsClient").finish_non_exhaustive()
    }
}

impl<'a> ConnectionsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Provision a connection. It is born `needs_credential`.
    pub async fn provision(&self, body: &ProvisionConnectionRequest) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/connections").json(body);
        self.http
            .send_json(&Method::POST, "/api/connections", req, Some(&token))
            .await
    }

    /// Enumerate connections.
    pub async fn list(&self, include_revoked: bool) -> Result<Vec<Connection>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections?include_revoked={include_revoked}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Load one connection.
    pub async fn get(&self, id: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// Revoke a connection. The profile, emitter entity, and home context survive — events
    /// already attributed to the emitter must keep resolving.
    pub async fn revoke(&self, id: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }

    /// Attach the credential — what flips `needs_credential` off.
    pub async fn attach_credential(
        &self,
        id: Uuid,
        body: &ConnectionCredential,
    ) -> Result<AttachCredentialResponse> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}/credential");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// Register the remote event types. Non-empty ⇒ ledger-capable.
    pub async fn set_webhook_events(&self, id: Uuid, events: Vec<String>) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}/webhook-events");
        let req = self
            .http
            .post(&path)
            .json(&SetWebhookEventsRequest { events });
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// Declare the read-only remote tools. Non-empty ⇒ reach-capable.
    pub async fn set_tool_manifest(&self, id: Uuid, tools: Vec<String>) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}/tool-manifest");
        let req = self
            .http
            .post(&path)
            .json(&SetToolManifestRequest { tools });
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// Grant a TEAM read-reach on this connection. Owning ≠ reaching — this writes an access grant
    /// so the team's members inherit read on what the connection receives.
    pub async fn grant_reach(&self, id: Uuid, team: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}/reach");
        let req = self
            .http
            .post(&path)
            .json(&GrantConnectionReachRequest { team });
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op.
    pub async fn revoke_reach(&self, id: Uuid, team: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/connections/{id}/reach");
        let req = self
            .http
            .delete(&path)
            .json(&GrantConnectionReachRequest { team });
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
}
