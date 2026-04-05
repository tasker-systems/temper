use std::env;

/// Configuration specific to the MCP server deployment.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// Public base URL of this MCP server, e.g. `https://temperkb.io`.
    /// Used in WWW-Authenticate headers and oauth-protected-resource responses.
    pub mcp_base_url: String,

    /// Auth0 domain (issuer), e.g. `https://your-tenant.auth0.com/`.
    /// Reuses AUTH_ISSUER — no new env var needed.
    pub auth0_domain: String,

    /// OAuth audience / resource indicator for MCP tokens.
    /// Must match what Auth0 is configured to issue tokens for.
    pub mcp_audience: String,

    /// Pre-registered Auth0 application client_id for MCP clients.
    /// Returned by the registration endpoint so clients like Claude Desktop
    /// can complete OAuth without manual client_id entry.
    pub mcp_client_id: String,
}

impl McpConfig {
    pub fn from_env() -> Result<Self, env::VarError> {
        Ok(Self {
            mcp_base_url: env::var("MCP_BASE_URL")?,
            auth0_domain: env::var("AUTH_ISSUER")?,
            mcp_audience: env::var("MCP_AUDIENCE").or_else(|_| env::var("AUTH_AUDIENCE"))?,
            mcp_client_id: env::var("MCP_CLIENT_ID")?,
        })
    }
}
