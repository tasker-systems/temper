//! Typed sub-client for the operator-only `/api/machine-clients` endpoints.

use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;
use temper_core::types::machine::{
    IssueMachineRequest, IssuedMachineCredential, MachineClient, ProvisionMachineRequest,
    RebindMachineRequest, RotateSecretRequest,
};

/// Sub-client for machine-principal registration.
pub struct MachineClientsClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for MachineClientsClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineClientsClient")
            .finish_non_exhaustive()
    }
}

impl<'a> MachineClientsClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Register a new machine principal, creating its agent profile.
    pub async fn provision(&self, body: &ProvisionMachineRequest) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let op = &ops::PROVISION_MACHINE_CLIENT;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Point a fresh client id at an existing agent profile.
    pub async fn rebind(&self, id: Uuid, body: &RebindMachineRequest) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_REBIND_MACHINE_CLIENT;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Enumerate registered clients.
    pub async fn list(&self, include_revoked: bool) -> Result<Vec<MachineClient>> {
        let token = self.http.resolve_token()?;
        let op = &ops::LIST_MACHINE_CLIENTS;
        let path = format!("{}?include_revoked={include_revoked}", op.path(&[])?);
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Load one registered client.
    pub async fn get(&self, id: Uuid) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let op = &ops::GET_MACHINE_CLIENT;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Revoke a client. Denies authentication; grants and memberships survive (D11).
    pub async fn revoke(&self, id: Uuid) -> Result<MachineClient> {
        let token = self.http.resolve_token()?;
        let op = &ops::REVOKE_MACHINE_CLIENT;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Issue a temper-minted machine credential. Returns the one-time plaintext secret.
    pub async fn issue(&self, body: &IssueMachineRequest) -> Result<IssuedMachineCredential> {
        let token = self.http.resolve_token()?;
        let op = &ops::ISSUE_MACHINE_CREDENTIAL;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Rotate a temper-issued secret, leaving the previous valid for a grace window.
    pub async fn rotate_secret(
        &self,
        id: Uuid,
        body: &RotateSecretRequest,
    ) -> Result<IssuedMachineCredential> {
        let token = self.http.resolve_token()?;
        let op = &ops::ROTATE_MACHINE_CLIENT_SECRET;
        let path = op.path(&[&id])?;
        let req = self.http.request(op, &path).json(body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
