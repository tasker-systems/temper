//! Vercel serverless function entry point for the temper MCP server.
//!
//! Bridges the axum Router from temper-mcp-server to Vercel's serverless interface
//! via VercelLayer — identical pattern to api/axum.rs.

use tower::ServiceBuilder;
use vercel_runtime::axum::VercelLayer;

use temper_mcp::McpConfig;
use temper_mcp_server::McpServerConfig;
use temper_services::state::JwksKeyStore;

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    // Name this executable in exported spans before the exporter is built — see api/axum.rs. The MCP
    // and API executables share a Vercel project, so distinct code-set names are what keep their
    // spans apart in the Tier-2 waterfall (work item 2b).
    temper_telemetry::set_service_name("temper-mcp");
    temper_telemetry::init_server_logging();

    // `unwrap_or_else(panic!)` rather than `.expect()`: expect prints Debug, and these errors carry
    // their remedy in Display. An instance that cannot state which audience it validates must not
    // serve traffic. This governs BOTH loads below — `McpConfig` used `.expect()` until
    // `[found — 2026-08-21]`, so a misconfigured MCP deployment aborted with a Debug dump instead
    // of the remedy, on the surface whose misconfiguration is hardest to notice.
    //
    // `McpServerConfig`, not the API's `ApiConfig`: this function holds no database pool and opens
    // no connection — every tool relays to the API, which owns the database. `DATABASE_URL` is
    // never read here (on Vercel it is still in this process's environment, because environment
    // variables are project-scoped; the API's function is the one that uses it).
    let server_config =
        McpServerConfig::from_env().unwrap_or_else(|e| panic!("refusing to start: {e}"));
    let mcp_config = McpConfig::from_env().unwrap_or_else(|e| panic!("refusing to start: {e}"));

    let jwks_store = JwksKeyStore::new(server_config.auth.jwks_url.clone());
    let app = temper_mcp_server::build_router(server_config, jwks_store, mcp_config);

    let service = ServiceBuilder::new().layer(VercelLayer::new()).service(app);

    tracing::info!("temper-mcp: Vercel function initialized");

    vercel_runtime::run(service).await
}
