//! An instance's auth identity — parsed in `temper_auth::config` (published as `temperkb-auth`),
//! re-exported here so the API's configuration and every existing import keep one path.
//!
//! The parser moved so the MCP server can parse the same identity without temper-services'
//! `ApiConfig`: two surfaces verifying one instance's tokens must not hold two parsers.

pub use temper_auth::config::{parse_auth_config, AuthConfig, AuthConfigError, AuthMode};

/// A boot-blocking configuration fault.
///
/// Every message names the offending environment variable and states the relation it must satisfy.
/// **No message ever prints a value** — anyone who can act on the error can already read them, and a
/// config value in a log is a liability with no upside.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The auth identity or shared-secret hygiene refused — parsed in `temper_auth::config`, the
    /// one parser every verifying surface shares. Display is the inner message, unchanged.
    #[error(transparent)]
    Auth(#[from] AuthConfigError),

    #[error("{0} is not set.")]
    Missing(&'static str),

    #[error(
        "{0} and {1} hold the SAME value. Each of this instance's shared secrets gates a different \
         internal capability, and their being different values is the whole security property: \
         SLACK_LINK_SECRET answers \"is this principal linked?\", SLACK_MINT_SECRET vends a token \
         carrying that human's entire reach, and SLACK_VAULT_ENC_KEY decrypts every stored grant. \
         Two of them sharing a value means whoever holds the cheap capability already holds the \
         expensive one. Give each its own: `openssl rand -base64 32`."
    )]
    SecretCollision(&'static str, &'static str),

    // --- the rate-limit seam (spec A7/A9: chosen values, default off) ---
    //
    // Both arms name the variable and the shape it must take, and print no value — the
    // house rule every message above follows. They exist because a limit that fails to
    // parse must refuse to boot, not silently degrade to an unlimited door: an operator
    // who set a limit and made a typo would otherwise ship the exact posture the seam
    // exists to close, believing it bounded.
    #[error(
        "{0} must be {1}. The value is read once at boot; a value boot cannot read is a limit \
         that does not exist, so the instance refuses to start rather than run unlimited while \
         appearing bounded."
    )]
    NotAnInteger(&'static str, &'static str),

    #[error(
        "{0} must be {1}. A value outside that shape silently does nothing — a limit no window \
         ever counts is an absent limit wearing a configured one's clothes."
    )]
    RateLimitOutOfRange(&'static str, &'static str),
}
