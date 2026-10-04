//! Shared OAuth2 Authorization Code + PKCE mechanics, and the instance auth-identity parser.
//!
//! Pure: crypto, string building and config parsing over an injected lookup — no HTTP and no
//! I/O. The PKCE half serves temper-client (the CLI's loopback login) and temper-services (the
//! server-side Slack account-link callback), neither of which may depend on the other. The
//! [`config`] half is the one parser of an instance's auth identity, read by both verifying
//! surfaces' boots: the API (through temper-services' `ApiConfig`) and the MCP server
//! (temper-mcp-server's `McpServerConfig`), so they cannot disagree about which tokens name the
//! instance.
//!
//! What deliberately does NOT live here: the claims -> profile seam. `authenticate` /
//! `resolve_from_claims` are `pub(crate)` in temper-services *as a security property*
//! (a surface cannot hand them claims it built itself). Lifting them into a shared
//! crate would turn `pub(crate)` into `pub` across a crate boundary and the guarantee
//! would evaporate silently.

pub mod authorize;
pub mod config;
pub mod pkce;
pub mod token;

pub use authorize::{build_authorize_url, AuthorizeParams};
pub use pkce::generate_pkce_pair;
pub use token::TokenResponse;

/// A fault in OAuth parameter construction.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    /// `authorize_url` came from configuration — a malformed value is a configuration
    /// fault, not a programming bug, so it propagates rather than panicking.
    #[error("authorize_url is not a valid URL ({0})")]
    InvalidAuthorizeUrl(String),
}
