//! temper-client — Auth-aware HTTP client wrapping the temper cloud API.
//!
//! Shared by temper-cli, temper-mcp, and any future client. Handles JWT
//! lifecycle (login, refresh, logout), device identity, and typed methods
//! for every R5 API endpoint.

pub mod auth;
pub mod config;
pub mod error;
pub mod events;
pub mod http;
pub mod ingest;
pub mod login;
pub mod profile;
pub mod resources;
pub mod search;
pub mod sync;
pub mod upload;

use error::{ClientError, Result};

/// Top-level client for the temper cloud API.
///
/// Provides typed sub-clients via accessor methods (`resources()`, `search()`,
/// etc.) and handles authentication lifecycle (login, logout, status).
#[derive(Debug)]
pub struct TemperClient {
    http: http::HttpClient,
    oauth_config: Option<login::OAuthConfig>,
}

impl TemperClient {
    /// Create a new client targeting `base_url`.
    ///
    /// `device_id` is sent as `X-Temper-Client-Id` on every request for
    /// per-device manifest tracking.
    pub fn new(base_url: &str, device_id: Option<String>) -> Self {
        Self {
            http: http::HttpClient::new(base_url, device_id),
            oauth_config: None,
        }
    }

    /// Attach OAuth configuration for login and token refresh.
    pub fn with_oauth(mut self, config: login::OAuthConfig) -> Self {
        self.oauth_config = Some(config);
        self
    }

    /// Get a valid access token, refreshing via the token endpoint if needed.
    ///
    /// Requires OAuth config to have been set via [`with_oauth`](Self::with_oauth).
    pub async fn token(&self) -> Result<String> {
        let config = self
            .oauth_config
            .as_ref()
            .ok_or(ClientError::NotAuthenticated)?;
        auth::get_valid_token(&config.token_url, &config.client_id).await
    }

    // ----- Sub-client accessors -----

    /// Resource CRUD sub-client.
    pub fn resources(&self) -> resources::ResourceClient<'_> {
        resources::ResourceClient::new(&self.http)
    }

    /// Search sub-client.
    pub fn search(&self) -> search::SearchClient<'_> {
        search::SearchClient::new(&self.http)
    }

    /// Profile sub-client.
    pub fn profile(&self) -> profile::ProfileClient<'_> {
        profile::ProfileClient::new(&self.http)
    }

    /// Events sub-client.
    pub fn events(&self) -> events::EventClient<'_> {
        events::EventClient::new(&self.http)
    }

    /// Upload sub-client.
    pub fn upload(&self) -> upload::UploadClient<'_> {
        upload::UploadClient::new(&self.http)
    }

    /// Ingest sub-client.
    pub fn ingest(&self) -> ingest::IngestClient<'_> {
        ingest::IngestClient::new(&self.http)
    }

    /// Sync sub-client.
    pub fn sync(&self) -> sync::SyncClient<'_> {
        sync::SyncClient::new(&self.http)
    }

    // ----- Auth lifecycle -----

    /// Run the OAuth2 PKCE login flow (opens browser, waits for callback).
    pub async fn auth_login(&self) -> Result<auth::StoredAuth> {
        let config = self.oauth_config.as_ref().ok_or_else(|| {
            ClientError::Other("no OAuth config — call with_oauth() first".into())
        })?;
        login::login(config).await
    }

    /// Remove stored authentication credentials.
    pub fn auth_logout(&self) -> Result<()> {
        auth::clear_auth()
    }

    /// Return a summary of the current authentication state.
    pub fn auth_status(&self) -> Result<auth::AuthStatus> {
        auth::auth_status()
    }
}
