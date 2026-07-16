//! Shared OAuth2 Authorization Code + PKCE mechanics.
//!
//! Pure: crypto and string building, no HTTP and no I/O. Both surfaces need these —
//! temper-client for the CLI's loopback login, temper-services for the server-side
//! Slack account-link callback — and neither may depend on the other.
//!
//! What deliberately does NOT live here: the claims -> profile seam. `authenticate` /
//! `resolve_from_claims` are `pub(crate)` in temper-services *as a security property*
//! (a surface cannot hand them claims it built itself). Lifting them into a shared
//! crate would turn `pub(crate)` into `pub` across a crate boundary and the guarantee
//! would evaporate silently.

pub mod authorize;
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
