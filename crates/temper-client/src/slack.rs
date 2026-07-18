//! Slack account-link client surface.

use reqwest::Method;
use temper_core::types::slack::{SlackDisconnectRequest, SlackDisconnectResponse};

use crate::error::Result;
use crate::http::HttpClient;

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
        let path = "/api/auth/slack/link/me";
        let req = self.http.delete(path);
        self.http
            .send_json(&Method::DELETE, path, req, Some(&token))
            .await
    }

    /// Disconnect any principal. Requires system admin. Idempotent.
    pub async fn admin_disconnect(
        &self,
        slack_principal_id: &str,
    ) -> Result<SlackDisconnectResponse> {
        let token = self.http.resolve_token()?;
        let path = "/api/admin/slack/links/disconnect";
        let body = SlackDisconnectRequest {
            slack_principal_id: slack_principal_id.to_string(),
        };
        let req = self.http.post(path).json(&body);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }
}
