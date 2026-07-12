use serde::Deserialize;
use std::env;

/// Static MCP server configuration embedded at compile time from `mcp-server.toml`.
static MCP_SERVER_TOML: &str = include_str!("../mcp-server.toml");

/// Top-level shape of `mcp-server.toml`.
#[derive(Debug, Clone, Deserialize)]
struct McpServerFile {
    oauth: OAuthStaticConfig,
}

/// OAuth-related static configuration (allowed redirect URIs, etc.).
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthStaticConfig {
    /// Allowed redirect URIs echoed back in DCR responses.
    pub redirect_uris: Vec<String>,
    /// Accept any `http://localhost` or `http://127.0.0.1` redirect URI.
    #[serde(default)]
    pub allow_localhost: bool,
}

/// Configuration specific to the MCP server deployment.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Public base URL of this MCP server, e.g. `https://temperkb.io`.
    /// Used in WWW-Authenticate headers and oauth-protected-resource responses.
    pub mcp_base_url: String,

    // NOTE: there is deliberately no `mcp_audience` here any more. An instance has exactly ONE
    // audience, parsed once into `temper_services::auth_config::AuthConfig` and read by both
    // surfaces. `MCP_AUDIENCE` the env var still exists, but it is now only an assertion that it
    // restates `AUTH_AUDIENCE` — enforced at boot, in one place. Two parsers for one concept is
    // what let an empty value disable validation on temper-api while rejecting every token on
    // temper-mcp.
    /// Pre-registered Auth0 application client_id for MCP clients.
    /// Returned by the registration endpoint so clients like Claude Desktop
    /// can complete OAuth without manual client_id entry.
    /// `None` if `MCP_CLIENT_ID` is not set — DCR will return 503.
    pub mcp_client_id: Option<String>,

    /// Static OAuth config loaded from the embedded `mcp-server.toml`.
    pub oauth: OAuthStaticConfig,
}

impl McpConfig {
    pub fn from_env() -> Result<Self, McpConfigError> {
        let server_file: McpServerFile =
            toml::from_str(MCP_SERVER_TOML).map_err(McpConfigError::Toml)?;

        Ok(Self {
            mcp_base_url: env::var("MCP_BASE_URL").map_err(McpConfigError::Env)?,
            mcp_client_id: env::var("MCP_CLIENT_ID").ok(),
            oauth: server_file.oauth,
        })
    }
}

/// Errors that can occur when loading MCP configuration.
#[derive(Debug)]
pub enum McpConfigError {
    Env(env::VarError),
    Toml(toml::de::Error),
}

impl std::fmt::Display for McpConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env(e) => write!(f, "missing environment variable: {e}"),
            Self::Toml(e) => write!(f, "invalid mcp-server.toml: {e}"),
        }
    }
}

impl std::error::Error for McpConfigError {}
