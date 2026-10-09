//! Slack account-link client surface.

use temper_core::types::slack::{SlackDisconnectRequest, SlackDisconnectResponse};

use crate::error::Result;
use crate::http::HttpClient;
use crate::ops;

pub struct SlackClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SlackClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackClient").finish_non_exhaustive()
    }
}

impl<'a> SlackClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Disconnect the caller's own Slack link. Idempotent.
    pub async fn disconnect_me(&self) -> Result<SlackDisconnectResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::DISCONNECT_ME;
        let path = op.path(&[])?;
        let req = self.http.request(op, &path);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }

    /// Disconnect any principal. Requires system admin. Idempotent.
    pub async fn admin_disconnect(
        &self,
        slack_principal_id: &str,
    ) -> Result<SlackDisconnectResponse> {
        let token = self.http.resolve_token()?;
        let op = &ops::ADMIN_DISCONNECT;
        let path = op.path(&[])?;
        let body = SlackDisconnectRequest {
            slack_principal_id: slack_principal_id.to_string(),
        };
        let req = self.http.request(op, &path).json(&body);
        self.http
            .send_json(&op.method(), &path, req, Some(&token))
            .await
    }
}
