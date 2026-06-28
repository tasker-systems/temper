//! temper-client — Auth-aware HTTP client wrapping the temper cloud API.
//!
//! Shared by temper-cli, temper-mcp, and any future client. Handles JWT
//! lifecycle (login, refresh, logout), device identity, and typed methods
//! for every R5 API endpoint.

pub mod access;
pub mod auth;
pub mod cognitive_maps;
pub mod config;
pub mod contexts;
pub mod error;
pub mod events;
pub mod http;
pub mod ingest;
pub mod invocations;
pub mod login;
pub mod profile;
pub mod relationships;
pub mod resources;
pub mod search;
pub mod upload;

use std::sync::Arc;

use error::{ClientError, Result};

/// Top-level client for the temper cloud API.
///
/// Provides typed sub-clients via accessor methods (`resources()`, `search()`,
/// etc.) and handles authentication lifecycle (login, logout, status).
///
/// Holds an `Arc<dyn TokenStore>` so every auth operation (token refresh,
/// status, logout) routes through the store chosen at construction time.
/// Cloud sessions bind `MemoryTokenStore`; local sessions bind
/// `DiskTokenStore`. There is no "default disk path" fallback at this
/// layer — a missing store is a programming error.
pub struct TemperClient {
    http: http::HttpClient,
    oauth_config: Option<login::OAuthConfig>,
    store: Arc<dyn auth::TokenStore>,
}

impl std::fmt::Debug for TemperClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemperClient")
            .field("http", &self.http)
            .field("has_oauth_config", &self.oauth_config.is_some())
            .finish()
    }
}

impl TemperClient {
    /// Create a new client targeting `base_url`.
    ///
    /// `device_id` is sent as `X-Temper-Device-Id` on every request for
    /// per-device manifest tracking. `store` is the source of truth for
    /// token resolution.
    pub fn new(
        base_url: &str,
        device_id: Option<String>,
        store: Arc<dyn auth::TokenStore>,
    ) -> Self {
        Self {
            http: http::HttpClient::new(base_url, device_id, Some(store.clone())),
            oauth_config: None,
            store,
        }
    }

    /// Create a new client with a pre-resolved token override.
    ///
    /// Used by `build_client_from` after resolving the current token from
    /// the store — the override path keeps the request path off any further
    /// store reads for the lifetime of this client. The store is still held
    /// for refresh / logout / status operations.
    pub fn with_token(
        base_url: &str,
        device_id: Option<String>,
        token: String,
        store: Arc<dyn auth::TokenStore>,
    ) -> Self {
        Self {
            http: http::HttpClient::with_token_override(base_url, device_id, token),
            oauth_config: None,
            store,
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
        auth::get_valid_token(&*self.store, &config.token_url, &config.client_id).await
    }

    // ----- Sub-client accessors -----

    /// System access sub-client.
    pub fn access(&self) -> access::AccessClient<'_> {
        access::AccessClient::new(&self.http)
    }

    /// Resource CRUD sub-client.
    pub fn resources(&self) -> resources::ResourceClient<'_> {
        resources::ResourceClient::new(&self.http)
    }

    /// Relationship write sub-client (assert / retype / reweight / fold).
    pub fn relationships(&self) -> relationships::RelationshipClient<'_> {
        relationships::RelationshipClient::new(&self.http)
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

    /// Context CRUD sub-client.
    pub fn contexts(&self) -> contexts::ContextClient<'_> {
        contexts::ContextClient::new(&self.http)
    }

    /// Upload sub-client.
    pub fn upload(&self) -> upload::UploadClient<'_> {
        upload::UploadClient::new(&self.http)
    }

    /// Ingest sub-client.
    pub fn ingest(&self) -> ingest::IngestClient<'_> {
        ingest::IngestClient::new(&self.http)
    }

    /// Cognitive-map sub-client (reconcile).
    pub fn cognitive_maps(&self) -> cognitive_maps::CognitiveMapClient<'_> {
        cognitive_maps::CognitiveMapClient::new(&self.http)
    }

    /// Invocation-envelope sub-client (open / close / show / list).
    pub fn invocations(&self) -> invocations::InvocationsClient<'_> {
        invocations::InvocationsClient::new(&self.http)
    }

    // ----- Auth lifecycle -----

    /// Run the OAuth2 PKCE login flow (opens browser, waits for callback).
    pub async fn auth_login(&self) -> Result<auth::StoredAuth> {
        let config = self.oauth_config.as_ref().ok_or_else(|| {
            ClientError::Other("no OAuth config — call with_oauth() first".into())
        })?;
        login::login(config, &*self.store).await
    }

    /// Remove stored authentication credentials via the bound store.
    pub fn auth_logout(&self) -> Result<()> {
        self.store.clear()
    }

    /// Return a summary of the current authentication state from the bound store.
    pub fn auth_status(&self) -> Result<auth::AuthStatus> {
        auth::auth_status(&*self.store)
    }
}
