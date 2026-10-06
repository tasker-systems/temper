//! Typed sub-client for the operator-only `/api/connections` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
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
        let op = &ops::PROVISION_CONNECTION;
        let path = op.path(&[]);
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Enumerate connections.
    pub async fn list(&self, include_revoked: bool) -> Result<Vec<Connection>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_CONNECTIONS;
        let path = format!("{}?include_revoked={include_revoked}", op.path(&[]));
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Load one connection.
    pub async fn get(&self, id: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_CONNECTION;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Revoke a connection. The profile, emitter entity, and home context survive — events
    /// already attributed to the emitter must keep resolving.
    pub async fn revoke(&self, id: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let op = &ops::REVOKE_CONNECTION;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Attach the credential — what flips `needs_credential` off.
    pub async fn attach_credential(
        &self,
        id: Uuid,
        body: &ConnectionCredential,
    ) -> Result<AttachCredentialResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::ATTACH_CONNECTION_CREDENTIAL;
        let path = op.path(&[&id]);
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Register the remote event types. Non-empty ⇒ ledger-capable.
    pub async fn set_webhook_events(&self, id: Uuid, events: Vec<String>) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let op = &ops::SET_CONNECTION_WEBHOOK_EVENTS;
        let path = op.path(&[&id]);
        let req = self
            .http
            .request(op, &path)
            .json(&SetWebhookEventsRequest { events });
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Declare the read-only remote tools. Non-empty ⇒ reach-capable.
    pub async fn set_tool_manifest(&self, id: Uuid, tools: Vec<String>) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let op = &ops::SET_CONNECTION_TOOL_MANIFEST;
        let path = op.path(&[&id]);
        let req = self
            .http
            .request(op, &path)
            .json(&SetToolManifestRequest { tools });
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Grant a TEAM read-reach on this connection. Owning ≠ reaching — this writes an access grant
    /// so the team's members inherit read on what the connection receives.
    ///
    /// `affirm_reach` carries the intentional affirmation required when the connection declares a
    /// reach: without it the grant FAILS server-side. It records the intent for review; it does not
    /// narrow the remote reach.
    pub async fn grant_reach(
        &self,
        id: Uuid,
        team: Uuid,
        affirm_reach: Option<String>,
    ) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let op = &ops::GRANT_CONNECTION_REACH;
        let path = op.path(&[&id]);
        let req = self
            .http
            .request(op, &path)
            .json(&GrantConnectionReachRequest { team, affirm_reach });
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Revoke a team's read-reach on this connection. Idempotent — an absent grant is a no-op.
    pub async fn revoke_reach(&self, id: Uuid, team: Uuid) -> Result<Connection> {
        let token = self.http.resolve_token()?;
        let op = &ops::REVOKE_CONNECTION_REACH;
        let path = op.path(&[&id]);
        let req = self
            .http
            .request(op, &path)
            .json(&GrantConnectionReachRequest {
                team,
                affirm_reach: None,
            });
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
